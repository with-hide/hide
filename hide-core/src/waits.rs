//! Hide's one wait (PRD core-wait): something a mission node (`kind: wait`)
//! or an agent (`hide wait`) waits on, the deadline it waits until and how it
//! ended. This module holds the model and the rules, with no I/O and no
//! clock of its own; `runtime/waits.rs` keeps the agents' waits and
//! `runtime/mission_outside.rs` reads the code host for both.
//!
//! A wait is judged by its state, again and again, not by the events that
//! moved it (D-11): what the core already reads (a project's list of open
//! change requests, a change request by its number, a branch against the
//! default branch, the mission book) says whether it is still open, met or
//! failed, and the same answer read twice ends it once.

use serde::{Deserialize, Serialize};

use crate::missions::{MissionBook, MissionStatus, NodeStatus};
use crate::model::{CiEnd, PullRequestBadge, PullRequestSnapshot};

/// Open waits one agent holds (D-13): the next is refused, the open ones stay.
pub const AGENT_WAIT_LIMIT: usize = 8;
/// Open waits on this machine, every agent's and every wait node's (D-13).
pub const MACHINE_WAIT_LIMIT: usize = 256;
/// The deadline a wait gets when none is named: seven days (D-07).
pub const DEFAULT_WAIT_MINUTES: u32 = 7 * 24 * 60;
/// The shortest deadline a wait may name: an hour (D-07).
pub const MIN_WAIT_MINUTES: u32 = 60;
/// The longest deadline a wait may name: thirty days (D-07).
pub const MAX_WAIT_MINUTES: u32 = 30 * 24 * 60;

const MINUTE_MS: u64 = 60_000;

/// What a wait waits on (D-02).
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WaitTarget {
    /// A change request, until it merges (met) or closes without a merge.
    Pr(u32),
    /// A branch, until its own work reaches the default branch.
    Branch(String),
    /// The checks on a change request's current head, until they all end.
    Ci(u32),
    /// Another mission on this machine, until it closes as done.
    Mission(String),
    /// A node of another mission on this machine, until it is done.
    Node { mission: String, node: String },
}

impl WaitTarget {
    /// The change request it reads by number, for a pull request or its CI.
    pub fn change_request(&self) -> Option<u32> {
        match self {
            Self::Pr(number) | Self::Ci(number) => Some(*number),
            _ => None,
        }
    }

    /// Whether the project's code host answers it: a change request, its CI
    /// or a branch. A mission or node target is the book's.
    pub fn code_host(&self) -> bool {
        matches!(self, Self::Pr(_) | Self::Ci(_) | Self::Branch(_))
    }

    /// The mission it names, for a mission or node target.
    pub fn mission(&self) -> Option<&str> {
        match self {
            Self::Mission(id) | Self::Node { mission: id, .. } => Some(id),
            _ => None,
        }
    }

    /// The target in a few words, as `hide wait list` and a cell name it.
    pub fn words(&self) -> String {
        match self {
            Self::Pr(number) => format!("#{number}"),
            Self::Branch(branch) => format!("branch {branch}"),
            Self::Ci(number) => format!("#{number} CI"),
            Self::Mission(id) => id.clone(),
            Self::Node { mission, node } => format!("{mission} node {node}"),
        }
    }
}

/// A wait as a plan entry or `hide wait` names it (D-02, B1, B14): one
/// target, `ci` only beside `pr`, `node` only beside `mission`, and an
/// optional deadline (`for`, `30m`, `2h` or `3d`).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaitInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ci: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
    pub for_span: Option<String>,
}

/// Why a wait's input is refused, before anything is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputRefusal {
    /// No target, two targets, `ci` without `pr` or `node` without `mission`.
    Target,
    /// The deadline is not `30m`, `2h` or `3d` between an hour and 30 days.
    Deadline,
}

