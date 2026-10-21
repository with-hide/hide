//! Each agent row's own session facts: its group, the line its task row
//! draws under the title (docs/status-model.md, The sidebar's 태스크 list),
//! whether a resolve holds it, and its own PR summary. Which task a row
//! belongs to and in what order the tasks stand is `tally/tasks.rs`'s.
//! Memory and conversation history are separate readers, not session state.
use crate::model::{PrState, SidebarAgentSnapshot};
use serde::{Deserialize, Serialize};

/// How long a resolved session stays under Resolved (PRD D-23).
pub const RESOLVED_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveSource {
    Operator,
    Automatic,
}

/// A record written before the 24-hour window also carried its local date;
/// serde ignores that field on read and the next save drops it.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Resolution {
    pub at_unix_ms: u64,
    pub source: ResolveSource,
    pub session: Option<String>,
    pub activity: String,
}

/// A merged pull request settles the session only once the row is at rest
/// with nothing left to see: an unread finish stays under 읽을 것 and a wait
/// under 대기, rather than resting with a mark that says otherwise.
pub(crate) fn auto_resolvable(agent: &SidebarAgentSnapshot, input_after: Option<u64>) -> bool {
    if agent.activity != "stopped"
        || agent.blocked
        || agent.demand != "none"
        || agent.group != "seen"
    {
        return false;
    }
    let mut assigned = agent
        .request
        .iter()
        .flat_map(|r| &r.pull_requests)
        .filter(|p| p.duty)
        .peekable();
    assigned.peek().is_some()
        && assigned.all(|p| {
            p.badge.is_settled()
                && input_after.is_none_or(|input| p.settled_at_unix_ms.is_some_and(|at| at > input))
        })
}

/// The agent's own state names the group, the same names the sidebar uses
/// (PRD D-16): no PR stage and no tag decides it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    NeedsYou,
    Working,
    Done,
    #[default]
    Idle,
    Resolved,
}

pub const GROUPS: [Group; 5] = [
    Group::NeedsYou,
    Group::Working,
    Group::Done,
    Group::Idle,
    Group::Resolved,
];

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Row {
    pub group: Group,
    /// What the session is doing or has done (PRD B31); absent rather than
    /// invented. A Needs You row draws its `ask` instead.
    pub line: Option<String>,
    /// The line is the remaining work of an unfinished turn (`일 남음`).
    pub unfinished: bool,
}

pub(crate) fn row(agent: &SidebarAgentSnapshot) -> Row {
    let label_line = || agent.request.as_ref().and_then(|r| r.line.clone());
    let unfinished = super::axes::stopped_unfinished(agent);
    if agent.resolved.is_some() {
        return Row {
            group: Group::Resolved,
            ..Row::default()
        };
    }
    match agent.group.as_str() {
        // A Needs You row draws its ask; a block has none and draws its
        // cause (agent-blocked-state B1).
        "needs_you" => Row {
            group: Group::NeedsYou,
            line: (agent.demand == "error" && agent.unread)
                .then(|| agent.detail.clone())
                .flatten(),
            unfinished: false,
        },
        "working" => Row {
            group: Group::Working,
            line: if agent.wait == Some(crate::model::AgentWait::Children) {
                agent.descendant_line.clone()
            } else {
                label_line()
            },
            unfinished: false,
        },
        "done" => Row {
            group: Group::Done,
            line: label_line(),
            unfinished: false,
        },
        // A read block rests here with its cause, ahead of the other Idle
        // rows (agent-blocked-state B2).
        _ if agent.demand == "error" => Row {
            group: Group::Idle,
            line: agent.detail.clone(),
            unfinished: true,
        },
        _ => Row {
            group: Group::Idle,
            line: label_line().filter(|_| unfinished),
            unfinished,
        },
    }
}

/// The PRs this row holds the duty of, plus its provisional history for
/// display only, never its descendants' (PRD D-18,
/// D-39): how many, the worst state, and each PR as an
/// index into `request.pull_requests`, worst first, each in the one PR state
/// ([`PrState::of`]). Closed PRs are not counted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrSummary {
    pub count: usize,
    pub worst: PrState,
    pub pulls: Vec<OwnPr>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct OwnPr {
    pub index: usize,
    pub state: PrState,
}

pub(crate) fn own_prs(agent: &SidebarAgentSnapshot) -> Option<PrSummary> {
    let mut pulls: Vec<OwnPr> = agent
        .request
        .iter()
        .flat_map(|request| request.pull_requests.iter().enumerate())
        .filter(|(_, pull)| (pull.duty || pull.provisional) && pull.live)
        .map(|(index, pull)| OwnPr {
            index,
            state: pull.state(),
        })
        .filter(|pull| pull.state != PrState::Closed)
        .collect();
    pulls.sort_by_key(|pull| pull.state);
    let worst = pulls.first()?.state;
    Some(PrSummary {
        count: pulls.len(),
        worst,
        pulls,
    })
}
