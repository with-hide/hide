//! Each agent row's own session facts: its group, the line its task row
//! draws under the title (docs/status-model.md, The sidebar's 태스크 list),
//! whether a resolve holds it, and its own PR summary. Which task a row
//! belongs to and in what order the tasks stand is `tally/tasks.rs`'s.
//! Memory and conversation history are separate readers, not session state.
use crate::model::SidebarAgentSnapshot;
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

/// How many tasks may carry the operator's ★ at once (PRD revisit-group D-15).
pub const MAX_STARRED_SESSIONS: usize = 256;

/// The operator's 다시 볼 것 decisions about one root session (PRD
/// revisit-group D-03, D-05): the ★ and the read ask 다 봤음 put away. Bound
/// to the session the pane hosted when it was written, like a resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct RevisitMark {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starred_at_unix_ms: Option<u64>,
    /// The stamp (`ask_stamp`) of the read ask 치우기 or 다 봤음 put away:
    /// reading leaves it alone and a later ask has another, which stands
    /// again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_ask_since: Option<u64>,
    pub session: Option<String>,
}

impl RevisitMark {
    pub fn is_empty(&self) -> bool {
        self.starred_at_unix_ms.is_none() && self.dismissed_ask_since.is_none()
    }
}

/// What a row carries for 다시 볼 것 (PRD revisit-group D-02, D-03): the
/// operator's ★, an ask the operator read and has not answered, and since
/// when it waits, the older of the two.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Revisit {
    pub starred: bool,
    pub read_ask: bool,
    pub since_unix_ms: u64,
}

/// An ask the operator read and has not answered: a question or an approval
/// that no longer holds the row in Needs You (a block holds it there, D-12),
/// on a root that is not working or resolved (해결 puts it away, D-14), and
/// not the one 다 봤음 put away. The ask is known by its `ask_stamp`.
pub(crate) fn revisit_of(
    agent: &SidebarAgentSnapshot,
    mark: Option<&RevisitMark>,
) -> Option<Revisit> {
    let read_ask =
        read_open_ask(agent) && mark.and_then(|m| m.dismissed_ask_since) != ask_stamp(agent);
    let starred_at = mark.and_then(|m| m.starred_at_unix_ms);
    let since = [read_ask.then(|| ask_stamp(agent)).flatten(), starred_at]
        .into_iter()
        .flatten()
        .min()?;
    Some(Revisit {
        starred: starred_at.is_some(),
        read_ask,
        since_unix_ms: since,
    })
}

/// A read, unanswered question or approval at rest on a root, whether or not
/// 치우기 put it away.
fn read_open_ask(agent: &SidebarAgentSnapshot) -> bool {
    agent.lineage_parent_pane_id.is_none()
        && matches!(agent.demand.as_str(), "question" | "approval")
        && !agent.blocked
        && !agent.unread
        && agent.activity != "working"
        && agent.resolved.is_none()
        && ask_stamp(agent).is_some()
}

/// The stamp an ask is known by: when the core laid it (a plan review or a
/// mission decision, which leave the agent's state alone), else the stamp of
/// the state it stands in, which the operator reading it does not move.
pub(crate) fn ask_stamp(agent: &SidebarAgentSnapshot) -> Option<u64> {
    agent.laid_since_unix_ms.or(agent.changed_at_unix_ms)
}

/// Whether 치우기 put the row's read ask away (needs-you-cleanup B15): the ask
/// stands but the row carries no read ask, so nothing lists it until the agent
/// asks again.
pub(crate) fn ask_put_away(agent: &SidebarAgentSnapshot) -> bool {
    read_open_ask(agent) && !agent.revisit.is_some_and(|revisit| revisit.read_ask)
}

/// Why the operator cannot resolve the row now: work that still runs or
/// waits, or an approval prompt that holds the agent, would rest a task
/// whose mark says it is not at rest.
pub(crate) fn resolve_refusal(agent: &SidebarAgentSnapshot) -> Option<&'static str> {
    if super::has_work_in_progress(agent) {
        Some("This agent is working")
    } else if agent.blocked {
        Some("This agent waits for an approval")
    } else {
        None
    }
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