impl WaitInput {
    /// The one target it names, or why not.
    pub fn target(&self) -> Result<WaitTarget, InputRefusal> {
        let named = [
            self.pr.is_some(),
            self.branch.is_some(),
            self.mission.is_some(),
        ]
        .iter()
        .filter(|named| **named)
        .count();
        if named != 1
            || (self.ci && self.pr.is_none())
            || (self.node.is_some() && self.mission.is_none())
        {
            return Err(InputRefusal::Target);
        }
        let key = |text: &str| crate::delivery::valid_key(text);
        Ok(match (self.pr, &self.branch, &self.mission, &self.node) {
            (Some(number), ..) if number > 0 && self.ci => WaitTarget::Ci(number),
            (Some(number), ..) if number > 0 => WaitTarget::Pr(number),
            (_, Some(branch), ..) if valid_branch(branch) => WaitTarget::Branch(branch.clone()),
            (_, _, Some(mission), None) if key(mission) => WaitTarget::Mission(mission.clone()),
            (_, _, Some(mission), Some(node)) if key(mission) && valid_node_key(node) => {
                WaitTarget::Node {
                    mission: mission.clone(),
                    node: node.clone(),
                }
            }
            _ => return Err(InputRefusal::Target),
        })
    }

    /// The deadline it names in minutes, the default when it names none.
    pub fn minutes(&self) -> Result<u32, InputRefusal> {
        match &self.for_span {
            None => Ok(DEFAULT_WAIT_MINUTES),
            Some(span) => span_minutes(span).ok_or(InputRefusal::Deadline),
        }
    }
}

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && !branch.starts_with('-')
        && branch.len() <= 256
        && !branch
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
}

fn valid_node_key(key: &str) -> bool {
    crate::dag::valid_key(key)
}

/// `30m`, `2h` or `3d` in minutes, between `MIN_WAIT_MINUTES` and
/// `MAX_WAIT_MINUTES`; anything else is none.
pub fn span_minutes(span: &str) -> Option<u32> {
    let count = |digits: &str| digits.parse::<u32>().ok();
    let minutes = if let Some(digits) = span.strip_suffix('m') {
        count(digits)
    } else if let Some(digits) = span.strip_suffix('h') {
        count(digits).and_then(|hours| hours.checked_mul(60))
    } else if let Some(digits) = span.strip_suffix('d') {
        count(digits).and_then(|days| days.checked_mul(24 * 60))
    } else {
        None
    };
    minutes.filter(|minutes| (MIN_WAIT_MINUTES..=MAX_WAIT_MINUTES).contains(minutes))
}

/// The deadline `minutes` from `now`.
pub fn deadline(now_unix_ms: u64, minutes: u32) -> u64 {
    now_unix_ms.saturating_add(u64::from(minutes) * MINUTE_MS)
}

/// How a wait ended (D-03, D-04, D-05, D-07). The met ones are `merged`,
/// `ci_passed`, `mission_done` and `node_done`; every other is a failure,
/// which a mission node raises to its Conductor and an agent is woken with.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WaitOutcome {
    Merged,
    ClosedUnmerged,
    CiPassed,
    CiFailed {
        /// The checks that failed on the head, by name, as the list read them.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checks: Vec<String>,
    },
    MissionDone,
    MissionCancelled,
    NodeDone,
    NodeDropped,
    Expired,
}

/// The most failed check names one outcome keeps.
const FAILED_CHECK_NAMES: usize = 8;

impl WaitOutcome {
    /// Whether the wait was met; a failure is anything else.
    pub fn met(&self) -> bool {
        matches!(
            self,
            Self::Merged | Self::CiPassed | Self::MissionDone | Self::NodeDone
        )
    }

