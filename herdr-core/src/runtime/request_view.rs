//! Keeps every agent row's request block current (`crate::request_view`).
//!
//! A session's rows are rebuilt fresh on every update, so the block is laid
//! on them in the same pipeline, before they are compared with the rows
//! already shown; an unchanged update stays unchanged. It is laid again
//! where its other inputs move: after GitHub's pull requests land, after
//! the read axis moves, and after the cross-device lineage pass. Each is a
//! pass over rows in hand, under the lock, with no I/O; the verb records are
//! saved through the coalesced UI-state save.

use super::*;
use crate::request_view::{self, RowPlace};

/// How often, while the request view is shown or a row waits on its pull
/// request's checks, a project whose pull request has checks running is read
/// again (D-32, status-marks B13).
const PENDING_CHECKS_REREAD: std::time::Duration = std::time::Duration::from_secs(60);

impl Runtime {
    /// The PRs list turns a working agent stalled and a pull request old with
    /// the clock alone (PRD work-tab-pr-lens D-10, D-11), so the projection
    /// is judged again once a minute while any pull request is listed, a
    /// node's project's as this machine's. The scope cache compares what it
    /// computes, so a minute that changes nothing publishes nothing.
    pub(super) fn tick_work_clock(&mut self, at: u64) -> bool {
        let minute = at / 60_000;
        if minute == self.work_clock_minute {
            return false;
        }
        self.work_clock_minute = minute;
        self.catalog_workspaces()
            .any(|workspace| !workspace.pull_requests.is_empty())
            && self.refresh_agent_scopes_at(at)
    }

    pub(super) fn refresh_agent_scopes(&mut self) -> bool {
        self.refresh_agent_scopes_at(crate::runtime::unix_milliseconds())
    }

    fn refresh_agent_scopes_at(&mut self, now: u64) -> bool {
        self.agent_scope_cache.refresh(&mut self.snapshot, now) | self.refresh_pane_headers()
    }

