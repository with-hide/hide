//! The Project tool's results column (PRD project-panel-results D-04, D-05,
//! B1-B17): one project's pull request counts, its worktrees' state and its
//! 흘린 것, derived from what its scope already holds (the PRs list's
//! groups and the agents' rows), the sessions its record says ran in each
//! worktree (the link worker's summaries) and the catalog's Git facts. It is
//! rebuilt only with the scope cache's compared input, never stored, and the
//! shell only draws it. Sessions are the scope's own `buckets`.
use crate::agent_state::work::board::Board;
use crate::links::LinkSummariesSnapshot;
use crate::model::{
    PrState, PullRequestBadge, PullRequestChecks, SidebarAgentSnapshot, WorkspaceSnapshot,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The most worktree marks one project publishes; the counts stay exact.
pub const DOT_LIMIT: usize = 96;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectStatus {
    /// Whether this is a project's status; a checkout's or a device's scope
    /// carries none and leaves it off the wire.
    #[serde(skip)]
    pub projected: bool,
    pub prs: PrCounts,
    pub git: GitCounts,
    /// 흘린 것 (PRD project-panel-results D-05, B6-B8): the worktrees where
    /// work ended unfinished, at most `STRAY_LIMIT`, the latest left first.
    pub strays: Vec<Stray>,
    /// Every such worktree, the ones past `STRAY_LIMIT` included: the
    /// rail's number.
    pub strays_total: usize,
}

/// The most 흘린 것 rows one project publishes; past it the oldest go,
/// `strays_total` keeps the real count and a diagnostic says so
/// (engineering principle 15).
pub const STRAY_LIMIT: usize = 64;

/// One worktree where nobody works or waits on the operator and something
/// is left: changes not committed, commits not pushed, or a pull request
/// whose CI failed (PRD project-panel-results D-05). A pull request a
/// reviewer only asked changes of is not one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Stray {
    pub checkout_id: String,
    pub branch: Option<String>,
    /// The task that last worked here: its live row's name, else the
    /// record's title for the session that last ran here (B6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The live pane of the session that last worked here, which a press
    /// brings forward and a hand-off writes to (B9, B11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// That session's agent, which a hand-off starts again in the worktree
    /// when its pane is gone (B11). Core only.
    #[serde(skip)]
    pub agent: Option<String>,
    pub changed_files: u32,
    pub unpushed: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_pr: Option<u32>,
    /// When work last happened here: the last row's change, the record's
    /// end or the last commit (B6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<u64>,
    /// The commit 놔두기 binds to (D-08). Core only.
    #[serde(skip)]
    pub head: Option<String>,
}

/// The operator's 놔두기 on one worktree (PRD project-panel-results D-08,
/// D-17): bound to the commit and the failing pull request it saw, so a new
/// commit or a new CI failure stands the row again.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, Serialize)]
pub struct StrayDismissal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_pr: Option<u32>,
    pub at_unix_ms: u64,
}

impl StrayDismissal {
    /// Whether this 놔두기 still holds `stray` down.
    pub(crate) fn holds(&self, stray: &Stray) -> bool {
        self.head == stray.head && (stray.failed_pr.is_none() || stray.failed_pr == self.failed_pr)
    }
}

/// How many 놔두기 the core keeps; past it the oldest goes and its row
/// stands again (B16).
pub const MAX_STRAY_DISMISSALS: usize = 256;

/// A session the record says ran in a project, for a worktree with no live
/// row: who it was and when it ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedRun {
    pub cwd: String,
    pub agent: String,
    pub title: String,
    pub ended_at_unix_ms: Option<u64>,
}

/// The open pull requests the PRs list holds, by where each stands (B29).
/// A draft counts in `open` and in no bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PrCounts {
    pub open: usize,
    pub mergeable: usize,
    pub failed: usize,
    /// Checks passed, a review still to come.
    pub review: usize,
    /// Checks still running or not reported.
    pub checking: usize,
}

/// The project's Git checkouts (B30).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GitCounts {
    pub worktrees: usize,
    /// Commits the primary checkout's upstream has that it does not; absent
    /// while unread or with no upstream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
    pub changed: usize,
    pub unpushed: usize,
    pub removable: usize,
    /// One mark per worktree, changed first, then unpushed, removable and
    /// clean, at most `DOT_LIMIT`: the tile's dots. Every checkout carries
    /// its own in its scope's `git_mark`, uncapped.
    pub dots: Vec<Dot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Dot {
    pub checkout_id: String,
    /// `changed`, `unpushed`, `removable` or `clean`.
    pub state: &'static str,
}