    /// The outcome's key, as the snapshot and a cell name it
    /// (`contracts/snapshot-wire-enums.json`: `wait_outcome`).
    pub fn key(&self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::ClosedUnmerged => "closed_unmerged",
            Self::CiPassed => "ci_passed",
            Self::CiFailed { .. } => "ci_failed",
            Self::MissionDone => "mission_done",
            Self::MissionCancelled => "mission_cancelled",
            Self::NodeDone => "node_done",
            Self::NodeDropped => "node_dropped",
            Self::Expired => "expired",
        }
    }

    /// The raise a mission node's failed wait carries to its Conductor
    /// (D-08); none for a met outcome.
    pub fn attention(&self) -> Option<crate::missions::AttentionReason> {
        use crate::missions::AttentionReason;
        Some(match self {
            Self::ClosedUnmerged => AttentionReason::PrClosed,
            Self::CiFailed { .. } => AttentionReason::CiFailed,
            Self::MissionCancelled => AttentionReason::MissionCancelled,
            Self::NodeDropped => AttentionReason::NodeDropped,
            Self::Expired => AttentionReason::WaitExpired,
            Self::Merged | Self::CiPassed | Self::MissionDone | Self::NodeDone => return None,
        })
    }

    /// The outcome in plain words after the target, the first line of the
    /// agent's notice (B17): `#1232 CI failed: build, lint`.
    pub fn sentence(&self, target: &WaitTarget) -> String {
        let what = target.words();
        match self {
            Self::Merged => format!("{what} merged"),
            Self::ClosedUnmerged => format!("{what} closed without a merge"),
            Self::CiPassed => format!("{what} passed"),
            Self::CiFailed { checks } if checks.is_empty() => format!("{what} failed"),
            Self::CiFailed { checks } => format!("{what} failed: {}", checks.join(", ")),
            Self::MissionDone => format!("{what} closed as done"),
            Self::MissionCancelled => format!("{what} was cancelled"),
            Self::NodeDone => format!("{what} is done"),
            Self::NodeDropped => format!("{what} was dropped from its plan"),
            Self::Expired => format!("the wait for {what} passed its deadline"),
        }
    }
}

/// What a change request's list entry says of a target that names it:
/// still open (`None`), or how it ended. A CI target is judged on the head
/// the list read, so a new push waits for the new head's checks (D-05); a
/// head with no checks reported yet waits too, since checks registered a
/// moment after a push would otherwise read as passed.
pub(crate) fn judge_listed(target: &WaitTarget, pull: &PullRequestSnapshot) -> Option<WaitOutcome> {
    match pull.badge {
        PullRequestBadge::Merged => return Some(WaitOutcome::Merged),
        PullRequestBadge::Closed => return Some(WaitOutcome::ClosedUnmerged),
        _ => {}
    }
    if !matches!(target, WaitTarget::Ci(_)) {
        return None;
    }
    match pull.ci_end() {
        CiEnd::Ended(crate::missions::CiResult::Passed) => Some(WaitOutcome::CiPassed),
        CiEnd::Ended(_) => Some(WaitOutcome::CiFailed {
            checks: pull
                .work
                .failed_checks
                .iter()
                .filter_map(|check| check.name.clone())
                .take(FAILED_CHECK_NAMES)
                .collect(),
        }),
        CiEnd::NoChecks | CiEnd::Unread | CiEnd::Running => None,
    }
}

/// What a change request's standing by number says of a target that names
/// it and that the project's list does not hold: an open one is still
/// waited on (for a CI target the list holds every open change request the
/// read took, and the next read judges it there).
pub(crate) fn judge_standing(standing: crate::code_host::Standing) -> Option<WaitOutcome> {
    match standing {
        crate::code_host::Standing::Open => None,
        crate::code_host::Standing::Merged => Some(WaitOutcome::Merged),
        crate::code_host::Standing::Closed => Some(WaitOutcome::ClosedUnmerged),
    }
}

