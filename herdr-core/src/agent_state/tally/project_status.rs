//! The Project tool's 현황 (PRD lane-two-columns D-14, D-15, D-16, B27-B38):
//! one project's to-do list, its pull request counts and its worktrees'
//! state, derived from what its scope already holds (the 태스크 cards and
//! the PRs list's groups), the open asks its sessions' records hold (the
//! link worker's summaries) and the catalog's Git facts. It is rebuilt only
//! with the scope cache's compared input (B42), never stored, and the shell
//! only draws it. Sessions are the scope's own `buckets`.
use super::tasks::{TaskState, Tasks};
use crate::agent_state::work::board::Board;
use crate::links::LinkSummariesSnapshot;
use crate::model::{PrState, PullRequestChecks, SidebarAgentSnapshot, WorkspaceSnapshot};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

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
    /// How many of `todo_total` are agents' rows (asks and 다시 볼 것), the
    /// rest being pull requests: where the rows past the cap are found,
    /// the sidebar's 태스크 or PR list (B37).
    pub todo_agents: usize,
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
    /// The line stands for an open ask the session's record holds rather
    /// than a 태스크 card (B34).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub record: bool,
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

/// A session of this machine whose record holds an open ask (session
/// record D-12), by the agent and native session the ledger keys it by.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordAsk {
    pub agent: String,
    pub session_id: String,
}

/// The sessions with an open ask in each local project's record, keyed by
/// device and project. The summaries are the link worker's, read off the
/// runtime lock; a project on another device has no record here.
pub(super) fn record_asks(
    summaries: Option<&LinkSummariesSnapshot>,
    device: &str,
    projects: &[&WorkspaceSnapshot],
) -> HashMap<(String, String), Vec<RecordAsk>> {
    let Some(summaries) = summaries else {
        return HashMap::new();
    };
    projects
        .iter()
        .filter(|project| project.device_id == device)
        .filter_map(|project| {
            let asks: Vec<_> = summaries
                .projects
                .get(&project.id)?
                .work
                .iter()
                .filter(|work| work.open_asks > 0 && work.session.device_id == device)
                .map(|work| RecordAsk {
                    agent: work.session.agent.clone(),
                    session_id: work.session.session_id.clone(),
                })
                .collect();
            (!asks.is_empty()).then(|| ((project.device_id.clone(), project.id.clone()), asks))
        })
        .collect()
}

/// The status of one project.
///
/// 할 일's asks are the 태스크 cards standing in Needs You and 다시 볼 것, then
/// the live sessions whose record still holds an open ask the cards do not
/// already show (B34): one line per pane, the card's line winning, an ask
/// read or not (needs-you-cleanup B11), gone when an answer
/// resolves the ask in the record, 치우기 puts it away (B15) or the agent
/// works again. A closed
/// session's ask stays in its record and blocks nothing, so it is no line.
pub(super) fn project(
    project: &WorkspaceSnapshot,
    tasks: &Tasks,
    board: &Board,
    record_asks: &[RecordAsk],
    agents: &[&SidebarAgentSnapshot],
) -> ProjectStatus {
    let mut asks = Vec::new();
    let mut revisits = Vec::new();
    let mut asking = HashSet::new();
    for (state, list) in [
        (TaskState::NeedsYou, &mut asks),
        (TaskState::Revisit, &mut revisits),
    ] {
        for card in tasks.cards.iter().filter(|card| card.state == state) {
            asking.extend(card.members.iter().map(String::as_str));
            list.push((card.root.clone(), false));
        }
    }
    let panes: HashSet<_> = project
        .checkouts
        .iter()
        .flat_map(|checkout| &checkout.tabs)
        .flat_map(|tab| &tab.panes)
        .map(|pane| pane.id.as_str())
        .collect();
    for ask in record_asks {
        let row = agents.iter().find(|row| {
            panes.contains(row.pane_id.as_str())
                && row.session_id.as_deref() == Some(ask.session_id.as_str())
                && hide_session::Agent::from_kind(&row.agent_kind)
                    .is_some_and(|agent| agent.as_str() == ask.agent)
        });
        let Some(row) = row else { continue };
        if asking.contains(row.pane_id.as_str())
            || row.activity == "working"
            || row.resolved.is_some()
            || crate::agent_state::sessions::ask_put_away(row)
        {
            continue;
        }
        // Read or not, an unanswered ask is the operator's to answer
        // (needs-you-cleanup B11); only ☆ puts it off, and 치우기 away (B15).
        asking.insert(row.pane_id.as_str());
        asks.push((row.pane_id.clone(), true));
    }
    let mut todo: Vec<_> = asks
        .into_iter()
        .map(|line| ("ask", line))
        .chain(revisits.into_iter().map(|line| ("revisit", line)))
        .map(|(kind, (pane, record))| Todo {
            kind,
            pane_id: Some(pane),
            number: None,
            record,
        })
        .collect();
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
                record: false,
            });
        }
    }
    let todo_total = todo.len();
    let todo_agents = todo.iter().filter(|row| row.kind != "pr").count();
    todo.truncate(TODO_LIMIT);
    ProjectStatus {
        projected: true,
        todo,
        todo_total,
        todo_agents,
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
