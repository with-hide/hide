//! The Project tool's 현황 (PRD lane-two-columns D-14, D-15, D-16, B27-B38):
//! one project's to-do list, its pull request counts and its worktrees'
//! state, derived from what its scope already holds (the 태스크 cards and
//! the PRs list's groups) and the catalog's Git facts. It is rebuilt only
//! with the scope cache's compared input (B42), never stored, and the shell
//! only draws it. Sessions are the scope's own `buckets`.
use super::tasks::{TaskState, Tasks};
use crate::agent_state::work::board::Board;
use crate::model::{PrState, PullRequestChecks, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::HashSet;

/// The most to-do rows one project publishes; past it the oldest of the
/// list's order go, `todo_total` keeps the real count and a diagnostic
/// says so (engineering principle 15).
pub const TODO_LIMIT: usize = 64;

/// The most worktree marks one project publishes; the counts stay exact.
pub const DOT_LIMIT: usize = 96;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProjectStatus {
    /// Whether this is a project's status; a checkout's or a device's scope
    /// carries none and leaves it off the wire.
    #[serde(skip)]
    pub projected: bool,
    /// In the order the 할 일 list draws them: new asks, then 다시 볼 것,
    /// then the operator's pull requests and reviews (B37).
    pub todo: Vec<Todo>,
    /// Every to-do row, the ones past `TODO_LIMIT` included: the rail's
    /// number (B38).
    pub todo_total: usize,
    pub prs: PrCounts,
    pub git: GitCounts,
}

/// One row the operator's move unblocks (D-16). The shell draws the agent's
/// row by `pane_id` and the pull request by `number`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Todo {
    /// `ask` (a question or decision not read yet), `revisit` (a read,
    /// unanswered ask or a ★, B34) or `pr` (a Needs You pull request, B35).
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
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
    /// clean, at most `DOT_LIMIT`.
    pub dots: Vec<Dot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Dot {
    pub checkout_id: String,
    /// `changed`, `unpushed`, `removable` or `clean`.
    pub state: &'static str,
}

/// The status of one project.
pub(super) fn project(project: &WorkspaceSnapshot, tasks: &Tasks, board: &Board) -> ProjectStatus {
    let mut todo = Vec::new();
    let mut asking = HashSet::new();
    for (kind, state) in [
        ("ask", TaskState::NeedsYou),
        ("revisit", TaskState::Revisit),
    ] {
        for card in tasks.cards.iter().filter(|card| card.state == state) {
            asking.extend(card.members.iter().map(String::as_str));
            todo.push(Todo {
                kind,
                pane_id: Some(card.root.clone()),
                number: None,
            });
        }
    }
    for row in &board.rows {
        // An agent asking about its pull request is already its ask row.
        let asked = row.reason.kind == "reply"
            && row
                .agent_pane
                .as_deref()
                .is_some_and(|pane| asking.contains(pane));
        if row.group == "needs_you" && row.fold.is_none() && !asked {
            todo.push(Todo {
                kind: "pr",
                pane_id: None,
                number: Some(row.number),
            });
        }
    }
    let todo_total = todo.len();
    todo.truncate(TODO_LIMIT);
    ProjectStatus {
        projected: true,
        todo,
        todo_total,
        prs: pr_counts(project, board),
        git: git_counts(project),
    }
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

fn git_counts(project: &WorkspaceSnapshot) -> GitCounts {
    let mut counts = GitCounts::default();
    if !project.is_git {
        return counts;
    }
    let mut dots = Vec::new();
    for checkout in &project.checkouts {
        let Some(worktree) = checkout.worktree.as_ref().filter(|w| !w.missing) else {
            continue;
        };
        counts.worktrees += 1;
        if checkout.is_primary {
            counts.behind = worktree.behind_upstream;
        }
        // Not pushed: commits the upstream lacks, or commits of its own with
        // nowhere they were pushed to and no pull request carrying them.
        let unpushed = checkout.unpushed.as_ref().is_some_and(|u| u.count > 0)
            || (checkout.unpushed.is_none()
                && checkout.ahead > 0
                && checkout.pull_request.is_none()
                && !checkout.landed);
        let state = if worktree.dirty {
            counts.changed += 1;
            "changed"
        } else if unpushed {
            counts.unpushed += 1;
            "unpushed"
        } else if checkout.landed && !checkout.is_primary && !worktree.is_main {
            counts.removable += 1;
            "removable"
        } else {
            "clean"
        };
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