/// What the book says of a mission or node target (D-03, D-04): none while
/// it is open, and for a code-host target. A mission closed as `merge_wait`
/// is still waited on; a node is met once it is done, whatever its pull
/// request does after (a node waits for its own done, not for main). A
/// mission shelved out of the book is read from its line. A node its plan
/// no longer holds was dropped; a mission the book no longer names at all is
/// left to the deadline, and the log says so where it is noticed.
pub(crate) fn judge_book(book: &MissionBook, target: &WaitTarget) -> Option<WaitOutcome> {
    let mission_id = target.mission()?;
    let status = book
        .get(mission_id)
        .map(|mission| mission.status)
        .or_else(|| {
            book.shelved
                .iter()
                .find(|line| line.id == mission_id)
                .map(|line| line.status)
        })?;
    match target {
        WaitTarget::Mission(_) => match status {
            MissionStatus::Done => Some(WaitOutcome::MissionDone),
            MissionStatus::Cancelled => Some(WaitOutcome::MissionCancelled),
            MissionStatus::Planning | MissionStatus::Running | MissionStatus::MergeWait => None,
        },
        WaitTarget::Node { node, .. } => {
            let Some(mission) = book.get(mission_id) else {
                // Shelved: closed. A node done stays done in its file, which
                // only a reopen reads back; a cancelled one ends the wait.
                return (status == MissionStatus::Cancelled)
                    .then_some(WaitOutcome::MissionCancelled);
            };
            match mission.nodes.iter().find(|kept| kept.key == *node) {
                None => Some(WaitOutcome::NodeDropped),
                Some(kept) => match kept.status {
                    NodeStatus::Done => Some(WaitOutcome::NodeDone),
                    NodeStatus::Dropped => Some(WaitOutcome::NodeDropped),
                    _ if status == MissionStatus::Cancelled => Some(WaitOutcome::MissionCancelled),
                    _ => None,
                },
            }
        }
        WaitTarget::Pr(_) | WaitTarget::Ci(_) | WaitTarget::Branch(_) => None,
    }
}

/// A mission's progress as a wait on it shows it (D-16): finished nodes of
/// all, a dropped one counted as finished, as `끝 2/7`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WaitProgress {
    pub done: u32,
    pub total: u32,
}

/// The progress of mission `id`, when the book holds its nodes.
pub(crate) fn progress(book: &MissionBook, id: &str) -> Option<WaitProgress> {
    let mission = book.get(id)?;
    let count = |filter: &dyn Fn(&crate::missions::MissionNode) -> bool| {
        u32::try_from(mission.nodes.iter().filter(|node| filter(node)).count()).unwrap_or(u32::MAX)
    };
    Some(WaitProgress {
        done: count(&|node| node.status.finished()),
        total: count(&|_| true),
    })
}

/// The missions mission `from` would wait on, directly or through the
/// missions those wait on, by its wait nodes and the waits `more` names
/// (each `(waiting mission, target mission)`): a wait from `from` on one of
/// them, or on `from` itself, closes a loop (D-22, B10).
pub(crate) fn waits_on(
    book: &MissionBook,
    from: &str,
    more: &[(String, String)],
) -> std::collections::BTreeSet<String> {
    let edges = |id: &str| -> Vec<String> {
        let mut out: Vec<String> = book
            .get(id)
            .into_iter()
            .flat_map(|mission| &mission.nodes)
            .filter(|node| !node.status.finished())
            .filter_map(|node| node.wait_target.as_ref())
            .filter_map(|target| target.mission().map(str::to_owned))
            .collect();
        out.extend(
            more.iter()
                .filter(|(waiting, _)| waiting == id)
                .map(|(_, target)| target.clone()),
        );
        out
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut next = vec![from.to_owned()];
    while let Some(id) = next.pop() {
        for target in edges(&id) {
            if seen.insert(target.clone()) {
                next.push(target);
            }
        }
    }
    seen
}

/// One agent's wait (D-21), kept in `core-state.json` so a restart goes on
/// with it (B26). Ended, it stays only until its agent has seen the notice
/// that tells it, and it no longer counts against a limit (D-13).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentWait {
    /// `wait-<n>`, from the store's count.
    pub id: String,
    /// The registration that waits; its end ends the wait (D-22).
    pub agent_id: String,
    /// The pane and session the agent was in when it asked, which a notice
    /// and a wake find it by while it has no live observation.
    pub actor: crate::delivery::Actor,
    /// The machine whose project a code-host target is read on.
    pub device: String,
    /// The project a code-host target is read in; none for a mission or
    /// node target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub target: WaitTarget,
    pub created_at_unix_ms: u64,
    pub until_unix_ms: u64,
    /// A branch head read with commits the default branch lacked: its own
    /// work, which makes the wait met once the default branch holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_ahead: Option<String>,
    /// How it ended, with the store's count then, which is its notice's
    /// revision; none while it is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
}