/// For each local worktree with something left, the last session its
/// project's record says ran there, keyed by device and project: what a 흘린
/// 것 row names and starts again when no live row works there (B6, B11).
/// Only worktrees with something left are looked up, so a publish reads no
/// more of the summaries than the rows it can make.
pub(super) fn recorded_runs(
    summaries: Option<&LinkSummariesSnapshot>,
    device: &str,
    projects: &[&WorkspaceSnapshot],
) -> HashMap<(String, String), Vec<RecordedRun>> {
    let Some(summaries) = summaries else {
        return HashMap::new();
    };
    projects
        .iter()
        .filter(|project| project.device_id == device && project.is_git)
        .filter_map(|project| {
            let work = &summaries.projects.get(&project.id)?.work;
            let runs: Vec<_> = project
                .checkouts
                .iter()
                .filter(|checkout| {
                    checkout.worktree.as_ref().is_some_and(|w| w.dirty)
                        || checkout.unpushed.as_ref().is_some_and(|u| u.count > 0)
                        || checkout.ahead > 0
                        || checkout
                            .pull_request
                            .as_ref()
                            .is_some_and(|pr| pr.checks == PullRequestChecks::Failed)
                })
                .filter_map(|checkout| {
                    work.iter()
                        .filter(|run| run.session.device_id == device)
                        .filter_map(|run| Some((run, run.cwd.as_deref()?)))
                        .filter(|(_, cwd)| {
                            crate::files::path_inside_root(&checkout.path, cwd, true).is_ok()
                        })
                        .max_by_key(|(run, _)| run.ended_at_unix_ms)
                        .map(|(run, cwd)| RecordedRun {
                            cwd: cwd.to_owned(),
                            agent: run.session.agent.clone(),
                            title: run.title.clone(),
                            ended_at_unix_ms: run.ended_at_unix_ms,
                        })
                })
                .collect();
            (!runs.is_empty()).then(|| ((project.device_id.clone(), project.id.clone()), runs))
        })
        .collect()
}

/// The status of one project: its pull request counts, its worktrees and
/// its 흘린 것, the 놔두기 still holding a row left out.
pub(super) fn project(
    project: &WorkspaceSnapshot,
    board: &Board,
    agents: &[&SidebarAgentSnapshot],
    dismissals: &BTreeMap<String, StrayDismissal>,
    recorded: &[RecordedRun],
) -> ProjectStatus {
    let mut strays: Vec<_> = project
        .checkouts
        .iter()
        .filter_map(|checkout| stray(project, checkout, agents, recorded))
        .filter(|row| {
            !dismissals
                .get(&row.checkout_id)
                .is_some_and(|dismissal| dismissal.holds(row))
        })
        .collect();
    strays.sort_by(|a, b| {
        b.since_unix_ms
            .cmp(&a.since_unix_ms)
            .then_with(|| a.checkout_id.cmp(&b.checkout_id))
    });
    let strays_total = strays.len();
    strays.truncate(STRAY_LIMIT);
    ProjectStatus {
        projected: true,
        prs: pr_counts(project, board),
        git: git_counts(project),
        strays,
        strays_total,
    }
}