    /// Lays the block on this Mac's rows, whose pull requests come from
    /// their checkout's branch and their session. `live_panes` is Herdr's
    /// pane topology when the caller has one; only then do the verb records
    /// of panes outside it go.
    pub(super) fn lay_local_requests(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        live_panes: Option<&HashSet<String>>,
    ) {
        let places: HashMap<&str, (Option<&str>, Option<&str>, &str)> = self
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
                                pane.id.as_str(),
                                (
                                    checkout.branch.as_deref(),
                                    checkout.head_sha(),
                                    workspace.path.as_str(),
                                ),
                            )
                        })
                    })
                })
            })
            .collect();
        let verbs = &mut self.snapshot.ui_state.request_verbs;
        let mut changed = request_view::apply(
            agents,
            |pane| {
                places
                    .get(pane)
                    .map(|(branch, head_sha, root_path)| RowPlace {
                        branch: *branch,
                        head_sha: *head_sha,
                        root_path,
                    })
            },
            &self.github,
            verbs,
            unix_milliseconds(),
        );
        if let Some(live) = live_panes {
            self.prune_resolved_sessions(|pane| {
                !pane.starts_with("remote:") && !live.contains(pane)
            });
            let verbs = &mut self.snapshot.ui_state.request_verbs;
            changed |= request_view::prune_verbs(verbs, |pane| {
                !pane.starts_with("remote:")
                    && !live.contains(pane)
                    && !agents.iter().any(|row| row.pane_id == pane)
            });
        }
        if changed {
            self.persist_ui_state();
        }
    }

    /// Lays the block on a device's rows; their pull requests are not read
    /// (PRD Non-goals), so they carry none. `fetched` says the rows are a
    /// session the device just answered with, whose panes are the device's
    /// whole list; only then do the others' verb records go.
    pub(super) fn lay_remote_requests(
        &mut self,
        target_id: &str,
        agents: &mut [SidebarAgentSnapshot],
        fetched: bool,
    ) {
        let verbs = &mut self.snapshot.ui_state.request_verbs;
        let mut changed = request_view::apply(
            agents,
            |_| None,
            &crate::model::GithubSnapshot::default(),
            verbs,
            unix_milliseconds(),
        );
        let prefix = remote_pane_id_prefix(target_id);
        if fetched {
            self.prune_resolved_sessions(|pane| {
                pane.starts_with(&prefix) && !agents.iter().any(|row| row.pane_id == pane)
            });
            let verbs = &mut self.snapshot.ui_state.request_verbs;
            changed |= request_view::prune_verbs(verbs, |pane| {
                pane.starts_with(&prefix) && !agents.iter().any(|row| row.pane_id == pane)
            });
        }
        if changed {
            self.persist_ui_state();
        }
    }

    /// Lays the block again on every row already shown. Returns whether a
    /// row changed.
    pub(super) fn sync_request_rows(&mut self) -> bool {
        let mut agents = self.snapshot.navigator.agents.clone();
        self.lay_local_requests(&mut agents, None);
        let mut changed = agents != self.snapshot.navigator.agents;
        self.snapshot.navigator.agents = agents;
        let targets: Vec<String> = self
            .snapshot
            .status
            .remote
            .iter()
            .filter(|remote| remote.session.is_some())
            .map(|remote| remote.target_id.clone())
            .collect();
        for target_id in targets {
            let Some(mut agents) = self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == target_id)
                .and_then(|remote| remote.session.as_ref())
                .map(|session| session.agents.clone())
            else {
                continue;
            };
            self.lay_remote_requests(&target_id, &mut agents, false);
            if let Some(session) = self
                .snapshot
                .status
                .remote
                .iter_mut()
                .find(|remote| remote.target_id == target_id)
                .and_then(|remote| remote.session.as_mut())
                && session.agents != agents
            {
                session.agents = agents;
                changed = true;
            }
        }
        changed | self.sync_session_state() | self.refresh_agent_scopes()
    }

    /// A window began or stopped showing the request view.
    pub(super) fn observe_request_view(&mut self, observing: bool, now: std::time::Instant) {
        if observing && !self.request_view_observed {
            // Opening the view reads its projects already; the first re-read
            // is a minute later.
            self.pending_checks_read_at = Some(now);
        }
        self.request_view_observed = observing;
    }

    /// Once a minute while the request view is shown, asks GitHub again for
    /// each project a pull request has checks running in, this machine's or a
    /// node's (`reread_node_github`), so a finished run moves its row without
    /// a new turn (D-32, B12, work-tab-pr-lens B44).
    /// A project a row waits on running checks in is asked for whatever the
    /// window shows, so the row leaves progress when they end (status-marks
    /// B13); there are at most as many as such rows. Nothing is asked while
    /// neither holds.
    pub(crate) fn reread_pending_checks(&mut self, now: std::time::Instant) {
        let waits = self.checks_wait_projects().clone();
        if !self.request_view_observed && waits.is_empty() {
            return;
        }
        match self.pending_checks_read_at {
            // The read that showed the wait is a moment old: the first
            // re-read is a minute later.
            None => {
                self.pending_checks_read_at = Some(now);
                return;
            }
            Some(at) if now.duration_since(at) < PENDING_CHECKS_REREAD => return,
            Some(_) => {}
        }
        self.pending_checks_read_at = Some(now);
        if !self.request_view_observed {
            for project in waits {
                self.refresh_pull_requests(&project);
            }
            return;
        }
        // A pull request the Work list shows (D-25), an agent's or not, with
        // checks running counts: the lens and a card draw them (PRD
        // work-tab-pr-lens B44), a node's project's as this machine's. A
        // teammate's pull request nothing reaches is not read for.
        let mut running = BTreeSet::new();
        let mut node_running = BTreeSet::new();
        for workspace in self.catalog_workspaces().filter(|workspace| {
            self.catalog_github_project(workspace).is_some()
                && workspace.pull_requests.iter().any(|pull_request| {
                    !pull_request.badge.is_settled()
                        && pull_request.checks == crate::model::PullRequestChecks::Pending
                        && crate::agent_state::work::board::listed(pull_request)
                })
        }) {
            if workspace.device_id == self.node.as_str() {
                running.insert(workspace.path.clone());
            } else {
                node_running.insert((workspace.device_id.clone(), workspace.path.clone()));
            }
        }
        for (node, root) in node_running {
            self.reread_node_github(&node, &root);
        }
        // A row of the request view with a pull request of its own checks running.
        let agents: BTreeSet<String> = self
            .snapshot
            .navigator
            .agents
            .iter()
            .filter_map(|row| row.request.as_ref())
            .flat_map(|request| request.pull_requests.iter())
            .filter(|pull_request| {
                pull_request.live
                    && !pull_request.badge.is_settled()
                    && pull_request.checks == crate::model::PullRequestChecks::Pending
            })
            .filter_map(|pull_request| {
                self.github
                    .projects
                    .iter()
                    .find(|project| {
                        project
                            .pull_requests
                            .iter()
                            .any(|known| known.url == pull_request.url)
                    })
                    .map(|project| project.root_path.clone())
            })
            .collect();
        let running: BTreeSet<String> = running
            .union(&agents)
            .chain(waits.iter())
            .cloned()
            .collect();
        for project in running {
            self.refresh_pull_requests(&project);
        }
    }

    /// The operator opened a finished row in the request view: the pane is
    /// read, as a focus would make it, without moving the focus (D-29). A
    /// row asking a question keeps its demand.
    pub(super) fn open_result(&mut self, pane_id: &str) -> bool {
        // A result a settled pull request gave is seen once opened; the read
        // axis below covers an unread completion.
        let marked = request_view::open_result(
            &mut self.snapshot.ui_state.request_verbs,
            pane_id,
            unix_milliseconds(),
        );
        if marked {
            self.persist_ui_state();
        }
        let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
        let mut changes = Vec::new();
        if self
            .snapshot
            .navigator
            .agents
            .iter()
            .any(|row| row.pane_id == pane_id)
        {
            let mut agents = self.snapshot.navigator.agents.clone();
            changes = crate::agent_state::apply_read_state(
                &mut agents,
                &mut records,
                &mut self.local_read_connection,
                Some(pane_id),
            );
        } else {
            let remote = self.snapshot.status.remote.iter().find_map(|remote| {
                let session = remote.session.as_ref()?;
                session
                    .agents
                    .iter()
                    .any(|row| row.pane_id == pane_id)
                    .then(|| (remote.target_id.clone(), session.agents.clone()))
            });
            if let Some((target_id, mut agents)) = remote {
                changes = crate::agent_state::apply_read_state(
                    &mut agents,
                    &mut records,
                    self.read_connection(Some(&target_id)),
                    Some(pane_id),
                );
            }
        }
        self.snapshot.ui_state.pane_read_records = records;
        if changes.is_empty() {
            return marked && self.sync_request_rows();
        }
        self.record_read_record_changes(&changes);
        // The rows take the new record with the focus each server reports.
        let mut changed = self.refresh_pane_read_state();
        let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
        let mut remotes = std::mem::take(&mut self.snapshot.status.remote);
        for remote in &mut remotes {
            let Some(session) = remote.session.as_mut() else {
                continue;
            };
            let before = session.agents.clone();
            crate::agent_state::apply_read_state(
                &mut session.agents,
                &mut records,
                self.read_connection(Some(&remote.target_id)),
                session.focused_pane_id.as_deref(),
            );
            changed |= before != session.agents;
        }
        self.snapshot.status.remote = remotes;
        self.snapshot.ui_state.pane_read_records = records;
        changed | self.sync_request_rows()
    }
}