/// How and when an agent's wait ended.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Ended {
    #[serde(flatten)]
    pub outcome: WaitOutcome,
    pub at_unix_ms: u64,
    pub seq: u64,
}

impl AgentWait {
    pub fn open(&self) -> bool {
        self.ended.is_none()
    }
}

/// The line under a sleeping agent's row: what it waits on and until when
/// (D-15, B18). A pull request or CI target carries its code host's
/// reference, so the row writes `#12` or `!12` as the host does.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WaitLine {
    pub target: WaitTarget,
    pub until_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<crate::code_host::ChangeRequestRef>,
}

/// Every agent's wait, open and ended-unseen.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WaitStore {
    /// The last id given.
    #[serde(default)]
    pub next: u64,
    /// The last end counted, each end's notice revision.
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub waits: Vec<AgentWait>,
}

impl WaitStore {
    /// Whether no wait was ever made: a store whose waits all ended and went
    /// is still written, so its ids and notice revisions never start again
    /// at one, which a seen record of an earlier `wait-1` would hide.
    pub fn is_empty(&self) -> bool {
        self.waits.is_empty() && self.next == 0 && self.seq == 0
    }

    /// The open waits of every agent.
    pub fn open(&self) -> impl Iterator<Item = &AgentWait> {
        self.waits.iter().filter(|wait| wait.open())
    }

    /// The open wait `agent_id` already holds on `target`, which asking
    /// again answers (B14).
    pub fn same(&self, agent_id: &str, target: &WaitTarget) -> Option<&AgentWait> {
        self.open()
            .find(|wait| wait.agent_id == agent_id && wait.target == *target)
    }

    /// Gives the next id.
    pub fn allocate(&mut self) -> String {
        self.next += 1;
        format!("wait-{}", self.next)
    }

    /// Ends wait `id` with `outcome` at `now`; false when it is not open.
    pub fn end(&mut self, id: &str, outcome: WaitOutcome, now: u64) -> bool {
        let seq = self.seq + 1;
        let Some(wait) = self
            .waits
            .iter_mut()
            .find(|wait| wait.id == id && wait.open())
        else {
            return false;
        };
        wait.ended = Some(Ended {
            outcome,
            at_unix_ms: now,
            seq,
        });
        self.seq = seq;
        true
    }