/// A worktree's 흘린 것 row, before any 놔두기 (PRD project-panel-results
/// D-05, B6, B7): none on a checkout of the default branch, while an agent
/// there works or waits on the operator, or with nothing left.
pub(crate) fn stray(
    project: &WorkspaceSnapshot,
    checkout: &crate::model::CheckoutSnapshot,
    agents: &[&SidebarAgentSnapshot],
    recorded: &[RecordedRun],
) -> Option<Stray> {
    if !project.is_git {
        return None;
    }
    let worktree = checkout.worktree.as_ref().filter(|w| !w.missing)?;
    let branch = checkout.branch.as_deref()?;
    if checkout.base_branch.as_deref() == Some(branch) {
        return None;
    }
    let changed_files = if worktree.dirty {
        worktree.changed_file_count.max(1)
    } else {
        0
    };
    // The same rule the worktree's mark reads (`checkout_mark`).
    let unpushed = match &checkout.unpushed {
        Some(unpushed) => unpushed.count,
        None if checkout.pull_request.is_none() && !checkout.landed => checkout.ahead,
        None => 0,
    };
    // CI alone: a reviewer's change request is the PR list's to show.
    let failed_pr = checkout
        .pull_request
        .as_ref()
        .filter(|pr| {
            matches!(pr.badge, PullRequestBadge::Open | PullRequestBadge::Review)
                && !pr.is_draft
                && pr.checks == PullRequestChecks::Failed
        })
        .map(|pr| pr.number);
    if changed_files == 0 && unpushed == 0 && failed_pr.is_none() {
        return None;
    }
    let panes: HashSet<&str> = checkout
        .tabs
        .iter()
        .flat_map(|tab| &tab.panes)
        .map(|pane| pane.id.as_str())
        .collect();
    let rows: Vec<_> = agents
        .iter()
        .filter(|row| panes.contains(row.pane_id.as_str()))
        .collect();
    if rows.iter().any(|row| {
        matches!(
            crate::agent_state::turn::AgentUse::of(&row.demand, row.blocked, &row.activity),
            crate::agent_state::turn::AgentUse::Working
                | crate::agent_state::turn::AgentUse::Waiting
        )
    }) {
        return None;
    }
    let last = rows.iter().max_by_key(|row| row.changed_at_unix_ms);
    let run = recorded
        .iter()
        .filter(|run| crate::files::path_inside_root(&checkout.path, &run.cwd, true).is_ok())
        .max_by_key(|run| run.ended_at_unix_ms);
    let commit = worktree
        .last_commit_unix_seconds
        .map(|seconds| seconds.saturating_mul(1000));
    let since = [
        last.and_then(|row| row.changed_at_unix_ms),
        run.and_then(|run| run.ended_at_unix_ms),
        commit,
    ]
    .into_iter()
    .flatten()
    .max();
    Some(Stray {
        checkout_id: checkout.id.clone(),
        branch: Some(branch.to_owned()),
        name: last
            .map(|row| row.identity_label.clone())
            .or_else(|| run.map(|run| run.title.clone()))
            .filter(|name| !name.is_empty()),
        pane_id: last.map(|row| row.pane_id.clone()),
        agent: last
            .and_then(|row| hide_session::Agent::from_kind(&row.agent_kind))
            .map(|agent| agent.as_str().to_owned())
            .or_else(|| run.map(|run| run.agent.clone())),
        changed_files,
        unpushed,
        failed_pr,
        since_unix_ms: since,
        head: checkout.head_sha().map(str::to_owned),
    })
}

fn pr_counts(project: &WorkspaceSnapshot, board: &Board) -> PrCounts {
    let mut counts = PrCounts::default();
    for row in board.rows.iter().filter(|row| row.group != "merged") {
        let Some(pr) = project
            .pull_requests
            .iter()
            .find(|pr| pr.number == row.number)
        else {
            continue;
        };
        counts.open += 1;
        match pr.state() {
            PrState::Mergeable => counts.mergeable += 1,
            PrState::Failed => counts.failed += 1,
            PrState::Pending if pr.checks == PullRequestChecks::Passing => counts.review += 1,
            PrState::Pending => counts.checking += 1,
            PrState::Draft | PrState::Merged | PrState::Closed => {}
        }
    }
    counts
}

/// A Git checkout's mark (B30): `changed`, `unpushed`, `removable` or
/// `clean`, or none while its worktree is unread or missing. The tile's dots
/// and each checkout's own scope read this one rule.
pub(super) fn checkout_mark(
    project: &WorkspaceSnapshot,
    checkout: &crate::model::CheckoutSnapshot,
) -> Option<&'static str> {
    if !project.is_git {
        return None;
    }
    let worktree = checkout.worktree.as_ref().filter(|w| !w.missing)?;
    // Not pushed: commits the upstream lacks, or commits of its own with
    // nowhere they were pushed to and no pull request carrying them.
    let unpushed = checkout.unpushed.as_ref().is_some_and(|u| u.count > 0)
        || (checkout.unpushed.is_none()
            && checkout.ahead > 0
            && checkout.pull_request.is_none()
            && !checkout.landed);
    Some(if worktree.dirty {
        "changed"
    } else if unpushed {
        "unpushed"
    } else if checkout.landed && !checkout.is_primary && !worktree.is_main {
        "removable"
    } else {
        "clean"
    })
}

fn git_counts(project: &WorkspaceSnapshot) -> GitCounts {
    let mut counts = GitCounts::default();
    let mut dots = Vec::new();
    for checkout in &project.checkouts {
        let Some(state) = checkout_mark(project, checkout) else {
            continue;
        };
        counts.worktrees += 1;
        if checkout.is_primary {
            counts.behind = checkout.worktree.as_ref().and_then(|w| w.behind_upstream);
        }
        match state {
            "changed" => counts.changed += 1,
            "unpushed" => counts.unpushed += 1,
            "removable" => counts.removable += 1,
            _ => {}
        }
        dots.push(Dot {
            checkout_id: checkout.id.clone(),
            state,
        });
    }
    let rank = |state: &str| match state {
        "changed" => 0,
        "unpushed" => 1,
        "removable" => 2,
        _ => 3,
    };
    dots.sort_by_key(|dot| rank(dot.state));
    dots.truncate(DOT_LIMIT);
    counts.dots = dots;
    counts
}

impl ProjectStatus {
    pub(crate) fn is_absent(&self) -> bool {
        !self.projected
    }
}
