//! A row in progress on its pull request's checks (docs/status-model.md, A
//! quiet row that waits; status-marks D-07, D-08, D-09).
//!
//! Each of this Mac's rows gets two inputs before the lineage decides what it
//! waits for: the pull request whose running checks it waits on (or whose
//! GitHub read, set off by a turn that just printed its address, has not
//! landed), and how the checks it waited on in its current state ended. The
//! second needs memory of the first, kept here per pane and dropped when the
//! pane's state moves or the pane goes. Everything is a pass over rows in
//! hand, under the lock, with no I/O; GitHub is read by the existing reader.

use super::*;
use crate::model::{ChecksEnded, PullRequestChecks};
use crate::request_view::RowPlace;

/// What one pane waited on in the state it was seen in.
#[derive(Default)]
struct Awaited {
    state: Option<u64>,
    numbers: BTreeSet<u32>,
    ended: Option<ChecksEnded>,
}

/// The checks each pane waited on, by pane, and the exclusions already logged.
/// A full pass keeps both to the panes it listed; a pass that finds no pull
/// request waiting leaves the exclusions as they were, so the next full pass
/// bounds them. `projects` are the projects a row waits on running checks in,
/// as of the last pass.
#[derive(Default)]
pub(super) struct ChecksMemory {
    awaited: HashMap<String, Awaited>,
    excluded: HashSet<(String, u32, &'static str)>,
    projects: BTreeSet<String>,
}

impl Runtime {
    /// Lays `checks_wait` and `checks_ended` on every row in `agents`. A
    /// device's rows link no pull request (PRD Non-goals), so they get none.
    /// `agents` is every row the projection holds, so a pane it no longer
    /// lists leaves the memory.
    pub(super) fn lay_checks(&mut self, agents: &mut [SidebarAgentSnapshot]) {
        // Most passes have nothing to wait on: no check runs, no read is owed
        // and nothing ended unseen. They take no copy of the rows.
        let pending = self.github.projects.iter().any(|project| {
            project
                .pull_requests
                .iter()
                .any(|pull_request| pull_request.checks == PullRequestChecks::Pending)
        });
        let owed = agents.iter().any(|row| {
            row.row_facts.as_ref().is_some_and(|facts| {
                facts
                    .sighted_prs
                    .iter()
                    .any(|address| self.sighted_read_owed(address))
            })
        });
        if !pending && !owed && self.checks_memory.awaited.is_empty() {
            for row in agents.iter_mut() {
                row.checks_wait = None;
                row.checks_ended = None;
            }
            // The log-once memory of exclusions stays: a pull request whose
            // checks run again is not logged again (the full pass bounds it
            // to listed panes).
            self.checks_memory.projects.clear();
            return;
        }
        let places: HashMap<String, (Option<String>, Option<String>, String)> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .flat_map(|workspace| {
                workspace.checkouts.iter().flat_map(move |checkout| {
                    checkout.tabs.iter().flat_map(move |tab| {
                        tab.panes.iter().map(move |pane| {
                            (
                                pane.id.clone(),
                                (
                                    checkout.branch.clone(),
                                    checkout.head_sha().map(str::to_owned),
                                    workspace.path.clone(),
                                ),
                            )
                        })
                    })
                })
            })
            .collect();
        // A row fresh from the engine has no lineage yet; the duty goes to the row
        // nearest its root, so the depth the last projection knew stands in.
        let depths: HashMap<&str, usize> = self
            .snapshot
            .navigator
            .agents
            .iter()
            .map(|row| (row.pane_id.as_str(), row.lineage_depth))
            .collect();
        let mut ranked = agents.to_vec();
        for row in &mut ranked {
            if let Some(depth) = depths.get(row.pane_id.as_str()) {
                row.lineage_depth = row.lineage_depth.max(*depth);
            }
        }
        let github = &self.github;
        let mut linked: Vec<_> = ranked
            .iter()
            .map(|row| {
                let place = places
                    .get(&row.pane_id)
                    .map(|(branch, head_sha, root_path)| RowPlace {
                        branch: branch.as_deref(),
                        head_sha: head_sha.as_deref(),
                        root_path,
                    });
                crate::agent_state::work::linked_pull_requests(row, place, github)
            })
            .collect();
        crate::agent_state::work::assign_duty(&ranked, &mut linked);
        let mut excluded = Vec::new();
        let mut projects = BTreeSet::new();
        let listed: HashSet<&str> = agents.iter().map(|row| row.pane_id.as_str()).collect();
        self.checks_memory
            .awaited
            .retain(|pane, _| listed.contains(pane.as_str()));
        self.checks_memory
            .excluded
            .retain(|(pane, ..)| listed.contains(pane.as_str()));
        for (row, links) in agents.iter_mut().zip(&linked) {
            if !places.contains_key(&row.pane_id) {
                row.checks_wait = None;
                row.checks_ended = None;
                continue;
            }
            let duty: Vec<&crate::model::PullRequestSnapshot> = links
                .iter()
                .filter(|link| link.duty())
                .map(|link| link.pull_request())
                .collect();
            let mut running = None;
            for pull_request in &duty {
                if pull_request.badge.is_settled()
                    || pull_request.checks != PullRequestChecks::Pending
                {
                    continue;
                }
                // A read that failed or a `gh` that cannot read shows the
                // last answer, which may be long over; a required check that
                // never reported may never run. Neither holds a row (B12).
                let project = github.projects.iter().find(|project| {
                    project
                        .pull_requests
                        .iter()
                        .any(|known| known.url == pull_request.url)
                });
                let current = project.is_some_and(|project| {
                    project.status.available
                        && !project.status.stale
                        && project.status.unavailable_reason.is_none()
                });
                if !current {
                    excluded.push((row.pane_id.clone(), pull_request.number, "stale"));
                } else if pull_request.work.running_checks == 0 {
                    excluded.push((row.pane_id.clone(), pull_request.number, "expected_only"));
                } else {
                    running = running.or(Some(pull_request.number));
                    projects.extend(project.map(|project| project.root_path.clone()));
                }
            }
            let owed = row.row_facts.as_ref().and_then(|facts| {
                facts
                    .sighted_prs
                    .iter()
                    .find(|address| self.sighted_read_owed(address))
                    .and_then(|(_, number)| u32::try_from(*number).ok())
            });
            row.checks_wait = running.or(owed);
            let awaited = self
                .checks_memory
                .awaited
                .entry(row.pane_id.clone())
                .or_default();
            if awaited.state != row.state_change_seq {
                // The agent moved on: what it waited on before is not news.
                *awaited = Awaited {
                    state: row.state_change_seq,
                    ..Awaited::default()
                };
            }
            awaited.numbers.extend(running);
            let numbers = std::mem::take(&mut awaited.numbers);
            for number in numbers {
                match duty
                    .iter()
                    .find(|pull_request| pull_request.number == number)
                    .map(|pull_request| pull_request.checks)
                {
                    Some(PullRequestChecks::Passing) => {
                        awaited.ended = Some(ChecksEnded {
                            number,
                            passed: true,
                        });
                    }
                    Some(PullRequestChecks::Failed) => {
                        awaited.ended = Some(ChecksEnded {
                            number,
                            passed: false,
                        });
                    }
                    Some(PullRequestChecks::Pending) => {
                        awaited.numbers.insert(number);
                    }
                    // No longer this row's, or its checks went unknown: it
                    // is not a result Hide can name.
                    _ => {}
                }
            }
            row.checks_ended = awaited.ended;
        }
        self.checks_memory
            .awaited
            .retain(|_, awaited| !awaited.numbers.is_empty() || awaited.ended.is_some());
        self.checks_memory.projects = projects;
        for (pane, number, reason) in excluded {
            if self
                .checks_memory
                .excluded
                .insert((pane.clone(), number, reason))
            {
                crate::diagnostic!(serde_json::json!({
                    "component": "checks_wait",
                    "kind": "checks_wait.excluded",
                    "pane_id": pane,
                    "pull_request": number,
                    "reason": reason,
                }));
            }
        }
    }

    /// Lays the checks again on the rows already shown, after GitHub's
    /// answer moved a pull request or a held read ended, and rebuilds what
    /// reads them: the lineage decides the wait, the read axis the news.
    pub(super) fn refresh_checks_waits(&mut self) -> bool {
        let mut agents = self.snapshot.navigator.agents.clone();
        self.lay_checks(&mut agents);
        let moved = agents
            .iter()
            .zip(&self.snapshot.navigator.agents)
            .any(|(now, before)| {
                now.checks_wait != before.checks_wait || now.checks_ended != before.checks_ended
            });
        if !moved {
            return false;
        }
        self.snapshot.navigator.agents = agents;
        self.refresh_agent_lineage() | self.refresh_pane_read_state()
    }

    /// The projects with a row waiting on running checks; each is read
    /// again once a minute while it has one, whatever the window shows (B13).
    /// There are at most as many as such rows.
    pub(super) fn checks_wait_projects(&self) -> &BTreeSet<String> {
        &self.checks_memory.projects
    }
}