    /// Keeps the ended waits within `MACHINE_WAIT_LIMIT`, the oldest end
    /// going first: an ended wait whose notice was never seen is finished
    /// work, and an open one is never dropped (engineering #15). Returns the
    /// ids dropped.
    pub fn bound_ended(&mut self) -> Vec<String> {
        let ended = self.waits.iter().filter(|wait| !wait.open()).count();
        let Some(over) = ended.checked_sub(MACHINE_WAIT_LIMIT) else {
            return Vec::new();
        };
        let mut ends: Vec<(u64, String)> = self
            .waits
            .iter()
            .filter_map(|wait| wait.ended.as_ref().map(|end| (end.seq, wait.id.clone())))
            .collect();
        ends.sort_unstable();
        let dropped: Vec<String> = ends.into_iter().take(over).map(|(_, id)| id).collect();
        self.waits.retain(|wait| !dropped.contains(&wait.id));
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(json: serde_json::Value) -> WaitInput {
        serde_json::from_value(json).expect("input")
    }

    #[test]
    fn an_input_names_one_target_and_a_deadline_within_its_bounds() {
        use serde_json::json;
        assert_eq!(input(json!({"pr": 12})).target(), Ok(WaitTarget::Pr(12)));
        assert_eq!(
            input(json!({"pr": 12, "ci": true})).target(),
            Ok(WaitTarget::Ci(12))
        );
        assert_eq!(
            input(json!({"mission": "mission-9", "node": "api"})).target(),
            Ok(WaitTarget::Node {
                mission: "mission-9".into(),
                node: "api".into()
            })
        );
        for refused in [
            json!({}),
            json!({"pr": 1, "branch": "x"}),
            json!({"ci": true}),
            json!({"node": "api"}),
            json!({"branch": "x", "ci": true}),
            json!({"pr": 0}),
            json!({"branch": "has space"}),
        ] {
            assert_eq!(
                input(refused.clone()).target(),
                Err(InputRefusal::Target),
                "{refused}"
            );
        }
        assert!(serde_json::from_value::<WaitInput>(json!({"issue": 3})).is_err());
        assert_eq!(input(json!({"pr": 1})).minutes(), Ok(DEFAULT_WAIT_MINUTES));
        assert_eq!(input(json!({"pr": 1, "for": "3d"})).minutes(), Ok(3 * 1440));
        assert_eq!(input(json!({"pr": 1, "for": "60m"})).minutes(), Ok(60));
        for span in ["59m", "31d", "721h", "2w", "", "d"] {
            assert_eq!(
                input(json!({"pr": 1, "for": span})).minutes(),
                Err(InputRefusal::Deadline),
                "{span}"
            );
        }
        assert_eq!(span_minutes("30d"), Some(MAX_WAIT_MINUTES));
    }

    #[test]
    fn a_stored_target_reads_the_old_outside_shape() {
        // A wait node's target is stored as an outside node's was.
        let pr: WaitTarget = serde_json::from_str(r#"{"pr":1232}"#).unwrap();
        let branch: WaitTarget = serde_json::from_str(r#"{"branch":"feat/x"}"#).unwrap();
        assert_eq!(
            (pr, branch),
            (WaitTarget::Pr(1232), WaitTarget::Branch("feat/x".into()))
        );
        let node = WaitTarget::Node {
            mission: "mission-1".into(),
            node: "a".into(),
        };
        let text = serde_json::to_string(&node).unwrap();
        assert_eq!(text, r#"{"node":{"mission":"mission-1","node":"a"}}"#);
    }

    #[test]
    fn ended_waits_past_the_limit_drop_oldest_first_and_open_ones_stay() {
        let actor = crate::delivery::Actor {
            pane_id: "w1:p1".into(),
            name: "a".into(),
            kind: "claude".into(),
            device_id: "here".into(),
            session: None,
        };
        let mut store = WaitStore::default();
        for index in 0..=MACHINE_WAIT_LIMIT + 1 {
            let id = store.allocate();
            store.waits.push(AgentWait {
                id: id.clone(),
                agent_id: format!("agent-{index}"),
                actor: actor.clone(),
                device: "here".into(),
                project: None,
                target: WaitTarget::Mission("mission-1".into()),
                created_at_unix_ms: 0,
                until_unix_ms: 1,
                branch_ahead: None,
                ended: None,
            });
            if index > 0 {
                assert!(store.end(&id, WaitOutcome::Expired, 1));
            }
        }
        let dropped = store.bound_ended();
        assert_eq!(dropped, vec!["wait-2".to_owned()]);
        assert!(
            store
                .waits
                .iter()
                .any(|wait| wait.id == "wait-1" && wait.open())
        );
        assert_eq!(
            store.waits.iter().filter(|wait| !wait.open()).count(),
            MACHINE_WAIT_LIMIT
        );
    }
}
