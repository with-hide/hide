//! The runtime half of a Conductor's mission (PRD software-factory-conductor):
//! the `hide mission` commands, each applied under the lock as one change;
//! the map's published view; the cap and the approval a mission's spawn is
//! admitted against; the wake a mission's pull request change sends its
//! Conductor; and the record going with its Conductor's archived session.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Runtime;
use super::mission_criteria::Caller as MissionCaller;
use super::mission_landing::{
    NodeLanding, landing_order, node_landing, reads_branch, stack_settled, stack_waiting,
};
use crate::delivery::Actor;
use crate::mission_criteria::{CheckKind, Criterion, CriterionKind, CriterionState};
use crate::mission_events::History;
use crate::mission_store::{Restored, Visiting};
use crate::missions::{
    AnswerFailure, AttentionTo, Blocked, CI_FAILED_NAMED, Checks, CiNews, CiResult, Conductor,
    ConductorSettings, Delivery, External, Mission, MissionBook, MissionCommand, MissionEnd,
    MissionNode, MissionScope, MissionStatus, NodeKind, NodeStatus, PENDING_WAKE_LIMIT,
    PendingWake, PrSeen, Question, Verdict, WaitingOn, WakeEvent, WarningKind, Worker,
};
use crate::model::{DetailItem, PrState, PullRequestBadge, ReviewDecision};
use crate::workspace_control::{Caller, Context, Query};

/// The role Hide wakes a mission's Conductor as (D-39). Like the memory
/// role it is constructed only by the core; unlike it, it sends letters.
pub(crate) const CONDUCTOR_ROLE: &str = "conductor";
/// Wakes one sending takes at once; the rest wait for the next.
const WAKE_QUEUE_LIMIT: usize = 256;
const WAKE_CALL: Duration = Duration::from_secs(10);

/// The missions the shell draws (D-14), each with what its map needs and
/// nothing the shell would have to work out.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MissionsSnapshot {
    pub items: Vec<MissionView>,
    /// The record could not be read at start; the map says so instead of
    /// drawing a number (B21).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MissionView {
    pub id: String,
    pub title: String,
    pub workspace_id: String,
    pub project: String,
    pub checkout_path: String,
    pub device_id: String,
    pub status: MissionStatus,
    pub scope: MissionScope,
    pub cap: u8,
    /// Made before node criteria: its nodes show their sentence only and
    /// nothing is gated (PRD mission-map-git-criteria B23).
    pub exempt: bool,
    /// How strongly the Conductor has the nodes checked; none for a mission
    /// from before verification strength.
    pub strength: Option<crate::missions::Strength>,
    /// What a mission from before verification strength recorded instead,
    /// as it was written; absent for every other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkers: Option<serde_json::Value>,
    pub conductor: ConductorView,
    pub nodes: Vec<NodeView>,
    pub merge_order: Vec<String>,
    /// The nodes left for the operator to merge or join, in the order to
    /// take them (`mission_landing::landing_order`, B18, B25).
    pub landing_order: Vec<String>,
    /// The stacked pull requests still waiting on the one below
    /// (`mission_landing::stack_waiting`).
    pub stack_waiting: u32,
    pub check: Option<String>,
    pub github: Option<String>,
    pub decision: Option<String>,
    /// What the Conductor said beside its decision (PRD
    /// mission-plan-visibility B25); a decision from before it has key 0
    /// and nothing beside its line.
    pub question: Option<Question>,
    /// The Views linked to that decision that are still open, for the
    /// operator to look at before answering (`mission_views`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub decision_views: Vec<super::mission_views::DecisionView>,
    /// The operator's answer from the banner to that decision (B26, B28).
    pub answer: Option<AnswerView>,
    /// Why the last answer did not reach the Conductor (B28).
    pub answer_failed: Option<crate::missions::AnswerFailure>,
    /// The operator asked the Conductor to end the mission and it has not
    /// closed it yet (`미션 종료`).
    pub end_asked: bool,
    /// The letters to the Conductor the doorbell holds for something only
    /// the operator can lift (`delivery::held::letters_held`), while the mission
    /// runs: its map's Conductor row and its 내 차례 say how many and what
    /// to do, and the line goes when they are delivered
    /// (with-hide/hide-private#678).
    pub conductor_letters: Option<crate::delivery::held::HeldLetters>,
    pub delay: Option<String>,
    /// When the delay ends by itself (`Mission::delay_until_unix_ms`).
    pub delay_until_unix_ms: Option<u64>,
    pub report: Option<String>,
    /// The mission's one card for the raises its Conductor has not settled
    /// (`mission_health::safety_net`): none while nothing is held, with no
    /// Conductor at once, and with one once the oldest held raise stood 20
    /// minutes. The operator's turn for raised nodes is this and nothing
    /// per node (hide-private docs/missions.md, the principle of
    /// 2026-11-11).
    pub safety_net: Option<SafetyNetView>,
    /// The lines the Conductor skipped on its own, with no decision of the
    /// operator's, oldest first: what the map's `Conductor가 정한 것` and the
    /// close report list.
    pub skipped_alone: Vec<SkippedAloneView>,
    pub created_at_unix_ms: u64,
    pub started_at_unix_ms: Option<u64>,
    pub closed_at_unix_ms: Option<u64>,
    /// Workers counted against the cap now: started and not ended, or still
    /// being started (D-29).
    pub running: u32,
    /// The device the mission's project is on does not answer, so what the
    /// map shows is the last value (B20).
    pub stale: bool,
    /// The last GitHub read of the project failed; its marks are dimmed with
    /// the time of the read before (B16).
    pub github_stale: bool,
    pub github_read_at_unix_ms: Option<u64>,
}

/// Where the operator's answer is (B26, B28): accepted and not yet tried,
/// waiting for the Conductor's turn, or typed without the engine confirming it
/// arrived, kept until the operator sends it again. A typed answer takes
/// its decision away, so it has no state of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    Sending,
    Waiting,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AnswerView {
    pub key: u64,
    pub text: String,
    pub option: Option<u8>,
    pub state: AnswerState,
    /// What the answer waits for while it is `waiting`, so the banner says it
    /// and offers `지금 보내기` where pressing could help.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<AnswerHold>,
}

/// Why a waiting answer is not typed yet, in the words the operator can act
/// on: the Conductor works, the operator's own typing is in its input, the
/// quiet period after either, or a menu of its own waits; every other wait ends on its own and has no word.
/// `Forced` is the press of `지금 보내기`, until the doorbell types it or
/// finds text in the input and gives the earlier reason back.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerHold {
    Working,
    Typing,
    /// The quiet period after the Conductor's last work or input: the input is
    /// empty, but a press may not be wanted yet, so it says what happened.
    Recent,
    Approval,
    Other,
    Forced,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConductorView {
    pub pane_id: String,
    pub name: String,
    pub agent_kind: String,
    pub session: Option<crate::links::ledger::SessionIdentity>,
    /// The Conductor's pane runs its session now; absent shows `Conductor
    /// 없음 · 이어받기` (B43).
    pub present: bool,
    /// Its agent takes an answer typed from the map's banner: the doorbell
    /// types only into the agents it rings (PRD mission-plan-visibility
    /// B26), and the banner sends any other to the conversation.
    pub takes_answers: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NodeView {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    pub depends_on: Vec<String>,
    pub status: NodeStatus,
    pub blocked: Option<Blocked>,
    pub delivery: Option<Delivery>,
    pub worker: Option<Worker>,
    pub agent_id: Option<String>,
    /// The worker's life as `hide agent show` gives it (`coordination::life`):
    /// a sleeping worker's card reads asleep, not working. None with no
    /// registration.
    pub agent_runtime: Option<crate::delivery::answer::Runtime>,
    /// What the node's current worker was proven launched with; none until
    /// it is proven, and none for a launch proven for an earlier worker.
    pub launched: Option<crate::missions::NodeLaunch>,
    /// The worker's pane, which the node's status mark and `터미널로 이동`
    /// follow, on `machine`.
    pub pane_id: Option<String>,
    pub machine: Option<String>,
    /// The current pull request, the last of `prs`, with its state and
    /// address.
    pub pr: Option<u32>,
    /// Every pull request the node holds, the current one last (PRD
    /// mission-node-simple D-10): the card draws one PR mark each.
    pub prs: Vec<NodePrView>,
    pub base_pr: Option<u32>,
    pub branch: Option<String>,
    pub pr_state: Option<PrState>,
    pub pr_url: Option<String>,
    /// The node's four steps (D-42, D-48): work, PR or branch, review, merge
    /// or main; none for a node with no pull request or branch (D-53).
    pub stage: Option<u8>,
    pub attempts: u8,
    pub line: Option<String>,
    pub change: Option<String>,
    pub review: Option<String>,
    /// The node's finish lines as they stand now: a check command passed on
    /// an older commit shows open (B7, B32).
    pub criteria: Vec<Criterion>,
    /// The steps its Conductor planned for the node, the card's and the
    /// panel's start until the worker's own plan (`plan`) exists.
    pub steps: Vec<String>,
    /// The size of the brief its worker was started on; none until the
    /// Conductor bound a worker. The text is read on request
    /// (`runtime/mission_brief.rs`).
    pub brief_bytes: Option<u32>,
    /// Lines that pass or were skipped, of `criteria`, which is what the
    /// gate counts, and how many were skipped; B28's `기준 n/m ⤳k` shows
    /// `criteria_met - criteria_skipped` as `n`.
    pub criteria_met: u32,
    pub criteria_skipped: u32,
    pub external: Option<External>,
    /// An outside node Hide could not read: it stays waiting, and the
    /// reason is in the diagnostic log (B20 `확인 못 함`).
    pub unread: bool,
    /// How often a later node's defect opened it again (B21 `↻n`).
    pub reopened: u8,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: Option<u64>,
    pub commit: Option<String>,
    /// Where its work is on its way to main and what is left before it
    /// (`mission_landing`): the card's `머지 남음` or `합치기 남음` and its
    /// `→ main <sha7>` read it; `commit` is what the Conductor recorded.
    pub landing: Option<NodeLanding>,
    pub verdict: Option<Verdict>,
    pub checks: Option<Checks>,
    /// The worker's own plan (PRD hide-plan D-17, D-18): the value its pane's
    /// chip and map draw, for the card's fourth line and the node's detail.
    pub plan: Option<crate::plans::PlanView>,
    /// What waits on someone to look, and who (PRD mission-node-simple D-02).
    pub attention: Option<AttentionView>,
    /// What Hide itself did to the node last (D-07, D-15).
    pub hide: Option<crate::missions::HideMark>,
    /// Its review's failures in a row (with-hide/hide-private#552).
    pub review_fails: u32,
    /// When Hide last read a new head on one of its pull requests.
    pub pushed_at_unix_ms: Option<u64>,
    /// A push is asked of it (`MissionNode::asks_push`), the node whose
    /// quiet can warn: `hide mission show` says `push 없음` before its first.
    pub asks_push: bool,
    /// The warnings it carries, its Conductor's alone: the card's `⚠`.
    pub warnings: Vec<crate::missions::NodeWarning>,
    /// Why and until when its Conductor holds it (`MissionNode::hold`).
    pub hold: Option<crate::missions::NodeHold>,
    /// Who or what it waits on now (`waiting_on`), which `hide mission show`
    /// prints; none for a node that waits on nothing.
    pub waiting_on: Option<WaitingOn>,
    /// Whether a node in progress is being worked now (`node_work`); none
    /// for a node not in progress, an outside one, or one whose worker's
    /// row Hide cannot see.
    pub work: Option<NodeWork>,
    /// The place the operator looks before the node is done, and where it
    /// stands (PRD mission-human-gate D-09); absent for a node without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<super::mission_gates::GateView>,
}

/// Whether a node in progress is worked now (with-hide/hide-private#680):
/// its worker works, its worker rests and the node waits on someone
/// (`waiting_on`, said beside it), or no worker stands for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeWork {
    Working,
    Waiting { on: WaitingOn },
    NoWorker,
}

/// What `view`, a node of a running mission, is doing now: from its
/// worker's registration (none or ended is no worker, asleep rests) and,
/// for a running one, its agent row's activity (`working` works, `stopped`
/// rests); a row Hide cannot see, or whose activity it does not know, says
/// nothing rather than a guess.
fn node_work(mission: &Mission, view: &NodeView, row_activity: Option<&str>) -> Option<NodeWork> {
    use crate::delivery::answer::Runtime as Life;
    if !mission.status.open()
        || view.status != NodeStatus::Working
        || view.kind == NodeKind::External
    {
        return None;
    }
    let waiting = || view.waiting_on.clone().map(|on| NodeWork::Waiting { on });
    match view.agent_runtime {
        None | Some(Life::Ended | Life::Closed) => Some(NodeWork::NoWorker),
        Some(Life::Asleep) => waiting(),
        Some(Life::Running) => match row_activity? {
            "working" => Some(NodeWork::Working),
            "stopped" => waiting(),
            _ => None,
        },
    }
}

/// What `view`, a node of `mission`, waits on (`WaitingOn`): its raise
/// first, then the decision that stopped it, then by its status what it has
/// not met. A planned node waits on the nodes before it, a working one on
/// its first line not met (a failed line is its worker's), then its pull
/// request's merge, and a done one on the merge left before main.
fn waiting_on(mission: &Mission, view: &NodeView) -> Option<WaitingOn> {
    if let Some(attention) = &view.attention {
        return Some(WaitingOn::Attention {
            to: attention.to,
            reason: attention.reason,
        });
    }
    if mission.deciding_on(&view.key) {
        return Some(WaitingOn::Decision);
    }
    match view.status {
        NodeStatus::Planned => {
            if view.blocked.is_some() {
                return Some(WaitingOn::Blocked);
            }
            let keys: Vec<String> = view
                .depends_on
                .iter()
                .filter(|key| {
                    mission
                        .nodes
                        .iter()
                        .find(|other| other.key == **key)
                        .is_some_and(|other| other.status != NodeStatus::Done)
                })
                .cloned()
                .collect();
            Some(if keys.is_empty() {
                WaitingOn::Start
            } else {
                WaitingOn::Nodes { keys }
            })
        }
        NodeStatus::Working => {
            if view
                .criteria
                .iter()
                .any(|line| line.state == CriterionState::Fail)
            {
                return Some(WaitingOn::Worker);
            }
            // A gate waits after every other line, on the operator.
            let open = view
                .criteria
                .iter()
                .filter(|line| line.kind != CriterionKind::Human)
                .find(|line| !matches!(line.state, CriterionState::Pass | CriterionState::Skipped))
                .or_else(|| {
                    view.criteria.iter().find(|line| {
                        line.kind == CriterionKind::Human && line.state != CriterionState::Pass
                    })
                });
            Some(match open.map(|line| line.kind) {
                Some(CriterionKind::Human) => WaitingOn::Decision,
                Some(CriterionKind::Review) => WaitingOn::Review,
                Some(CriterionKind::Verify) => WaitingOn::Verify,
                Some(CriterionKind::Ci) => WaitingOn::Ci,
                Some(_) => WaitingOn::Worker,
                None if view.pr_state.is_some_and(|state| state != PrState::Merged) => {
                    WaitingOn::Merge
                }
                None => WaitingOn::Conductor,
            })
        }
        NodeStatus::Done if matches!(view.landing, Some(NodeLanding::Left { .. })) => {
            Some(WaitingOn::Merge)
        }
        NodeStatus::Done | NodeStatus::Dropped => None,
    }
}

/// Letters `hide mission show <node>` lists of the node's worker, and
/// apart from them, of its checkers to its Conductor: each list keeps its
/// own five, so a busy worker never pushes its reviews off (#668).
const NODE_LETTER_LIMIT: usize = 5;
/// Characters of a letter's first line it keeps.
const NODE_LETTER_LINE: usize = 160;

/// One letter of a node's worker or checker, as `hide mission show <node>`
/// lists it: its id, which `hide request show` reads, who wrote to whom,
/// its kind, and its first line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NodeLetter {
    pub id: String,
    pub at_unix_ms: u64,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub line: String,
}

/// The newest `NODE_LETTER_LIMIT` letters of `ledger` that `pick` takes,
/// newest first, each with its first line cut to `NODE_LETTER_LINE`.
fn newest_letters(
    ledger: &crate::delivery::ledger::Ledger,
    pick: impl Fn(&crate::delivery::ledger::Letter) -> bool,
) -> Vec<NodeLetter> {
    let mut letters: Vec<&crate::delivery::ledger::Letter> = ledger
        .letters
        .iter()
        .filter(|letter| pick(letter))
        .collect();
    letters.sort_by_key(|letter| std::cmp::Reverse(letter.created_at_unix_ms));
    letters
        .into_iter()
        .take(NODE_LETTER_LIMIT)
        .map(|letter| NodeLetter {
            id: letter.id.clone(),
            at_unix_ms: letter.created_at_unix_ms,
            from: letter.sender.name.clone(),
            to: letter.recipient.name.clone(),
            kind: letter.kind.clone(),
            line: letter
                .body
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(NODE_LETTER_LINE)
                .collect(),
        })
        .collect()
}

/// One of a node's pull requests as its card draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NodePrView {
    pub number: u32,
    pub state: Option<PrState>,
    pub url: Option<String>,
}

/// The mission's safety net as its card draws it (`MissionView::safety_net`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SafetyNetView {
    /// A Conductor holds the mission: the card leads to its terminal;
    /// without one it offers 지휘 다시 열기.
    pub conductor_present: bool,
    /// What the card counts its minutes from: the oldest held raise, or the
    /// Conductor's arrival when that is later.
    pub since_unix_ms: u64,
    /// The held nodes, oldest raise first.
    pub nodes: Vec<HeldNodeView>,
}

/// One node the safety net holds: its raise's reason and since when it
/// stands.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HeldNodeView {
    pub key: String,
    pub title: String,
    pub reason: crate::missions::AttentionReason,
    pub pr: Option<u32>,
    pub since_unix_ms: u64,
}

/// A line the Conductor skipped on its own (no decision of the
/// operator's): the node, the line, why, who and when.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkippedAloneView {
    pub node: String,
    pub title: String,
    pub line: String,
    pub kind: CriterionKind,
    /// The line's own words, for a line the plan wrote (`own`, `defect`);
    /// a check, a CI, a review or a verification is named by its kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub reason: String,
    pub by: String,
    pub at_unix_ms: u64,
}

/// The lines of `mission` its Conductor skipped with no decision of the
/// operator's, oldest first.
pub(crate) fn skipped_alone(mission: &Mission) -> Vec<SkippedAloneView> {
    let mut out: Vec<SkippedAloneView> = mission
        .nodes
        .iter()
        .flat_map(|node| {
            node.criteria.iter().filter_map(move |line| {
                let skip = line.skip.as_ref().filter(|skip| skip.decision.is_none())?;
                let by = line
                    .by
                    .as_ref()
                    .filter(|by| by.role == crate::mission_criteria::Role::Conductor)?;
                (line.state == CriterionState::Skipped).then(|| SkippedAloneView {
                    node: node.key.clone(),
                    title: node.title.clone(),
                    line: line.key.clone(),
                    kind: line.kind,
                    text: line.text.clone(),
                    reason: skip.reason.clone(),
                    by: by.name.clone(),
                    at_unix_ms: by.at_unix_ms,
                })
            })
        })
        .collect();
    out.sort_by_key(|skip| skip.at_unix_ms);
    out
}

/// The `!` a node's card carries and the line its hover says.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AttentionView {
    pub reason: crate::missions::AttentionReason,
    pub pr: Option<u32>,
    /// The operator only for a reason only they settle
    /// (`AttentionReason::operator_first`); every other raise is the
    /// Conductor's, and the safety net is the mission's.
    pub to: AttentionTo,
    /// When it was raised, which names it in the operator's answer.
    pub since_unix_ms: u64,
    /// Waiting on the operator, a Needs You row carries it: its Conductor's
    /// or its worker's. Without one the mission itself is the Needs You
    /// item (PRD mission-node-simple D-19, B14).
    pub on_row: bool,
    /// The mission itself is a Needs You item for it: it waits on the
    /// operator with no row to carry it, in a mission that still asks and
    /// on a node not dropped. The one rule the count and the list read
    /// (`publish_mission_asks`), never worked out again by the screen.
    pub item: bool,
    /// The operator's choices for it, in order (`AttentionReason::choices`).
    pub choices: Vec<crate::missions::OperatorAction>,
    /// For a refused close, how many finished nodes have a line empty now,
    /// read from the book the way the close reads it, so the row can say
    /// how much is missing; 0 for every other reason.
    pub unmet_nodes: u32,
}

/// The spawn an admission judges: the machine its child starts on (none
/// for the caller's own), its agent, the profile it names by name, the
/// project it starts in, the model and effort it starts with, and whether
/// it retries a spawn that finished, which answers with the child it
/// started and is judged by what it did (engineering 11).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpawnWorker<'a> {
    pub(crate) machine: Option<&'a str>,
    pub(crate) kind: &'a str,
    pub(crate) profile: Option<&'a str>,
    pub(crate) repo: &'a str,
    pub(crate) launch: &'a hide_agent_adapter::LaunchChoice,
    pub(crate) finished: bool,
}

/// A spawn admitted as one of a mission's workers: its cap is checked in
/// the ledger, with the workers it already has.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionAdmission {
    pub mission: String,
    pub cap: u8,
    /// The registrations closed now, which Reopen closed holds and which
    /// hold no place under the cap (`closed_registrations.rs`).
    #[serde(default)]
    pub closed: Vec<String>,
}

/// One wake for a Conductor, made under the lock and sent off it.
struct Wake {
    mission: String,
    seq: u64,
    target: super::delivery::Observation,
    intent: String,
    body: String,
    kind: &'static str,
}

#[derive(Default)]
pub(crate) struct MissionRuntime {
    pub(crate) book: MissionBook,
    pub(crate) store: Option<crate::mission_store::MissionStore>,
    /// Why the stored record was not used; commands are refused while it
    /// stands so the file is not replaced by an empty book.
    pub(crate) unreadable: Option<String>,
    /// A sending thread is running; it is the only one.
    waking: bool,
    /// A wake was recorded or a Conductor arrived while it ran, so it looks
    /// again before it ends.
    rewake: bool,
    /// Counts the book's changes; every change is saved, so the save counts
    /// it.
    pub(crate) book_edit: u64,
    /// Counts the GitHub answers and the pull request lists replaced since.
    pub(super) github_edit: u64,
    /// What the published missions were made from, so a snapshot read whose
    /// inputs did not move makes nothing (docs/PERFORMANCE_TESTING.md).
    views: ViewsMadeFrom,
    /// Each mission's events (PRD mission-plan-visibility D-22), saved by
    /// their own store; only the mission the page's detail shows sends them.
    pub(crate) events: HashMap<String, History>,
    pub(crate) event_store: Option<crate::mission_events::MissionEventStore>,
    /// Counts the events' changes, for the published section.
    pub(crate) events_edit: u64,
    /// The answers the doorbell tried and held, by mission: shown as
    /// waiting for the Conductor's turn (B28). Memory only; a restart shows
    /// an answer as sending until the next pass holds it again.
    pub(crate) answers_held: HashMap<String, AnswerHold>,
    /// The answers the operator pressed `지금 보내기` for, by mission and the
    /// question's key: the doorbell types them past the Conductor's work, the
    /// quiet period and a draft it reads empty. Memory only, and only the
    /// missions whose answer still waits are kept (`mission_answers_due`).
    pub(crate) answers_forced: HashMap<String, u64>,
    /// The answers whose `agent.prompt` the doorbell has asked and not yet
    /// heard back from, by mission: shown as sending. Memory only; the book
    /// already holds them as unconfirmed, so a stop in between never types
    /// one again.
    pub(crate) answers_typing: std::collections::HashSet<String>,
    /// When each mission's Conductor was first seen missing while its
    /// answer waited; only those missions are kept.
    pub(crate) answer_absent_since: HashMap<String, u64>,
    /// The mission and events edit the published events were made from.
    pub(crate) events_made: (Option<String>, u64),
    /// The mission the page's mission screen shows in its detail
    /// (`mission_shown`, PRD mission-screen D-16): page navigation the core
    /// holds only to send that one mission's events, never saved (D-10).
    pub(crate) shown: Option<String>,
    /// The outside nodes' reads (B19, B20).
    pub(super) outside: super::mission_outside::Outside,
    /// The nodes finished as the book last read, by mission and node, so a
    /// save tells a node that has just finished (PRD mission-worker-sleep
    /// D-17). Set from the book read at start, so a restart sleeps nothing
    /// for a finish it already saw.
    pub(super) finished: std::collections::HashSet<(String, String)>,
    /// The agents a finish asks to put to sleep, in the order they finished
    /// (D-19). Memory only: a restart leaves them to the hours rule.
    pub(super) sleep_marks: Vec<super::agent_sleep::missions::SleepMark>,
    /// What the last health evaluation read (PRD mission-node-simple D-08).
    pub(super) health_made: super::mission_health::HealthMadeFrom,
    /// Since when each stall standing now has stood, by mission, node and
    /// cause, as the minute decision first saw it (`stall_missions`). Memory
    /// only: a restart or an update starts every clock again, so nothing that
    /// rested before it is raised at once; an entry goes once its stall
    /// stops standing, so it holds at most one per node standing.
    pub(super) stall_clock: std::collections::HashMap<(String, String, String), u64>,
    /// Since when this daemon has seen each mission's Conductor present
    /// (`Runtime::note_conductors_seen`), which the safety net counts a
    /// Conductor's 20 minutes from when it came after the raise. Memory only,
    /// at most one entry per mission of the book: a restart, a take or a
    /// resume starts it again.
    pub(super) conductor_seen: std::collections::HashMap<String, u64>,
    /// The merged worktrees a done node's evaluation retires (Settings ›
    /// Mission's cleanup switch).
    pub(super) cleanup: super::mission_cleanup::MissionCleanup,
    /// The projects, by device and path, a worker's letter or a Conductor's
    /// node change asked to read again (D-09, B23), kept until that read is
    /// asked. Memory only: a restart reads every project anew.
    pub(super) reads_due: std::collections::BTreeSet<(String, String)>,
    /// Counts the writes to the pane launch records (`ui_state.pane_launches`),
    /// every one of which goes through `runtime/agent_choice.rs`.
    pub(super) launches_edit: u64,
    /// What the last launch proof read (`record_node_launches`), so an agent
    /// pass in which none of it moved reads nothing.
    launch_proof_made: Option<LaunchProofMade>,
    /// The phone's presses that asked a decision's View to open again, and
    /// what became of each (`mission_views.rs`).
    pub(super) reopens: super::mission_views::Reopens,
}

/// The inputs of a launch proof: the ledger (held weakly, so it is never
/// kept alive by this and its address is never another ledger's), the
/// book's edit and the launch records' edit.
struct LaunchProofMade {
    ledger: std::sync::Weak<crate::delivery::ledger::Ledger>,
    book_edit: u64,
    launches_edit: u64,
}

#[derive(Default)]
struct ViewsMadeFrom {
    book_edit: u64,
    /// The plans' edit, since a node draws its worker's plan.
    plans_edit: u64,
    github_edit: u64,
    /// Held weakly, so the ledger it names is never kept alive by it and its
    /// address is never another ledger's.
    ledger: Option<std::sync::Weak<crate::delivery::ledger::Ledger>>,
    /// Per mission in book order: whether its Conductor is there, for an
    /// open one, and whether its device answers.
    present: Vec<bool>,
    stale: Vec<bool>,
    /// Per mission in book order: whether its Conductor's session cannot be
    /// read, which an answer waiting on it shows.
    unreadable: Vec<bool>,
    /// The earliest time a worker still being made stops counting against
    /// its mission's cap (`MISSION_RESERVATION_HOLD_MS`).
    until: u64,
    /// The missions' workers Hide holds asleep, which their cards read.
    sleeping: crate::coordination::RegistrationFacts,
    /// The rows carrying a core question, which decide whether a node's
    /// attention is on a row (`on_row`).
    laid: Vec<String>,
    /// While a mission runs: a digest of every agent row's activity, which
    /// a node's `work` reads, and of the doorbell's holds, which its
    /// Conductor's held letters read; zero with none running.
    rows_and_holds: u64,
    /// While a decision links Views: the View layouts' change count, since
    /// its card lists the ones still open and whether each was seen; zero
    /// with none linked.
    views_generation: u64,
    made: bool,
}

fn seen_of(pull: &crate::model::PullRequestSnapshot) -> PrSeen {
    PrSeen {
        number: pull.number,
        head: pull.head_oid.clone(),
        merged: pull.badge == PullRequestBadge::Merged,
        changes_requested: pull.review == Some(ReviewDecision::ChangesRequested),
        closed: pull.badge == PullRequestBadge::Closed,
        ci_told: Vec::new(),
    }
}

/// What an open `pull`'s CI ended as on its head, by the rule its ci line
/// reads too (`PullRequestSnapshot::ci_end`); none while anything on the
/// head still runs, while it has no checks, or while they cannot be read.
fn ci_of(pull: &crate::model::PullRequestSnapshot) -> Option<CiResult> {
    pull.head_oid.as_ref()?;
    if pull.badge.is_settled() {
        return None;
    }
    match pull.ci_end() {
        crate::model::CiEnd::Ended(result) => Some(result),
        _ => None,
    }
}

/// Keeps the CI results `before` told beside those `seen` holds while
/// `seen` is of the same head.
fn keep_told(seen: &mut PrSeen, before: &PrSeen) {
    if seen.head != before.head {
        return;
    }
    for result in &before.ci_told {
        if !seen.ci_told.contains(result) {
            seen.ci_told.push(*result);
        }
    }
}

/// The CI letter of `pull` ending as `result`: a failure names the checks
/// that failed, up to `CI_FAILED_NAMED`, and counts the rest.
fn ci_news(pull: &crate::model::PullRequestSnapshot, result: CiResult) -> CiNews {
    let mut names: Vec<String> = Vec::new();
    if result == CiResult::Failed {
        for name in pull
            .work
            .failed_checks
            .iter()
            .filter_map(|check| check.name.as_deref())
        {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_owned());
            }
        }
    }
    let more = names.len().saturating_sub(CI_FAILED_NAMED);
    names.truncate(CI_FAILED_NAMED);
    CiNews {
        result,
        failed: names,
        more,
    }
}

/// Notes on `node`'s record of pull request `number` that its CI `result`
/// was told on the head it holds, once its letter is recorded.
fn tell_ci(mission: &mut Mission, node: &str, number: u32, result: CiResult) {
    let Some(seen) = mission
        .nodes
        .iter_mut()
        .find(|held| held.key == node)
        .and_then(|node| node.pr_seen.iter_mut().find(|seen| seen.number == number))
    else {
        return;
    };
    if !seen.ci_told.contains(&result) {
        seen.ci_told.push(result);
    }
}

/// Records what `project`'s list says of each of `mission`'s pull requests
/// it holds; whether anything moved.
fn refresh_seen(
    mission: &mut Mission,
    project: &crate::model::GithubProjectSnapshot,
    now: u64,
) -> bool {
    let mut moved = false;
    for node in &mut mission.nodes {
        for number in node.prs.clone() {
            let Some(pull) = project
                .pull_requests
                .iter()
                .find(|pull| pull.number == number)
            else {
                continue;
            };
            moved |= note_seen(node, seen_of(pull), now);
        }
    }
    moved
}

/// The letter one recorded wake sends.
pub(super) fn wake_body(mission: &Mission, wake: &PendingWake) -> String {
    let pr = wake
        .pr
        .map_or_else(|| "The PR".to_owned(), |number| format!("PR #{number}"));
    let node = mission.nodes.iter().find(|node| node.key == wake.node);
    let outside = || match node.and_then(|node| node.external.as_ref()) {
        Some(External::Branch(branch)) => format!("Branch {branch}"),
        _ => pr.clone(),
    };
    let news = |what: String, act: &str| {
        format!(
            "{what} (mission {} \"{}\", node {}).\nRead the mission with `hide mission show {}` and act on it: {act}.",
            mission.id, mission.title, wake.node, mission.id
        )
    };
    match wake.event {
        WakeEvent::Merged => news(format!("{pr} was merged"), PR_ACT),
        WakeEvent::ChangesRequested => {
            news(format!("{pr} received a review asking for changes"), PR_ACT)
        }
        WakeEvent::NewHead => news(format!("{pr} has a new commit"), PR_ACT),
        WakeEvent::Ci => {
            let commit = wake
                .head
                .as_deref()
                .map_or("-", |head| head.get(..7).unwrap_or(head));
            let (what, act) = match &wake.ci {
                Some(CiNews {
                    result: CiResult::Passed,
                    ..
                }) => (
                    format!("{pr}: CI passed on commit {commit}"),
                    "go on with what waited on it",
                ),
                Some(CiNews {
                    result: CiResult::Cancelled,
                    ..
                }) => (
                    format!("{pr}: CI was cancelled on commit {commit} and no run replaced it"),
                    "run it again, or find out why it was cancelled",
                ),
                Some(CiNews {
                    result: CiResult::Failed,
                    failed,
                    more,
                }) => {
                    let mut names = failed.join(", ");
                    if *more > 0 {
                        names.push_str(&format!(" and {more} more"));
                    }
                    let names = if names.is_empty() {
                        names
                    } else {
                        format!(": {names}")
                    };
                    (
                        format!("{pr}: CI failed on commit {commit}{names}"),
                        "see that its worker fixes it, or run it again when the failure is not the change's",
                    )
                }
                None => (format!("{pr}: CI ended on commit {commit}"), PR_ACT),
            };
            news(what, act)
        }
        WakeEvent::OutsideDone => news(
            format!("{} is in main", outside()),
            "start the nodes that waited on it",
        ),
        WakeEvent::OutsideFailed => news(
            format!("{} was closed without a merge", outside()),
            "replan the nodes it blocks, or ask the operator",
        ),
        WakeEvent::Resumed => format!(
            "You conduct mission {} \"{}\" again: your pane had closed, and the operator brought your conversation back.\nRead `hide mission show {}` and `hide inbox`, check each worker's state, and go on from there.",
            mission.id, mission.title, mission.id
        ),
        WakeEvent::EndAsked => format!(
            "The operator asked from the map to end mission {} \"{}\".\nTreat it as the operator saying \"cancel\" and follow that item of `hide mission help`: ask about the open pull requests with `hide mission decide`, end the workers, then `hide mission close --as cancelled`.",
            mission.id, mission.title
        ),
        WakeEvent::MaterialAsked => format!(
            "The operator asks for something to look at before answering your decision on mission {} \"{}\": {}\nMake one page that starts with three lines (what changed, the 2-3 things to check, what good looks like), then ask again with `hide mission decide ... --show <page>`: Hide opens the page as a View and lists it on the decision.",
            mission.id,
            mission.title,
            mission.decision.as_deref().unwrap_or("-")
        ),
        WakeEvent::GatePassed => {
            let line = node.and_then(|node| crate::mission_criteria::gate(&node.criteria));
            format!(
                "The operator passed the gate on node {} of mission {} \"{}\" ({}): \"{}\".\nGo on with the node as its lines say, and start the nodes that waited on it.",
                wake.node,
                mission.id,
                mission.title,
                line.and_then(|line| line.via)
                    .map_or("-", crate::mission_criteria::GateVia::as_str),
                line.and_then(|line| line.evidence.as_deref())
                    .unwrap_or("-"),
            )
        }
        WakeEvent::GateFix => {
            let (defect, words) = node
                .and_then(super::mission_gates::operator_defect)
                .unwrap_or(("-", "-"));
            match &wake.to {
                Some(_) => format!(
                    "The operator looked at your node {} of mission {} \"{}\" and asked for a fix: \"{words}\".\nIt is line {defect} of your node: fix it, record it with `hide mission record {} {defect} --pass --evidence <what shows it>`, and report to your Conductor as before.",
                    wake.node, mission.id, mission.title, wake.node
                ),
                None => format!(
                    "The operator answered the gate on node {} of mission {} \"{}\" with a fix: \"{words}\".\nHide added it as line {defect}, set the node to work and sent the words to its worker; the gate asks again once its lines are met. Read `hide mission show {} --mission {}`, and brief a worker if the node has none.",
                    wake.node, mission.id, mission.title, wake.node, mission.id
                ),
            }
        }
        WakeEvent::Attention => {
            let (what, act) = attention_words(wake.reason, &pr);
            news(format!("Node {} needs you to look: {what}", wake.node), act)
        }
        WakeEvent::Resolved => {
            let how = match wake.action {
                Some(crate::missions::OperatorAction::Done) => {
                    "marked it done past the lines left open"
                }
                Some(crate::missions::OperatorAction::Drop) => "dropped it from the mission",
                Some(crate::missions::OperatorAction::Attach) => "attached the pull request to it",
                Some(crate::missions::OperatorAction::Leave) => match wake.reason {
                    Some(crate::missions::AttentionReason::UnplannedPr) => {
                        "left the pull request outside the nodes"
                    }
                    _ => "left it to you: judge what it needs",
                },
                Some(crate::missions::OperatorAction::Resume) => "set it to work again",
                None => "settled it",
            };
            format!(
                "The operator settled what waited on node {} of mission {} \"{}\": they {how}.\nRead `hide mission show {}` and go on from there.",
                wake.node, mission.id, mission.title, mission.id
            )
        }
        WakeEvent::Closed => format!(
            "Hide closed mission {} \"{}\": every node is done or dropped ({}).\nRead `hide mission show {}`; `hide mission reopen` opens it again.",
            mission.id,
            mission.title,
            match mission.status {
                MissionStatus::MergeWait => "waiting for the operator's merges",
                _ => "done",
            },
            mission.id
        ),
        WakeEvent::Reopened => format!(
            "The operator opened mission {} \"{}\" again from the map.\nRead `hide mission show {}` and `hide inbox`, and go on with what is left.",
            mission.id, mission.title, mission.id
        ),
        WakeEvent::Fix => {
            let what = match wake.why {
                Some(crate::missions::FixReason::CiFailed) => "failed its CI",
                Some(crate::missions::FixReason::Conflict) => "conflicts with its base branch",
                Some(crate::missions::FixReason::ChangesRequested) => {
                    "received a review asking for changes"
                }
                None => "needs your work",
            };
            format!(
                "{pr} {what} (mission {} \"{}\", node {}).\nFix it on your branch and push, then report to your Conductor as before.",
                mission.id, mission.title, wake.node
            )
        }
        WakeEvent::Warning => {
            let warning = node.and_then(|node| {
                node.warnings
                    .iter()
                    .find(|warning| Some(warning.kind) == wake.warning)
            });
            let what = match (wake.warning, node, warning) {
                (Some(WarningKind::ReviewFails), Some(node), _) => {
                    format!("failed its review {} times in a row", node.review_fails())
                }
                (Some(WarningKind::NoPush), _, Some(warning)) => format!(
                    "has worked {} minutes with no push",
                    warning.since_unix_ms.saturating_sub(warning.from_unix_ms) / 60_000
                ),
                _ => "shows signs its worker is stuck".to_owned(),
            };
            format!(
                "Node {} {what} (mission {} \"{}\").\nRead it with `hide mission show {} --mission {}` and judge whether its worker needs another approach or the node a fresh worker; the operator is told if the warning still stands in 30 minutes.",
                wake.node, mission.id, mission.title, wake.node, mission.id
            )
        }
    }
}

/// What a raise tells the Conductor, and what it asks of it.
fn attention_words(
    reason: Option<crate::missions::AttentionReason>,
    pr: &str,
) -> (String, &'static str) {
    use crate::missions::AttentionReason as Reason;
    match reason {
        Some(Reason::MergedUnproven) => (
            format!("{pr} is merged but a finish line is missing or failed"),
            "record the lines with `hide mission record`, skip one with its reason, or set the node",
        ),
        Some(Reason::WorkerEnded) => (
            "its worker ended with work left".to_owned(),
            "start a worker again, set the node, or drop it",
        ),
        Some(Reason::PrClosed) => (
            format!("{pr} was closed without a merge"),
            "replan the node or the nodes it blocks, or ask the operator",
        ),
        Some(Reason::UnplannedPr) => (
            format!("its worker opened {pr}, which no node holds"),
            "attach it with `hide mission node --pr`, or leave it",
        ),
        Some(Reason::WakeRefused) => (
            format!("Hide could not wake its sleeping worker about {pr}"),
            "wake or start the worker yourself",
        ),
        Some(Reason::CloseRefused) => (
            "Hide could not close the mission with every node finished".to_owned(),
            "check the node's lines and close the mission",
        ),
        Some(Reason::HumanGate) => (
            "its gate waits on the operator".to_owned(),
            "nothing: the operator answers on the decision card",
        ),
        Some(Reason::NotStarted) => (
            "it could start and was not started".to_owned(),
            "start its worker; or, to hold it, wait on a pull request as an `external` node, or hold the mission with `hide mission delay --line <why> --for <30m|2h>`; the operator is told in ten minutes",
        ),
        Some(Reason::WaitStood) => (
            format!("{pr} has waited over an hour on CI, a review or the merge"),
            "find what it waits on and move it, or merge it; the operator is told in ten minutes",
        ),
        Some(Reason::ConductorFailed | Reason::WarningStood) | None => (
            "the operator is asked to look".to_owned(),
            "nothing unless the operator asks",
        ),
    }
}

/// Keeps `seen` as what GitHub last said of its pull request, read at
/// `now`, with the CI results already told on its head; whether it moved.
/// A head Hide had not read of it is the node's last push
/// (with-hide/hide-private#552), and so is the first head read for a node a
/// book from before push times kept.
pub(super) fn note_seen(node: &mut MissionNode, mut seen: PrSeen, now: u64) -> bool {
    let unread = node.pushed_at_unix_ms.is_none();
    let held = node
        .pr_seen
        .iter_mut()
        .find(|held| held.number == seen.number);
    let pushed =
        seen.head.is_some() && (unread || held.as_ref().is_none_or(|held| held.head != seen.head));
    if pushed {
        node.pushed_at_unix_ms = Some(now);
    }
    match held {
        Some(held) => {
            keep_told(&mut seen, held);
            if *held == seen {
                return pushed;
            }
            *held = seen;
            true
        }
        None => {
            node.pr_seen.push(seen);
            true
        }
    }
}

/// What a Conductor woken about a pull request of its own does.
const PR_ACT: &str = "move a stacked PR above it onto the change, or answer the review";

/// Records one wake for `mission`'s Conductor and returns its letter's
/// intent; none when `PENDING_WAKE_LIMIT` letters already wait.
pub(super) fn record_wake(
    mission: &mut Mission,
    node: String,
    pr: Option<u32>,
    event: WakeEvent,
    head: Option<String>,
    key: Option<String>,
) -> Option<String> {
    record(mission, wake_about(node, pr, event, head, key))
}

/// A wake for the Conductor about `event`, numbered when it is recorded.
fn wake_about(
    node: String,
    pr: Option<u32>,
    event: WakeEvent,
    head: Option<String>,
    key: Option<String>,
) -> PendingWake {
    PendingWake {
        seq: 0,
        node,
        pr,
        event,
        head,
        to: None,
        reason: None,
        why: None,
        action: None,
        warning: None,
        ci: None,
        key,
    }
}

/// Numbers `wake` as the mission's next and records it (`push_wake`); a
/// refused wake takes no number.
fn record(mission: &mut Mission, mut wake: PendingWake) -> Option<String> {
    wake.seq = mission.wake_seq + 1;
    let intent = push_wake(mission, wake)?;
    mission.wake_seq += 1;
    Some(intent)
}

/// Records `wake` for its recipient and returns its letter's intent. With
/// `PENDING_WAKE_LIMIT` letters waiting for the Conductor a new one for it
/// is refused and logged, and every one waiting keeps its place
/// (engineering #15): what it was about stays on the map, and a raise
/// reaches the operator on its own. A worker's wake is capped apart
/// (`WORKER_WAKE_LIMIT`), so a Conductor that is gone never starves it. A
/// wake with no key of its own gets one from what it is about.
pub(super) fn push_wake(mission: &mut Mission, mut wake: PendingWake) -> Option<String> {
    let waiting = mission
        .wakes
        .iter()
        .filter(|waiting| waiting.to.is_some() == wake.to.is_some())
        .count();
    if wake.to.is_none() && waiting >= PENDING_WAKE_LIMIT {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.wake_refused_full",
            "mission": mission.id, "node": wake.node, "event": wake.event.as_str(),
            "cap": PENDING_WAKE_LIMIT,
        }));
        return None;
    }
    if wake.key.is_none() {
        wake.key = Some(wake_key(&wake));
    }
    let intent = wake_intent(mission, &wake);
    mission.wakes.push(wake);
    Some(intent)
}

/// The key of a wake from what it is about (B27): a pull request's news by
/// its head, an outside node's by its outcome, so the same fact read again
/// is the same letter. A wake no fact names, the operator's or a
/// Conductor's coming back, is its own by when it was made.
fn wake_key(wake: &PendingWake) -> String {
    let head = wake
        .head
        .as_deref()
        .map_or("-", |head| head.get(..12).unwrap_or(head));
    let pr = wake
        .pr
        .map_or_else(|| "-".to_owned(), |number| number.to_string());
    let fact = format!("{}:{}:{pr}:{head}", wake.event.as_str(), wake.node);
    match wake.event {
        // One result on one head is one letter, whichever read found it.
        WakeEvent::Ci => format!(
            "{fact}:{}",
            wake.ci.as_ref().map_or("-", |ci| ci.result.as_str())
        ),
        WakeEvent::Merged
        | WakeEvent::ChangesRequested
        | WakeEvent::NewHead
        | WakeEvent::OutsideDone
        | WakeEvent::OutsideFailed => fact,
        _ => format!(
            "{fact}:{}:{}",
            wake.action
                .map_or("-", crate::missions::OperatorAction::as_str),
            super::unix_milliseconds()
        ),
    }
}

/// The letter intent of one recorded wake to its recipient now: the
/// mission, the wake's key and the recipient. The same facts evaluated
/// again after a restart or a lost save make the same letter, which the
/// letters keep once (engineering #11); a Conductor that took the mission
/// over gets its own letter, since one intent reaches one recipient. A
/// wake recorded before keys names its `seq`; a key too long for an intent
/// is hashed.
pub(super) fn wake_intent(mission: &Mission, wake: &PendingWake) -> String {
    let conductor = &mission.conductor;
    let recipient = wake.to.as_deref().unwrap_or_else(|| {
        conductor
            .agent_id
            .as_deref()
            .unwrap_or(&conductor.actor.pane_id)
    });
    let Some(key) = wake.key.as_deref() else {
        return format!(
            "mission:{}:{}:{}:{}:{}:{}:{recipient}",
            mission.id,
            wake.node,
            wake.pr
                .map_or_else(|| "-".to_owned(), |number| number.to_string()),
            wake.event.as_str(),
            wake.head
                .as_deref()
                .map_or("-", |head| head.get(..12).unwrap_or(head)),
            wake.seq,
        );
    };
    let intent = format!("mission:{}:{key}:{recipient}", mission.id);
    if crate::delivery::valid_key(&intent) {
        return intent;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("mission:{}:{hash:016x}:{recipient}", mission.id)
}

/// A book from before a typed answer took its decision away still holds
/// the decision with that answer marked typed: it is settled now, as typing
/// it would settle it today (B29).
fn settle_typed_answers(book: &mut crate::missions::MissionBook) {
    let now = super::memory::now_ms();
    for mission in &mut book.missions {
        if mission
            .answer
            .as_ref()
            .is_some_and(|answer| answer.delivered)
        {
            super::mission_events::settle_answered(mission, now);
        }
    }
}

impl Runtime {
    pub(crate) fn install_mission_store(
        &mut self,
        store: crate::mission_store::MissionStore,
        restored: Restored,
    ) {
        match restored {
            Restored::Fresh => {}
            Restored::Read(mut book) | Restored::Moving(mut book) => {
                settle_typed_answers(&mut book);
                self.missions.book = book;
            }
            Restored::Unreadable(reason) => {
                crate::diagnostic!(json!({
                    "component": "missions",
                    "kind": "missions.unreadable",
                    "path": store.path().to_string_lossy(),
                    "message": reason,
                }));
                self.missions.unreadable = Some(reason);
            }
        }
        self.missions.store = Some(store);
        self.missions.book_edit = self.missions.book_edit.wrapping_add(1);
        self.missions.finished = super::agent_sleep::missions::finished_nodes(&self.missions.book);
    }

    /// Follows mission `id`'s outside nodes and waiting branches now, for a
    /// command that changed what is followed.
    pub(super) fn follow_mission_outside(&mut self, id: &str) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let (device, project) = (mission.device_id.clone(), mission.project.clone());
        self.follow_outside_stored(&device, &project)
    }

    /// Closes a mission as its Conductor's `hide mission close` asks, or, as
    /// `cancelled`, as the operator's `미션 종료` does for a mission no
    /// Conductor holds; `by` names which for the log.
    pub(super) fn close_mission(
        &mut self,
        id: &str,
        end: MissionEnd,
        report: Option<String>,
        by: &str,
        now: u64,
    ) -> Result<bool, String> {
        self.close_mission_with(id, end, report, by, now, None, |_| Vec::new())
    }

    /// `close_mission`, with what the closer adds to the closed mission
    /// (`also`, which returns its events) in the same save, and the events
    /// keyed by `key` when an evaluation closes it (B27).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn close_mission_with(
        &mut self,
        id: &str,
        end: MissionEnd,
        report: Option<String>,
        by: &str,
        now: u64,
        key: Option<&str>,
        also: impl FnOnce(&mut Mission) -> Vec<crate::mission_events::MissionEvent>,
    ) -> Result<bool, String> {
        let current = self.missions.book.get(id).ok_or("mission_unknown")?;
        if current.status != MissionStatus::Cancelled {
            self.gate_close(current, end)?;
        }
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        let status = match (target.status, end) {
            (MissionStatus::Cancelled, _) => return Err("mission_cancelled".into()),
            (MissionStatus::Planning, MissionEnd::Done | MissionEnd::MergeWait) => {
                return Err("mission_not_started".into());
            }
            (_, MissionEnd::Done) => MissionStatus::Done,
            // A stack already merged has nothing to wait on (B41).
            (_, MissionEnd::MergeWait) if stack_settled(target) => MissionStatus::Done,
            (_, MissionEnd::MergeWait) => MissionStatus::MergeWait,
            (_, MissionEnd::Cancelled) => MissionStatus::Cancelled,
        };
        let before = target.clone();
        // A started mission's first close: a reopened one closing again, or a
        // waiting one closing as done, was counted when it first closed.
        let first_end = before.status == MissionStatus::Running && before.reopened_on.is_none();
        target.status = status;
        target.end_asked_at_unix_ms = None;
        if report.is_some() {
            target.report = report;
        }
        let ended = if status == MissionStatus::MergeWait {
            Vec::new()
        } else {
            super::mission_events::end_decision(target, now)
        };
        target.closed_at_unix_ms = Some(now);
        target.reopened_on = None;
        target.updated_at_unix_ms = now;
        if status == MissionStatus::Cancelled {
            // A cancelled mission wakes no one (D-51).
            target.wakes.clear();
        }
        let changed = *target != before;
        let mut events = super::mission_events::close_events(&before, target, now);
        events.extend(ended);
        events.extend(also(target));
        if let Some(key) = key {
            super::mission_health::key_events(key, &mut events);
        }
        if !target.status.open() {
            self.missions.answers_held.remove(id);
        }
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.closed",
            "mission": id, "end": status, "by": by,
        }));
        if first_end && let Some(sink) = &self.product_analytics {
            use crate::product_analytics::{Event, MissionOutcome};
            let outcome = match status {
                MissionStatus::MergeWait => MissionOutcome::MergeWait,
                MissionStatus::Cancelled => MissionOutcome::Cancelled,
                _ => MissionOutcome::Done,
            };
            sink.capture(Event::MissionEnded { outcome });
        }
        let place = (target.device_id.clone(), target.project.clone());
        self.record_mission_events(id, events);
        // A mission now waiting for its branches reads them at once.
        if status == MissionStatus::MergeWait {
            self.follow_outside_stored(&place.0, &place.1);
        }
        Ok(changed)
    }

    /// Closes mission `id`, waiting for its merges, as done once its stack
    /// is in main and every done node's lines still hold (B10, B18, B41);
    /// whether it closed.
    pub(super) fn close_if_landed(&mut self, id: &str, now: u64) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        if mission.status != MissionStatus::MergeWait
            || !stack_settled(mission)
            || self
                .close_held(mission, self.mission_pulls(mission))
                .is_some()
        {
            return false;
        }
        let Some(mission) = self.missions.book.get_mut(id) else {
            return false;
        };
        mission.status = MissionStatus::Done;
        mission.closed_at_unix_ms = Some(now);
        mission.reopened_on = None;
        mission.updated_at_unix_ms = now;
        let events = super::mission_events::end_decision(mission, now);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.closed_by_branches", "mission": id,
        }));
        self.missions.answers_held.remove(id);
        self.record_mission_events(id, events);
        true
    }

    /// The pull request lists the missions' maps read were replaced.
    pub(super) fn mission_pulls_moved(&mut self) {
        self.missions.github_edit = self.missions.github_edit.wrapping_add(1);
    }

    /// Records on each node whose worker's launch is not yet kept the launch
    /// its pane now proves for that worker (`proven_launch`), so the map
    /// still names what ran once the pane is gone. A closed mission's nodes
    /// are proven too, so a mission that closed before its worker's proof
    /// landed (a session reported late, a close on the same pass) still gets
    /// it while the pane's record stands. It runs on the agent pass that
    /// settles the pane launch records, right after the settle, and looks
    /// again only when a launch record (any write, a wake's or a resume's
    /// included), the ledger or the book moved since it last looked; it
    /// compares in memory only and saves through the book's save thread
    /// when one moved.
    pub(super) fn record_node_launches(&mut self) {
        let ledger = self.delivery_state().ok();
        let inputs = (self.missions.book_edit, self.missions.launches_edit);
        let same = |made: &LaunchProofMade| {
            (made.book_edit, made.launches_edit) == inputs
                && ledger
                    .as_ref()
                    .is_some_and(|ledger| std::ptr::eq(made.ledger.as_ptr(), Arc::as_ptr(ledger)))
        };
        if self.missions.launch_proof_made.as_ref().is_some_and(same) {
            return;
        }
        self.missions.launch_proof_made = ledger.as_ref().map(|ledger| LaunchProofMade {
            ledger: Arc::downgrade(ledger),
            book_edit: inputs.0,
            launches_edit: inputs.1,
        });
        let waiting = |node: &crate::missions::MissionNode| {
            node.agent_id.as_deref().is_some_and(|id| {
                node.launched
                    .as_ref()
                    .is_none_or(|launch| launch.agent_id != id)
            })
        };
        let missions = &self.missions.book.missions;
        if !missions
            .iter()
            .flat_map(|mission| &mission.nodes)
            .any(waiting)
        {
            return;
        }
        let Some(ledger) = ledger else {
            return;
        };
        let records = worker_records(Some(&ledger), missions.iter());
        let launches = &self.snapshot.ui_state.pane_launches;
        let node_id = self.node.as_str();
        let mut proven = Vec::new();
        for (index, mission) in missions.iter().enumerate() {
            for (at, node) in mission.nodes.iter().enumerate() {
                if !waiting(node) {
                    continue;
                }
                let launch = node
                    .agent_id
                    .as_deref()
                    .and_then(|id| records.get(id))
                    .and_then(|record| proven_launch(launches, node_id, record));
                if let Some(launch) = launch {
                    proven.push((index, at, launch));
                }
            }
        }
        if proven.is_empty() {
            return;
        }
        for (index, at, launch) in proven {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.node_launch_recorded",
                "mission": self.missions.book.missions[index].id,
                "node": self.missions.book.missions[index].nodes[at].key,
                "agent": launch.agent_id,
            }));
            self.missions.book.missions[index].nodes[at].launched = Some(launch);
        }
        self.save_missions();
        if let Some(made) = &mut self.missions.launch_proof_made {
            made.book_edit = self.missions.book_edit;
        }
    }

    pub(crate) fn save_missions(&mut self) {
        self.missions.book_edit = self.missions.book_edit.wrapping_add(1);
        // A reopened mission is held open until a node moves (B18); the
        // move is seen here, where every change is saved, so a node that
        // went back to work and finished again has moved even though it
        // looks as it did.
        for mission in &mut self.missions.book.missions {
            if mission
                .reopened_on
                .as_ref()
                .is_some_and(|marks| *marks != mission.node_marks())
            {
                mission.reopened_on = None;
            }
        }
        self.note_mission_finishes();
        // A book that could not be read is kept as it is on disk: the empty
        // one in memory is not the operator's missions.
        if self.missions.unreadable.is_some() {
            return;
        }
        let leaving = self.shelve_missions();
        self.trim_shelved_missions(&leaving);
        if let Some(store) = &self.missions.event_store {
            store.save_book(Arc::new(self.missions.book.clone()), leaving);
        }
    }

    /// Shelves the oldest closed missions that ask no one past
    /// `LOADED_CLOSED_LIMIT` (engineering #15): their lines stay in the
    /// book and their files on disk, and their bodies go to the writer
    /// once, for the last state their files take, with their histories
    /// let go from memory. Returns those bodies.
    fn shelve_missions(&mut self) -> Vec<Mission> {
        let book = &mut self.missions.book;
        let resting = book
            .missions
            .iter()
            .filter(|mission| crate::missions::MissionLine::of(mission).at_rest())
            .count();
        let Some(mut over) = resting.checked_sub(crate::mission_store::LOADED_CLOSED_LIMIT) else {
            return Vec::new();
        };
        if let Some(store) = &self.missions.event_store {
            over = over.min(
                crate::mission_events::PENDING_SHELVED_LIMIT
                    .saturating_sub(store.unsaved_shelved().len()),
            );
        }
        if over == 0 {
            return Vec::new();
        }
        let mut oldest: Vec<(u64, u64)> = book
            .missions
            .iter()
            .map(crate::missions::MissionLine::of)
            .filter(crate::missions::MissionLine::at_rest)
            .map(|line| line.closed_order())
            .collect();
        oldest.sort_unstable();
        let cut = oldest[over - 1];
        let (leaving, kept): (Vec<Mission>, Vec<Mission>) = std::mem::take(&mut book.missions)
            .into_iter()
            .partition(|mission| {
                let line = crate::missions::MissionLine::of(mission);
                line.at_rest() && line.closed_order() <= cut
            });
        book.missions = kept;
        for mission in &leaving {
            book.shelved.push(crate::missions::MissionLine::of(mission));
            self.missions.events.remove(&mission.id);
            if let Some(store) = &self.missions.event_store {
                store.release(&mission.id);
            }
        }
        self.missions.events_edit = self.missions.events_edit.wrapping_add(1);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "missions.shelved",
            "missions": leaving.iter().map(|mission| mission.id.as_str()).collect::<Vec<_>>(),
        }));
        let known: Vec<String> = book
            .missions
            .iter()
            .map(|mission| mission.id.clone())
            .collect();
        self.drop_detail_items(|item| match item {
            DetailItem::Mission { mission } => known.contains(mission),
            _ => true,
        });
        if self
            .missions
            .shown
            .as_ref()
            .is_some_and(|shown| !known.contains(shown))
        {
            self.missions.shown = None;
        }
        leaving
    }

    /// Finished records are discarded oldest first, never new work refused
    /// for a full shelf. The writer removes their body/history after index.
    fn trim_shelved_missions(&mut self, leaving: &[Mission]) {
        let lines = &mut self.missions.book.shelved;
        let mut resting = lines.iter().filter(|line| line.at_rest()).count();
        if resting <= crate::mission_store::SHELVED_LIMIT {
            return;
        }
        let mut protected = self
            .missions
            .event_store
            .as_ref()
            .map(|store| store.unsaved_shelved())
            .unwrap_or_default();
        protected.extend(leaving.iter().map(|mission| mission.id.clone()));
        lines.sort_by_key(crate::missions::MissionLine::closed_order);
        let mut gone = Vec::new();
        while resting > crate::mission_store::SHELVED_LIMIT {
            let Some(at) = lines
                .iter()
                .position(|line| line.at_rest() && !protected.contains(&line.id))
            else {
                break;
            };
            gone.push(lines.remove(at));
            resting -= 1;
        }
        for line in gone {
            crate::diagnostic!(
                json!({"component":"missions","kind":"missions.shelved_evicted","mission":line.id})
            );
        }
    }

    /// Runs a `hide mission` command from `caller`'s pane on `device`.
    /// Returns the answer and whether anything changed.
    #[cfg(test)]
    pub(crate) fn mission_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: MissionCommand,
    ) -> Result<(Value, bool), String> {
        self.mission_command_with(device, caller, expected, command, None)
    }

    /// `mission_command`, with the shelved mission the command names read
    /// back off the lock (`Core::mission_command`), or why it could not be.
    pub(crate) fn mission_command_with(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: MissionCommand,
        visiting: Option<Result<Visiting, String>>,
    ) -> Result<(Value, bool), String> {
        // Only a command that can change the record is measured, and only
        // it pays for the copy a refusal puts back.
        let before = command.mutates().then(|| self.missions.book.clone());
        let taken = match &command {
            MissionCommand::Take { mission, .. } => Some(mission.clone()),
            _ => None,
        };
        let answer = self.apply_mission_command(device, caller, expected, command, visiting)?;
        let Some(before) = before.filter(|_| answer.1) else {
            return Ok(answer);
        };
        // A change that would make the saved book larger than a start reads
        // back is refused whole, like a full book (engineering #15).
        if !crate::missions::may_keep(&before, &self.missions.book) {
            self.missions.book = before;
            self.save_missions();
            crate::diagnostic!(json!({
                "component": "missions", "kind": "missions.record_full",
            }));
            return Err("mission_full".into());
        }
        if let Some(id) = taken {
            // A take whose workers cannot be handed over is not made, so
            // asking it again tries again.
            let former = before
                .get(&id)
                .and_then(|mission| mission.conductor.agent_id.clone());
            let conductor = self
                .missions
                .book
                .get(&id)
                .and_then(|mission| mission.conductor.agent_id.clone());
            if let Err(code) =
                self.adopt_mission_workers(&id, conductor.as_deref(), former.as_deref())
            {
                self.missions.book = before;
                self.save_missions();
                return Err(code);
            }
            // What waited for a Conductor goes to the one that took it.
            self.flush_mission_wakes();
        }
        Ok(answer)
    }

    fn apply_mission_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: MissionCommand,
        visiting: Option<Result<Visiting, String>>,
    ) -> Result<(Value, bool), String> {
        if !command.valid() {
            return Err("mission_text_invalid".into());
        }
        if self.missions.unreadable.is_some() {
            return Err("mission_unavailable".into());
        }
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("agent_pane_required".into());
        };
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
        let actor = self
            .delivery_observations
            .get(&pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        actor.require_native_identity()?;
        let now = super::memory::now_ms();
        let project = self
            .catalog_project_of(device, &context.checkout_path)
            .map(|project| project.root_path.clone());
        match command {
            MissionCommand::Create {
                title,
                scope,
                cap,
                github,
            } => {
                if self.missions.book.conducted_open(&actor).is_some() {
                    return Err("mission_already_conducting".into());
                }
                if let Some(store) = &self.missions.event_store {
                    let pending = store.unsaved_shelved().len();
                    if pending >= crate::mission_events::PENDING_SHELVED_LIMIT {
                        crate::diagnostic!(
                            json!({"component":"missions","kind":"missions.save_pending_full","pending":pending,"limit":crate::mission_events::PENDING_SHELVED_LIMIT})
                        );
                        return Err(crate::coordination::refusal(
                            "mission_save_pending",
                            "32 shelved mission saves are pending; fix storage and retry",
                        ));
                    }
                }
                let project = match project {
                    Some(project) => project,
                    // Before the machine's projects are first read, an unknown checkout says nothing yet.
                    None if self.projects_loading(device) => {
                        return Err("mission_projects_loading".into());
                    }
                    None => return Err("mission_project_unknown".into()),
                };
                let settings = self.snapshot.ui_state.conductor_settings.clone();
                let cap = cap.unwrap_or(settings.concurrency);
                if !(1..=crate::missions::CAP_LIMIT).contains(&cap) {
                    return Err("mission_cap_out_of_range".into());
                }
                let id = self.missions.book.allocate();
                let mission = Mission {
                    id: id.clone(),
                    title,
                    project,
                    workspace_id: context.workspace_id.clone(),
                    checkout_path: context.checkout_path.clone(),
                    device_id: device.to_owned(),
                    conductor: self.conductor_of(device, &pane, actor),
                    status: MissionStatus::Planning,
                    scope: scope.unwrap_or(settings.default_scope),
                    cap,
                    candidates: settings.candidates.clone(),
                    strength: Some(settings.default_strength),
                    checkers: None,
                    exempt: false,
                    decided: Vec::new(),
                    nodes: Vec::new(),
                    merge_order: Vec::new(),
                    check: None,
                    github,
                    decision: None,
                    question: None,
                    question_seq: 0,
                    conductor_views: Vec::new(),
                    answer: None,
                    answer_failed: None,
                    delay: None,
                    delay_until_unix_ms: None,
                    report: None,
                    created_at_unix_ms: now,
                    updated_at_unix_ms: now,
                    started_at_unix_ms: None,
                    closed_at_unix_ms: None,
                    wakes: Vec::new(),
                    wake_seq: 0,
                    round_seq: 0,
                    end_asked_at_unix_ms: None,
                    attention_seq: 0,
                    reopened_on: None,
                    handover: None,
                };
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.created",
                    "mission": id, "device": device, "pane": pane,
                }));
                self.missions.book.missions.push(mission);
                self.save_missions();
                Ok((
                    json!({"mission": self.mission_view_of(&id), "settings": settings}),
                    true,
                ))
            }
            MissionCommand::Plan { mission, mut plan } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let current = self
                    .missions
                    .book
                    .get(&id)
                    .cloned()
                    .ok_or("mission_unknown")?;
                self.plan_profiles(&current, &mut plan)?;
                // A plan opens nothing in the lane: the map opens only from
                // the operator's press, and the shell shows a first plan on
                // the Conductor pane's own chip (PRD mission-pane-chip D-02).
                let mut next = crate::missions::apply_plan(&current, &plan, now)?;
                self.plan_candidates(&current, &plan)?;
                // A replan does not make a node an agent on another machine
                // works or checks one only the project's machine runs.
                for node in next.nodes.iter().filter(|node| !node.runs_elsewhere()) {
                    let checkers = node
                        .criteria
                        .iter()
                        .flat_map(|line| &line.checkers)
                        .filter_map(|checker| checker.agent_id.as_ref());
                    for id in node.agent_id.iter().chain(checkers) {
                        if self.registered_elsewhere(&next, id)?.is_some() {
                            return Err(crate::coordination::refusal(
                                "mission_machine_node_refused",
                                &node.key,
                            ));
                        }
                    }
                }
                let Mission {
                    nodes, round_seq, ..
                } = &mut next;
                for node in nodes.iter_mut() {
                    super::mission_outside::begin_round(round_seq, current.node(&node.key), node);
                }
                let overlaps = crate::missions::overlaps(&self.missions.book, &next);
                let events = super::mission_events::plan_events(
                    &current,
                    &next,
                    plan.strength_why.as_deref(),
                    now,
                );
                *self.missions.book.get_mut(&id).ok_or("mission_unknown")? = next;
                self.record_mission_events(&id, events);
                // The plan's outside nodes are read now, not at the next
                // GitHub read or fetch.
                self.follow_mission_outside(&id);
                Ok((
                    json!({"mission": self.mission_view_of(&id), "overlaps": overlaps}),
                    true,
                ))
            }
            MissionCommand::Node {
                mission,
                node,
                patch,
            } => {
                // A plan sets `planned`; the Conductor moves a node to work,
                // done or dropped (PRD mission-node-simple D-16, B25).
                if let Some(status) = patch.status
                    && !NodeStatus::SET_BY_CONDUCTOR.contains(&status)
                {
                    return Err(crate::coordination::refusal(
                        "mission_status_refused",
                        &NodeStatus::SET_BY_CONDUCTOR
                            .map(NodeStatus::as_str)
                            .join(", "),
                    ));
                }
                let ledger = self.delivery_state().ok();
                let (id, caller) = self.node_mission(
                    &actor,
                    ledger.as_deref(),
                    mission.as_deref(),
                    &node,
                    &patch,
                )?;
                // A worker whose spawn is under way has started the node, so
                // a hold set now could not stop it.
                if matches!(patch.hold, Some(Some(_)))
                    && ledger.as_deref().is_some_and(|ledger| {
                        let intent = format!("{id}:{node}");
                        ledger.spawns.iter().any(|spawn| {
                            !spawn.completed
                                && spawn.mission.as_deref() == Some(id.as_str())
                                && spawn.intent == intent
                        })
                    })
                {
                    return Err(crate::coordination::refusal("mission_node_started", &node));
                }
                let current = self.missions.book.get(&id).ok_or("mission_unknown")?;
                let index = current
                    .nodes
                    .iter()
                    .position(|row| row.key == node)
                    .ok_or("mission_node_unknown")?;
                let named = [
                    patch.reviewer.as_deref(),
                    patch.verifier.as_deref(),
                    patch.agent_id.as_ref().and_then(Option::as_deref),
                ];
                // An agent from another machine is bound only where a spawn
                // for this node would start it; one it holds already stays.
                let held = &current.nodes[index];
                for id in named.into_iter().flatten() {
                    if held.agent_id.as_deref() != Some(id)
                        && !Self::checks_node(held, id)
                        && let Some(machine) = self.registered_elsewhere(current, id)?
                    {
                        self.admit_elsewhere(current, &machine, Some(held))?;
                    }
                }
                let mut bound = current.nodes[index].clone();
                for (kind, checker) in [
                    (CheckKind::Review, &patch.reviewer),
                    (CheckKind::Verify, &patch.verifier),
                ] {
                    if let Some(checker) = checker {
                        bound = self.bind_checker(&bound, kind, checker)?;
                    }
                }
                // A node's checker never becomes its worker (B4).
                if let Some(Some(worker)) = &patch.agent_id
                    && Self::checks_node(&bound, worker)
                {
                    return Err(crate::coordination::refusal(
                        "mission_checker_is_worker",
                        &node,
                    ));
                }
                let mut next = self.prospective_node(current, &bound, &patch, now)?;
                // The worker bound now keeps the words it was started on, so
                // the panel still shows them after the ledger lets the letter
                // go; a worker whose spawn kept no brief leaves the last.
                if next.agent_id != bound.agent_id
                    && let Some(brief) = next
                        .agent_id
                        .as_deref()
                        .and_then(|worker| self.worker_brief(worker))
                {
                    next.brief = Some(brief);
                }
                // Done is held against the node as this command leaves it:
                // a done set now, or a done node given another pull request
                // or worker (B9).
                let subject_moved = next.prs != bound.prs || next.agent_id != bound.agent_id;
                if patch.status == Some(NodeStatus::Done) || subject_moved {
                    self.gate_node_done(current, &next)?;
                }
                // A pull request named now starts its wakes from what GitHub
                // says of it now, so its first change after this one wakes.
                for number in next.prs.clone() {
                    if !current.nodes[index].prs.contains(&number)
                        && let Some(seen) = self.pull_seen(current, number)
                    {
                        note_seen(&mut next, seen, now);
                    }
                }
                // The Conductor moving the node settles what waited on it
                // (B17); a cause it settles is not raised again.
                if patch.status.is_some() {
                    super::mission_health::settle(&id, &mut next, "conductor");
                }
                // Taken back to work from a finish, it is not finished again
                // by Hide on the pull requests it holds now.
                // The mark names the pull requests it held when it was taken
                // back, so one named in the same command finishes it again.
                if current.nodes[index].status.finished() && next.status == NodeStatus::Working {
                    next.resumed = Some(current.nodes[index].resumed_cause());
                    next.back_to_work(now);
                }
                let reads = reads_branch(current.status, &next);
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                super::mission_outside::begin_round(
                    &mut target.round_seq,
                    Some(&target.nodes[index]),
                    &mut next,
                );
                let changed = target.nodes[index] != next;
                let events = super::mission_events::node_events(&target.nodes[index], &next, now);
                target.nodes[index] = next;
                super::mission_health::prune_attention_wakes(target);
                // A waiting mission's node named another branch or finished
                // again, or a direct change finished or moved to another
                // branch while its mission runs: its branches are read now.
                let follow = target.status == MissionStatus::MergeWait || reads;
                if changed {
                    target.updated_at_unix_ms = now;
                    self.record_mission_events(&id, events);
                    if follow {
                        self.follow_mission_outside(&id);
                    }
                }
                // A hold set or lifted is the Conductor's move on the node:
                // whatever stood before it is counted again from now.
                if patch.hold.is_some() {
                    self.restart_stall_clock(&id, Some(&node));
                }
                // What the Conductor just did may follow a pull request it
                // pushed: its project is read again within the read's own
                // spacing (D-09, B23).
                self.ask_mission_pulls(&id);
                // A worker's patch binds no agent, so the registrations read
                // for it draw the answer.
                let view = match &caller {
                    Some(caller) => self.mission_view_for(&id, caller),
                    None => self.mission_view_of(&id),
                };
                Ok((json!({"mission": view}), changed))
            }
            MissionCommand::Start { mission } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
                if target.nodes.is_empty() {
                    return Err("mission_no_nodes".into());
                }
                let changed = target.status == MissionStatus::Planning;
                if changed {
                    target.status = MissionStatus::Running;
                    target.started_at_unix_ms = Some(now);
                    target.updated_at_unix_ms = now;
                    let nodes = crate::product_analytics::MissionSize::of(target.nodes.len());
                    if let Some(sink) = &self.product_analytics {
                        sink.capture(crate::product_analytics::Event::MissionStarted { nodes });
                    }
                    self.save_missions();
                    self.follow_mission_outside(&id);
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Decide {
                mission,
                line,
                why,
                options,
                recommend,
                node,
                nodes,
                show,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                let changed = self.decide_mission(
                    &id,
                    line,
                    (why, options, recommend, node, nodes),
                    Some((&actor, &show)),
                    now,
                )?;
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Delay {
                mission,
                line,
                for_minutes,
            } => self.set_mission_delay(&actor, mission.as_deref(), line, for_minutes, now),
            MissionCommand::Close {
                mission,
                end,
                report,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let changed = self.close_mission(&id, end, report, "conductor", now)?;
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Reopen { mission } => {
                if let Some(id) = mission.as_deref() {
                    self.unshelve_mission(id, &actor, visiting)?;
                }
                let id = self.conducted_mission(&actor, mission.as_deref(), false)?;
                let changed = self.reopen_mission(&id, now)?;
                if changed {
                    self.save_missions();
                    self.follow_mission_outside(&id);
                }
                Ok((json!({"mission": self.mission_view_of(&id)}), changed))
            }
            MissionCommand::Record {
                mission,
                node,
                line,
                pass,
                evidence,
                summary,
                commit,
                rerun,
            } => self.record_criterion(
                &actor,
                mission.as_deref(),
                &node,
                &line,
                (pass, evidence, summary, commit, rerun),
                now,
            ),
            MissionCommand::Skip {
                mission,
                node,
                line,
                reason,
                decision,
            } => self.skip_criterion(
                &actor,
                mission.as_deref(),
                &node,
                &line,
                (reason, decision),
                now,
            ),
            MissionCommand::Defect {
                mission,
                node,
                found_by,
                line,
            } => {
                let id = self.conducted_mission(&actor, mission.as_deref(), true)?;
                self.add_mission_defect(&actor, &id, &node, (found_by, line), now)
            }
            MissionCommand::Gate {
                mission,
                node,
                pass,
                said,
            } => self.relay_gate(&actor, mission.as_deref(), &node, (pass, said), now),
            MissionCommand::Take { mission, handover } => {
                let target = self.missions.book.get(&mission).ok_or("mission_unknown")?;
                if target.status == MissionStatus::Cancelled {
                    return Err("mission_cancelled".into());
                }
                if target.device_id != device || Some(&target.project) != project.as_ref() {
                    return Err("mission_other_project".into());
                }
                if target.conductor.actor.same_identity(&actor) {
                    return Ok((json!({"mission": self.mission_view_of(&mission)}), false));
                }
                // A Conductor still there is taken over only with the
                // operator's handover made on it, by the session it was
                // made for, once (PRD mission-node-simple B15).
                let handed = target.handover.as_ref().is_some_and(|made| {
                    handover.as_deref() == Some(made.token.as_str())
                        && made.from.same_identity(&target.conductor.actor)
                        && now.saturating_sub(made.at_unix_ms) < crate::missions::HANDOVER_TTL_MS
                });
                let present = self.conductor_present(&target.conductor.actor);
                if !handed && present {
                    // The current Conductor is named so the second session
                    // can tell the operator who holds it (B45).
                    return Err(crate::coordination::refusal(
                        "mission_conducted",
                        &target.conductor.actor.name,
                    ));
                }
                if self.missions.book.conducted_open(&actor).is_some() {
                    return Err("mission_already_conducting".into());
                }
                let conductor = self.conductor_of(device, &pane, actor);
                let target = self
                    .missions
                    .book
                    .get_mut(&mission)
                    .ok_or("mission_unknown")?;
                target.conductor = conductor;
                target.handover = None;
                // The new Conductor is told each raise it now settles, and its
                // safety-net time starts now.
                super::mission_health::tell_new_conductor(target);
                target.updated_at_unix_ms = now;
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.taken",
                    "mission": mission, "pane": pane, "handed": handed,
                }));
                self.note_conductor_arrived(&mission, now);
                self.save_missions();
                Ok((json!({"mission": self.mission_view_of(&mission)}), true))
            }
            MissionCommand::Show { mission, node } => {
                let ledger = self.delivery_state().ok();
                let (id, node, caller) = match mission {
                    Some(id) => (
                        self.missions
                            .book
                            .get(&id)
                            .map(|mission| mission.id.clone())
                            .or_else(|| self.missions.book.shelf(&id).map(|line| line.id.clone()))
                            .ok_or("mission_unknown")?,
                        node,
                        None,
                    ),
                    None => {
                        let caller = MissionCaller::of(
                            &actor,
                            ledger.as_deref(),
                            &self.missions.book.missions,
                        );
                        let (id, node) = self.shown_mission(&caller, node)?;
                        (id, node, Some(caller))
                    }
                };
                // Resolve node-key precedence before using an off-lock shelf read.
                let shelved = if self.missions.book.shelf(&id).is_some() {
                    Some(
                        visiting
                            .ok_or("mission_unavailable")?
                            .map_err(|reason| self.shelved_unreadable(&id, &reason))?
                            .mission,
                    )
                } else {
                    None
                };
                // One pass over the ledger's agents serves the view and both
                // letter lists: the one that found the caller's mission, else
                // one for the mission's workers and checkers.
                let mission = self
                    .missions
                    .book
                    .get(&id)
                    .or(shelved.as_ref())
                    .ok_or("mission_unknown")?;
                let agents = match &caller {
                    Some(caller) => caller.records_of(mission),
                    None => worker_records(ledger.as_deref(), std::iter::once(mission)),
                };
                let sleeping = self.sleeping_registrations(agents.values().copied());
                let view = self.view_mission(mission, &agents, &sleeping, ledger.as_deref());
                let Some(key) = node else {
                    let mut shown = json!(view);
                    for (node, view) in shown["nodes"]
                        .as_array_mut()
                        .into_iter()
                        .flatten()
                        .zip(&view.nodes)
                    {
                        self.name_worker_items(mission, view, node);
                    }
                    return Ok((json!({"mission": shown}), false));
                };
                let view = view
                    .nodes
                    .into_iter()
                    .find(|node| node.key == key)
                    .ok_or("mission_node_unknown")?;
                let mut shown = json!(view);
                self.name_worker_items(mission, &view, &mut shown);
                let node = mission.node(&key).ok_or("mission_node_unknown")?;
                Ok((
                    json!({
                        "mission_id": id,
                        "node": shown,
                        "letters": ledger.as_deref().map(|ledger| node_letters(ledger, &agents, node)),
                        "checker_letters": ledger
                            .as_deref()
                            .map(|ledger| node_checker_letters(ledger, &agents, mission, node)),
                    }),
                    false,
                ))
            }
            MissionCommand::List => {
                let mut lines: Vec<_> = self
                    .missions
                    .book
                    .missions
                    .iter()
                    .map(|mission| (crate::missions::MissionLine::of(mission), false))
                    .chain(
                        self.missions
                            .book
                            .shelved
                            .iter()
                            .cloned()
                            .map(|line| (line, true)),
                    )
                    .filter(|(line, _)| {
                        line.device_id == device && Some(&line.project) == project.as_ref()
                    })
                    .collect();
                lines.sort_by_key(|(line, _)| crate::missions::mission_number(&line.id));
                let items: Vec<Value> = lines
                    .iter()
                    .map(|(line, shelved)| {
                        json!({
                            "id": line.id,
                            "title": line.title,
                            "project": line.project,
                            "status": line.status,
                            "conductor": line.conductor.actor.name,
                            "present": self.conductor_present(&line.conductor.actor),
                            "yours": line.conductor.actor.same_identity(&actor),
                            "shelved": shelved,
                        })
                    })
                    .collect();
                Ok((json!({"items": items}), false))
            }
            MissionCommand::Settings => {
                let mut settings = json!(self.snapshot.ui_state.conductor_settings);
                settings["candidates"] = self.candidate_items().into();
                Ok((json!({ "settings": settings }), false))
            }
        }
    }

    fn set_mission_delay(
        &mut self,
        actor: &Actor,
        mission: Option<&str>,
        line: Option<String>,
        for_minutes: Option<u32>,
        now: u64,
    ) -> Result<(Value, bool), String> {
        let id = self.conducted_mission(actor, mission, true)?;
        let target = self.missions.book.get_mut(&id).ok_or("mission_unknown")?;
        let until = for_minutes.map(|minutes| now + u64::from(minutes) * 60_000);
        let changed = target.delay != line || target.delay_until_unix_ms != until;
        target.delay = line;
        target.delay_until_unix_ms = until;
        if changed {
            target.updated_at_unix_ms = now;
            self.restart_stall_clock(&id, None);
            self.save_missions();
        }
        Ok((json!({"mission": self.mission_view_of(&id)}), changed))
    }

    /// What `hide mission show` from `caller` names no mission for: the
    /// mission it means (`mission_meant`, with-hide/hide#2302), and `node`
    /// read in it. A key that is no node of that mission but names a
    /// mission is that whole mission, as Hide's own letters write
    /// `hide mission show <mission id>` (with-hide/hide#2301), even for a
    /// caller whose mission is ambiguous.
    fn shown_mission(
        &self,
        caller: &MissionCaller<'_>,
        node: Option<String>,
    ) -> Result<(String, Option<String>), String> {
        let taken = self.mission_meant(caller);
        if let Some(key) = &node
            && !matches!(&taken, Ok(Some(mission)) if mission.node(key).is_some())
            && (self.missions.book.get(key).is_some() || self.missions.book.shelf(key).is_some())
        {
            return Ok((key.clone(), None));
        }
        let mission = taken?.ok_or("mission_none")?;
        Ok((mission.id.clone(), node))
    }

    /// The mission `hide mission node <node>` from `actor` changes: one it
    /// conducts, or, for what a worker sets on its own node
    /// (`NodePatch::workers_own`), the open mission whose node `node` it
    /// works (with-hide/hide#2302), with the caller its answer is drawn
    /// from. Anything else from an agent of the mission, or about another
    /// node, stays its Conductor's.
    fn node_mission<'a>(
        &self,
        actor: &'a Actor,
        ledger: Option<&'a crate::delivery::ledger::Ledger>,
        named: Option<&str>,
        node: &str,
        patch: &crate::missions::NodePatch,
    ) -> Result<(String, Option<MissionCaller<'a>>), String> {
        let refused = match self.conducted_mission(actor, named, true) {
            Ok(id) => return Ok((id, None)),
            Err(refused) => refused,
        };
        use super::mission_criteria::Part;
        let caller = MissionCaller::of(actor, ledger, &self.missions.book.missions);
        let found = match named {
            Some(id) => self.missions.book.get(id),
            None => self.mission_meant(&caller)?,
        };
        let Some(mission) = found else {
            return Err(refused);
        };
        match (
            Self::part_in(mission, &caller, Some(node)),
            Self::part_in(mission, &caller, None),
        ) {
            (Some(Part::Worker), _) if patch.workers_own() => {}
            (_, Some(Part::Worker | Part::Checker)) => {
                return Err("mission_not_conductor".into());
            }
            _ => return Err(refused),
        }
        match mission.status {
            MissionStatus::Cancelled => Err("mission_cancelled".into()),
            MissionStatus::Done => Err("mission_closed".into()),
            _ => Ok((mission.id.clone(), Some(caller))),
        }
    }

    /// The mission a command from `actor` acts on: the one it names, which
    /// it must conduct, or the one it conducts. `changing` refuses a mission
    /// that is closed, since a plan or a node of one moves only once it is
    /// reopened.
    /// The shelved mission a command needs read back, if it names one:
    /// where its file and, for a reopen, its history are, read off the lock
    /// (`Core::mission_command`).
    pub(crate) fn shelved_files(
        &self,
        command: &MissionCommand,
    ) -> Option<crate::mission_store::ShelfRead> {
        let (id, history) = match command {
            MissionCommand::Show { mission, node } => (mission.as_ref().or(node.as_ref())?, false),
            MissionCommand::Reopen { mission: Some(id) } => (id, true),
            _ => return None,
        };
        self.missions.book.shelf(id)?;
        let dir = self.missions.store.as_ref()?.dir().to_path_buf();
        let events = history
            .then(|| {
                self.missions
                    .event_store
                    .as_ref()
                    .map(|store| store.dir().to_path_buf())
            })
            .flatten();
        Some(crate::mission_store::ShelfRead {
            id: id.clone(),
            dir,
            events,
            barrier: self
                .missions
                .event_store
                .as_ref()
                .map(|store| store.read_barrier()),
        })
    }

    fn shelved_unreadable(&self, id: &str, reason: &str) -> String {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "missions.shelved_unreadable",
            "mission": id, "message": reason,
        }));
        "mission_unavailable".into()
    }

    /// Takes shelved mission `id` back into the book for its Conductor's
    /// reopen, with its history, so the reopen and every later change go on
    /// from what its file held. Refused as the reopen would refuse the
    /// mission, so a refusal leaves it shelved.
    fn unshelve_mission(
        &mut self,
        id: &str,
        actor: &Actor,
        visiting: Option<Result<Visiting, String>>,
    ) -> Result<(), String> {
        if self.missions.book.shelf(id).is_none() {
            return Ok(());
        }
        let Some(read) = visiting else {
            return Err("mission_closed".into());
        };
        let Visiting { mission, history } =
            read.map_err(|reason| self.shelved_unreadable(id, &reason))?;
        if mission.id != id {
            return Err(self.shelved_unreadable(id, "holds another mission"));
        }
        if !mission.conductor.actor.same_identity(actor) {
            return Err("mission_not_conductor".into());
        }
        if mission.status == MissionStatus::Cancelled {
            return Err("mission_cancelled".into());
        }
        if self.missions.book.conducted_open(actor).is_some() {
            return Err("mission_already_conducting".into());
        }
        let book = &mut self.missions.book;
        book.shelved.retain(|line| line.id != id);
        let at = book.missions.partition_point(|kept| {
            crate::missions::mission_number(&kept.id) < crate::missions::mission_number(id)
        });
        book.missions.insert(at, mission);
        if let Some(log) = history {
            self.missions
                .events
                .insert(id.to_owned(), History::from(log.clone()));
            if let Some(store) = &self.missions.event_store {
                store.adopt(id, log);
            }
            self.missions.events_edit = self.missions.events_edit.wrapping_add(1);
        }
        crate::diagnostic!(json!({
            "component": "missions", "kind": "missions.unshelved", "mission": id,
        }));
        Ok(())
    }

    fn conducted_mission(
        &self,
        actor: &Actor,
        named: Option<&str>,
        changing: bool,
    ) -> Result<String, String> {
        let mission = match named {
            Some(id) => match self.missions.book.get(id) {
                Some(mission) => mission,
                // Only `show` and `reopen` read a shelved mission back.
                None => {
                    return Err(match self.missions.book.shelf(id) {
                        Some(line) if line.status == MissionStatus::Cancelled => {
                            "mission_cancelled".into()
                        }
                        Some(_) => "mission_closed".into(),
                        None => "mission_unknown".into(),
                    });
                }
            },
            None => self.missions.book.conducted(actor).ok_or("mission_none")?,
        };
        if !mission.conductor.actor.same_identity(actor) {
            return Err("mission_not_conductor".into());
        }
        if changing {
            match mission.status {
                MissionStatus::Cancelled => return Err("mission_cancelled".into()),
                MissionStatus::Done => return Err("mission_closed".into()),
                _ => {}
            }
        }
        Ok(mission.id.clone())
    }

    pub(super) fn reopen_mission(&mut self, id: &str, now: u64) -> Result<bool, String> {
        let target = self.missions.book.get(id).ok_or("mission_unknown")?;
        match target.status {
            MissionStatus::Cancelled => return Err("mission_cancelled".into()),
            MissionStatus::Planning | MissionStatus::Running => return Ok(false),
            MissionStatus::Done | MissionStatus::MergeWait => {}
        }
        // Its Conductor's commands and starts name no mission and mean the
        // one open mission it conducts, so it never has two.
        if self
            .missions
            .book
            .conducted_open(&target.conductor.actor)
            .is_some_and(|open| open.id != id)
        {
            return Err("mission_already_conducting".into());
        }
        // Work reopened can go on the same branches, so what was read in
        // main, or is being read now, is read again (B17).
        self.missions.outside.reopened(id, &self.missions.book);
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        target.status = MissionStatus::Running;
        target.closed_at_unix_ms = None;
        target.reopened_on = Some(target.node_marks());
        for node in &mut target.nodes {
            node.in_main = None;
            node.branch_ahead = None;
        }
        target.updated_at_unix_ms = now;
        Ok(true)
    }

    /// Hands the workers of `mission`, and every other child of the
    /// registration `former` that conducted it, to the registration that
    /// took it over, so the heir's retry of a start finds the worker it
    /// made, and the children's reports and watches reach the Conductor
    /// there now. It is queued before the takeover answers, so it lands
    /// before any command the heir sends next. `hide mission take` registers
    /// the heir before it asks (`Command::RegisterSelf`); a heir with no
    /// registration still is handed the mission's workers when it registers
    /// to start one (`coordination`, the reserve).
    fn adopt_mission_workers(
        &self,
        mission: &str,
        conductor: Option<&str>,
        former: Option<&str>,
    ) -> Result<(), String> {
        let Some(conductor) = conductor else {
            return Ok(());
        };
        if self.delivery_client.is_none() {
            // A runtime without a store (a test) has no workers to hand.
            return Ok(());
        }
        let (client, authority, actor) = self.hide_delivery(CONDUCTOR_ROLE)?;
        client
            .post(crate::delivery::worker::Effect::Agents {
                authority,
                actor,
                mutation: Box::new(crate::coordination::Mutation::AdoptMissionWorkers {
                    mission: mission.to_owned(),
                    conductor: conductor.to_owned(),
                    former: former.map(str::to_owned),
                }),
            })
            .inspect_err(|code| {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.adopt_failed",
                    "mission": mission, "code": code,
                }));
            })
    }

    fn conductor_of(&self, device: &str, pane: &str, actor: Actor) -> Conductor {
        let session = self
            .machine_agents(device)
            .iter()
            .find(|agent| {
                crate::address::same_pane(self.node.as_str(), device, &agent.pane_id, pane)
            })
            .and_then(|agent| {
                Some(crate::links::ledger::SessionIdentity {
                    device_id: device.to_owned(),
                    agent: hide_session::Agent::from_kind(&agent.agent_kind)?
                        .as_str()
                        .to_owned(),
                    session_id: agent.session_id.clone()?,
                })
            })
            // The pane's row can lag its session; the caller's own native
            // identity, which every mission command requires, names it too,
            // so the mission always has the session it is deleted with.
            .or_else(|| {
                Some(crate::links::ledger::SessionIdentity {
                    device_id: device.to_owned(),
                    agent: hide_session::Agent::from_kind(&actor.kind)?
                        .as_str()
                        .to_owned(),
                    session_id: actor.session.clone()?,
                })
            });
        let agent_id = self.delivery_state().ok().and_then(|ledger| {
            ledger
                .agents
                .iter()
                .rev()
                .find(|record| !record.ended && record.actor.same_identity(&actor))
                .map(|record| record.id.clone())
        });
        // The profile its pane's launch record says the session started
        // with, by the rule a child's start reads its parent's.
        let (profile, profile_unknown) = match self.caller_launch_profile(&actor, &actor.kind) {
            crate::launch::Parent::Profile(profile) => (Some(profile), false),
            crate::launch::Parent::Default | crate::launch::Parent::None => (None, false),
            crate::launch::Parent::Unknown => (None, true),
        };
        Conductor {
            actor,
            session,
            agent_id,
            profile,
            profile_unknown,
        }
    }

    /// The profile a resume of the session `payload` names by its id alone
    /// (이어받기, PRD mission-tool D-11) runs with when a mission's
    /// Conductor record names that session on that machine: the one the
    /// record kept, `None` for the built-in default (PRD agent-profiles
    /// D-27); no record that knows its profile names it is `Ok(None)`.
    /// Records that name it with different profiles answer none of them.
    pub(super) fn conductor_resume_profile(
        &self,
        payload: &super::events::AgentStartInCheckoutPayload,
    ) -> Result<Option<Option<crate::launch::LaunchProfile>>, &'static str> {
        let Some(session_id) = payload.resume_session_id.as_deref() else {
            return Ok(None);
        };
        // A resume that names its file is the file's (`SessionAccount`).
        if payload
            .resume_session_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
        {
            return Ok(None);
        }
        let Some(agent) = hide_session::Agent::from_kind(&payload.provider) else {
            return Ok(None);
        };
        let device = payload
            .device_id
            .as_deref()
            .filter(|device| !device.is_empty())
            .unwrap_or(self.node.as_str());
        let mut found: Option<&Option<crate::launch::LaunchProfile>> = None;
        // A record that does not know its Conductor's profile says nothing
        // of it; the session's machine answers then.
        for conductor in self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| &mission.conductor)
            .filter(|conductor| {
                !conductor.profile_unknown
                    && conductor.session.as_ref().is_some_and(|session| {
                        session.device_id == device
                            && session.agent == agent.as_str()
                            && session.session_id == session_id
                    })
            })
        {
            match found {
                Some(profile) if *profile != conductor.profile => {
                    return Err("agent_start.invalid_resume");
                }
                _ => found = Some(&conductor.profile),
            }
        }
        Ok(found.cloned())
    }

    /// What GitHub says now of pull request `number` in `mission`'s
    /// project, as a node remembers it; none before a current read holds it.
    pub(super) fn pull_seen(&self, mission: &Mission, number: u32) -> Option<PrSeen> {
        self.mission_pulls(mission)?
            .pull_requests
            .iter()
            .find(|pull| pull.number == number)
            .map(seen_of)
    }

    pub(crate) fn conductor_present(&self, actor: &Actor) -> bool {
        self.delivery_observations
            .get(&actor.pane_id)
            .is_some_and(|observation| observation.actor.same_identity(actor))
    }

    /// An open mission whose Conductor's pane is gone and whose Conductor's
    /// conversation now runs in another pane of the same device conducts
    /// from that pane, as `hide mission take` would make it, its workers
    /// handed over and its waiting wakes sent (PRD mission-tool D-11, B14).
    /// The conversation comes back through 이어받기, Reopen closed, a wake
    /// or the operator's own resume alike, so the move follows the session
    /// rather than the button that resumed it. A resumed conversation gets
    /// no prompt of its own, so a `resumed` wake tells it to go on.
    pub(super) fn follow_resumed_conductors(&mut self, device: &str) {
        let moves: Vec<(String, String, Actor)> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| {
                mission.status.open()
                    && mission.device_id == device
                    && !self.conductor_present(&mission.conductor.actor)
            })
            .filter_map(|mission| {
                let session = mission.conductor.session.as_ref()?;
                let row = self.machine_agents(device).iter().find(|agent| {
                    agent.session_id.as_deref() == Some(session.session_id.as_str())
                        && hide_session::Agent::from_kind(&agent.agent_kind)
                            .is_some_and(|kind| kind.as_str() == session.agent)
                })?;
                // A device row already carries its scoped id, which stands as it is.
                let pane = crate::address::named_pane_id(self.node.as_str(), device, &row.pane_id);
                let actor = self.delivery_observations.get(&pane)?.actor.clone();
                // A pane that already conducts another open mission keeps it.
                self.missions
                    .book
                    .conducted_open(&actor)
                    .is_none()
                    .then(|| (mission.id.clone(), pane, actor))
            })
            .collect();
        if moves.is_empty() {
            return;
        }
        let now = crate::delivery::worker::now();
        let mut moved = false;
        for (id, pane, actor) in moves {
            let mut conductor = self.conductor_of(device, &pane, actor);
            let agent_id = conductor.agent_id.clone();
            let Some(mission) = self.missions.book.get_mut(&id) else {
                continue;
            };
            // The conversation that came back is the one the record named:
            // a pane whose own record does not say its profile keeps what
            // the mission knew of the one the conversation started with.
            if conductor.profile_unknown && conductor.session == mission.conductor.session {
                conductor.profile = mission.conductor.profile.clone();
                conductor.profile_unknown = mission.conductor.profile_unknown;
            }
            let former = std::mem::replace(&mut mission.conductor, conductor);
            mission.updated_at_unix_ms = now;
            if let Err(code) =
                self.adopt_mission_workers(&id, agent_id.as_deref(), former.agent_id.as_deref())
            {
                // Not moved, so the next sight of the pane tries again.
                if let Some(mission) = self.missions.book.get_mut(&id) {
                    mission.conductor = former;
                }
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.conductor_resume_failed",
                    "mission": id, "pane": pane, "code": code,
                }));
                continue;
            }
            if let Some(mission) = self.missions.book.get_mut(&id) {
                record_wake(mission, String::new(), None, WakeEvent::Resumed, None, None);
            }
            self.note_conductor_arrived(&id, now);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.conductor_resumed",
                "mission": id, "pane": pane,
            }));
            moved = true;
        }
        if moved {
            self.save_missions();
            self.flush_mission_wakes();
        }
    }

    /// The admission of a spawn `actor` asks for (B4, B8, B11, B12): none
    /// when it conducts no open mission, a refusal while its plan is not
    /// approved, when the worker would run on another machine than the
    /// project's that may not run it (`admit_elsewhere`), or when it is none
    /// of the operator's worker candidates,
    /// and otherwise the mission whose cap the ledger checks.
    /// Whether the node `intent` names in `mission` is held now
    /// (`MissionNode::hold`): its worker's spawn is refused at its admission
    /// and again where its place is reserved (`delivery::worker`).
    pub(crate) fn mission_intent_held(&self, mission: &str, intent: &str) -> bool {
        self.missions
            .book
            .get(mission)
            .and_then(|mission| mission.intent_node(intent))
            .is_some_and(|node| node.hold.is_some())
    }

    pub(crate) fn mission_spawn_admission(
        &self,
        actor: &Actor,
        worker: &SpawnWorker<'_>,
        pick: &mut Option<crate::launch::Request>,
        intent: &str,
        briefed: bool,
    ) -> Result<Option<MissionAdmission>, String> {
        let machine = worker.machine;
        let Some(mission) = self.missions.book.conducted_open(actor) else {
            return Ok(None);
        };
        if mission.status == MissionStatus::Planning {
            return Err("mission_not_started".into());
        }
        let node = mission.intent_node(intent);
        // A node's worker starts on its brief, or stands at an empty prompt
        // until a person types: a letter cannot wake a session that has not
        // run its first turn (delivery.md, session_unread).
        if !briefed && node.is_some() {
            return Err("mission_brief_missing".into());
        }
        // A held node's worker waits for the hold to be lifted, as a start
        // by `hide mission node` does (`apply_patch`).
        if let Some(node) = node.filter(|node| node.hold.is_some()) {
            return Err(crate::coordination::refusal("mission_node_held", &node.key));
        }
        let worker_machine = machine.unwrap_or(actor.device_id.as_str());
        if worker_machine != mission.device_id {
            self.admit_elsewhere(mission, worker_machine, node)?;
        }
        // The candidates the mission was created with: a Settings edit
        // applies from the next mission (B48).
        if let Some(started) = self.outside_candidates(actor, worker, pick, &mission.candidates)? {
            return Err(crate::coordination::refusal(
                "mission_candidate_refused",
                &started,
            ));
        }
        Ok(Some(MissionAdmission {
            mission: mission.id.clone(),
            cap: mission.cap,
            closed: self.closed_registration_ids().into_iter().collect(),
        }))
    }

    /// Settings › Mission's candidates as `hide mission settings` names
    /// them (PRD agent-profiles-phase2 B11, B15): each row's agent item as
    /// the operator reads it (`item`, `Claude · work`) and the `--profile`
    /// value that starts it (`profile`, `default` for the built-in), from
    /// this machine's profiles, which the rows name; a row whose profile this
    /// machine no longer lists has neither and says `profile_gone`, since no
    /// worker can start as it.
    fn candidate_items(&self) -> Vec<Value> {
        let profiles = self.machine_profiles(self.node.as_str());
        self.snapshot
            .ui_state
            .conductor_settings
            .candidates
            .iter()
            .map(|candidate| {
                let mut row = json!({
                    "agent": candidate.agent,
                    "model": candidate.model,
                    "effort": candidate.effort,
                    "purpose": candidate.purpose,
                });
                let named = match &candidate.profile {
                    None => Some(None),
                    Some(id) => profiles
                        .and_then(|profiles| {
                            profiles.profiles.iter().find(|profile| {
                                profile.id == id.0
                                    && hide_agent_adapter::adapter(&candidate.agent)
                                        .is_some_and(|adapter| adapter.id == profile.kind)
                            })
                        })
                        .map(|profile| Some(profile.name.clone())),
                };
                match named {
                    Some(name) => {
                        let request = crate::launch::Request::new(
                            &candidate.agent,
                            candidate.profile.clone().zip(name.clone()),
                        );
                        row["item"] = request.shown().into();
                        row["profile"] = name.unwrap_or_else(|| "default".to_owned()).into();
                    }
                    None => row["profile_gone"] = true.into(),
                }
                row
            })
            .collect()
    }

    /// What the spawn `worker` would start, in the operator's words
    /// (`Claude · work (opus, high)`), when it is none of `candidates`;
    /// `None` when it is one, when there are none to hold it to, or when it
    /// retries a spawn that finished. The item judged is the one the spawn
    /// starts with: the one it names, else its caller's, the project's or
    /// the agent's default, never the name alone (PRD agent-profiles-phase2
    /// D-08, B13), picked once into `pick` (`spawn_request`), which the
    /// start then takes, so every admission and the start see one item.
    pub(crate) fn outside_candidates(
        &self,
        actor: &Actor,
        worker: &SpawnWorker<'_>,
        pick: &mut Option<crate::launch::Request>,
        candidates: &[crate::missions::Candidate],
    ) -> Result<Option<String>, String> {
        if candidates.is_empty() || worker.finished {
            return Ok(None);
        }
        let request = match pick {
            Some(request) => request,
            None => pick.insert(self.spawn_request(
                actor,
                worker.machine,
                worker.kind,
                worker.profile,
                worker.repo,
            )?),
        };
        if candidates
            .iter()
            .any(|candidate| candidate.admits(&request.target, worker.launch))
        {
            return Ok(None);
        }
        Ok(Some(item_words(request.shown(), worker.launch)))
    }

    /// The plan `plan` for `mission` with each worker's and checker's
    /// profile, which a plan names by the name `--profile` takes, as its id
    /// on the mission's machine, none for the built-in default (PRD
    /// agent-profiles-phase2 D-08); a name that machine does not list for
    /// that agent is refused with the node and the name
    /// (`mission_profile_unknown`). A node that started keeps the ids its
    /// workers have, and an agent Hide does not start keeps what it names,
    /// for the plan's own check to refuse the agent.
    pub(super) fn plan_profiles(
        &self,
        mission: &Mission,
        plan: &mut crate::missions::PlanInput,
    ) -> Result<(), String> {
        let profiles = self.machine_profiles(&mission.device_id);
        for node in &mut plan.nodes {
            // A node that started keeps the items its workers and checkers
            // were judged with, as `plan_candidates` holds it to nothing new:
            // each the plan names again, of the same agent, keeps the profile
            // id the book has, so a profile renamed or removed since does not
            // refuse a replan of work that ran.
            let kept = mission.node(&node.key).filter(|kept| kept.started());
            let workers = node
                .worker
                .iter_mut()
                .map(|worker| (worker, kept.and_then(|kept| kept.worker.as_ref())))
                .chain(node.checks.iter_mut().flat_map(|check| {
                    let line = kept.and_then(|kept| {
                        kept.criteria
                            .iter()
                            .find(|line| line.kind == check.kind.kind())
                    });
                    check.by.iter_mut().enumerate().map(move |(at, worker)| {
                        (
                            worker,
                            line.and_then(|line| line.checkers.get(at))
                                .map(|checker| &checker.worker),
                        )
                    })
                }));
            for (worker, before) in workers {
                if let Some(before) = before.filter(|before| {
                    hide_agent_adapter::canonical_kind(&before.agent)
                        == hide_agent_adapter::canonical_kind(&worker.agent)
                }) {
                    worker.profile.clone_from(&before.profile);
                    continue;
                }
                let Some(name) = worker.profile.take() else {
                    continue;
                };
                if hide_agent_adapter::adapter(&worker.agent)
                    .and_then(|adapter| adapter.start)
                    .is_none()
                {
                    worker.profile = Some(name);
                    continue;
                }
                let (id, _) =
                    crate::launch::chosen(&worker.agent, &name.0, profiles).map_err(|_| {
                        crate::coordination::refusal(
                            "mission_profile_unknown",
                            &format!(
                                "{}: {}",
                                node.key,
                                crate::launch::shown(&worker.agent, &name.0)
                            ),
                        )
                    })?;
                worker.profile = Some(id).filter(|id| !id.is_default());
            }
        }
        Ok(())
    }

    /// Refuses a plan whose work still to start none of `mission`'s
    /// candidates admits (`mission_candidate_refused`, naming the node and
    /// the item, as a spawn's refusal does): the worker and the checkers of
    /// each node not yet started, each judged as a start of it would be
    /// (`Worker::admitted_by`, PRD agent-profiles-phase2 D-08). A node that
    /// started had its workers judged when they were spawned, and a mission
    /// with no candidates holds none to them.
    pub(super) fn plan_candidates(
        &self,
        mission: &Mission,
        plan: &crate::missions::PlanInput,
    ) -> Result<(), String> {
        if mission.candidates.is_empty() {
            return Ok(());
        }
        let profiles = self.machine_profiles(&mission.device_id);
        for node in &plan.nodes {
            if mission.node(&node.key).is_some_and(MissionNode::started) {
                continue;
            }
            let workers = node
                .worker
                .iter()
                .chain(node.checks.iter().flat_map(|check| &check.by));
            for worker in workers {
                if mission
                    .candidates
                    .iter()
                    .any(|candidate| worker.admitted_by(candidate))
                {
                    continue;
                }
                let target = worker.target();
                let request = crate::launch::Request::new(
                    &worker.agent,
                    crate::launch::listed(&worker.agent, &target.profile, profiles),
                );
                let launch = hide_agent_adapter::LaunchChoice {
                    model: worker.model.clone(),
                    effort: worker.effort.clone(),
                };
                return Err(crate::coordination::refusal(
                    "mission_candidate_refused",
                    &format!("{}: {}", node.key, item_words(request.shown(), &launch)),
                ));
            }
        }
        Ok(())
    }

    /// Names a node's worker items in its view as `hide mission show` reads
    /// it (`shown`, the JSON of `view`): its launched worker and its planned
    /// one each get `item`, the agent with its profile's name now (the name
    /// then for a launch whose profile is gone, `profile_gone` for a planned
    /// one), and `model_label`, the name the agent shows for its model in
    /// that item's list as Hide last read it, where it differs. The screen
    /// names them from the snapshot's own lists, so the published view
    /// carries neither and is never made again for a list's read.
    fn name_worker_items(&self, mission: &Mission, view: &NodeView, shown: &mut Value) {
        let machine = view.machine.as_deref().unwrap_or(&mission.device_id);
        if let Some(launched) = &view.launched {
            let profile = launched.profile.as_ref().map(|profile| {
                crate::launch::listed(&launched.agent, &profile.id, self.machine_profiles(machine))
                    .unwrap_or_else(|| (profile.id.clone(), profile.name.clone()))
            });
            let id = profile
                .as_ref()
                .map_or_else(hide_agent_adapter::ProfileId::default_profile, |(id, _)| {
                    id.clone()
                });
            let row = &mut shown["launched"];
            row["item"] = crate::launch::Request::new(&launched.agent, profile)
                .shown()
                .into();
            if let Some(label) = launched
                .model
                .as_deref()
                .and_then(|model| self.model_label(machine, &launched.agent, &id, model))
            {
                row["model_label"] = label.into();
            }
        }
        if let Some(worker) = &view.worker {
            let profiles = self.machine_profiles(&mission.device_id);
            let target = worker.target();
            let listed = crate::launch::listed(&worker.agent, &target.profile, profiles);
            let row = &mut shown["worker"];
            if listed.is_none() && !target.profile.is_default() {
                row["profile_gone"] = true.into();
            }
            row["item"] = crate::launch::Request::new(&worker.agent, listed)
                .shown()
                .into();
            if let Some(label) = worker.model.as_deref().and_then(|model| {
                self.model_label(&mission.device_id, &worker.agent, &target.profile, model)
            }) {
                row["model_label"] = label.into();
            }
        }
    }

    /// The item a spawn of a node's worker starts with when it names no
    /// `--profile` (PRD agent-profiles-phase2 D-02): the one the plan names
    /// for that node's worker, the built-in default for none, when the spawn
    /// starts that agent on the mission's machine, whose profiles the plan
    /// names; otherwise none, and the start picks as any spawn does. A
    /// profile the machine no longer lists is refused (`profile_gone`), so
    /// the worker never starts in another account than the plan shows.
    pub(crate) fn planned_request(
        &self,
        actor: &Actor,
        machine: Option<&str>,
        kind: &str,
        intent: &str,
    ) -> Result<Option<crate::launch::Request>, String> {
        let Some(mission) = self.missions.book.conducted_open(actor) else {
            return Ok(None);
        };
        let Some(worker) = mission
            .intent_node(intent)
            .and_then(|node| node.worker.as_ref())
        else {
            return Ok(None);
        };
        if hide_agent_adapter::canonical_kind(&worker.agent)
            != hide_agent_adapter::canonical_kind(kind)
            || machine.unwrap_or(&actor.device_id) != mission.device_id
        {
            return Ok(None);
        }
        let target = worker.target();
        if target.profile.is_default() {
            return Ok(Some(crate::launch::Request::new(kind, None)));
        }
        crate::launch::listed(
            kind,
            &target.profile,
            self.machine_profiles(&mission.device_id),
        )
        .map(|named| Some(crate::launch::Request::new(kind, Some(named))))
        .ok_or_else(|| {
            format!(
                "{}: the plan starts this worker with a profile this machine no longer lists; replan the node with a profile hide agent catalog lists",
                crate::launch::PROFILE_GONE
            )
        })
    }

    /// Whether a worker or checker of `mission` may start on `machine`,
    /// another machine than the project's: the node its intent names, when
    /// it names one, is a pull-request change (`MissionNode::runs_elsewhere`),
    /// the machine answers, and it holds a project of the mission's GitHub
    /// repository, compared by the id GitHub gives it, which a rename keeps.
    /// A checker's intent names no node of its own; the node is judged when
    /// it is bound (`hide mission node`), which asks this same rule.
    fn admit_elsewhere(
        &self,
        mission: &Mission,
        machine: &str,
        node: Option<&MissionNode>,
    ) -> Result<(), String> {
        if let Some(node) = node.filter(|node| !node.runs_elsewhere()) {
            return Err(crate::coordination::refusal(
                "mission_machine_node_refused",
                &node.key,
            ));
        }
        if !self.device_answers(machine) {
            return Err("mission_machine_unavailable".into());
        }
        if self.projects_loading(machine) {
            return Err("mission_projects_loading".into());
        }
        let repository = self
            .github_project_on(&mission.device_id, &mission.project)
            .and_then(|project| project.issues.repository_id.as_deref());
        let held = repository.is_some_and(|id| {
            self.github_projects_on(machine)
                .any(|project| project.issues.repository_id.as_deref() == Some(id))
        });
        if !held {
            // A GitHub read that failed says why the repository is unknown,
            // so the agent can tell the operator what to fix rather than
            // wait for a read that keeps failing.
            let unread = self
                .github_project_on(&mission.device_id, &mission.project)
                .into_iter()
                .chain(self.github_projects_on(machine))
                .filter(|project| project.issues.repository_id.is_none())
                .find_map(|project| project.status.failure_category)
                .and_then(crate::github::unread_action);
            return Err(match unread {
                Some(action) => {
                    crate::coordination::refusal("mission_machine_no_repository", &action)
                }
                None => "mission_machine_no_repository".into(),
            });
        }
        Ok(())
    }

    /// The machine registration `id` runs on, when the ledger has it on
    /// another machine than `mission`'s. A ledger Hide could not read is its
    /// own refusal, since it cannot say where the agent runs.
    fn registered_elsewhere(&self, mission: &Mission, id: &str) -> Result<Option<String>, String> {
        Ok(self
            .delivery_state()?
            .agents
            .iter()
            .find(|record| record.id == id && record.machine != mission.device_id)
            .map(|record| record.machine.clone()))
    }

    /// The missions whose Conductor's session is one of `sessions`
    /// (`(machine, native session id)`) go with it (D-22, B47).
    pub(crate) fn drop_missions_of(&mut self, sessions: &[(String, String)]) -> bool {
        let before = self.missions.book.missions.len();
        let mut gone = Vec::new();
        self.missions.book.missions.retain(|mission| {
            let keep = !mission.conductor.session.as_ref().is_some_and(|session| {
                sessions
                    .iter()
                    .any(|(machine, id)| session.device_id == *machine && session.session_id == *id)
            });
            if !keep {
                gone.push(mission.id.clone());
            }
            keep
        });
        // Shelved missions keep the same machine/session identity in the index.
        let shelved = self.missions.book.shelved.len();
        self.missions.book.shelved.retain(|line| {
            let keep = !line.conductor.session.as_ref().is_some_and(|session| {
                sessions
                    .iter()
                    .any(|(machine, id)| session.device_id == *machine && session.session_id == *id)
            });
            if !keep {
                gone.push(line.id.clone());
            }
            keep
        });
        let dropped =
            before - self.missions.book.missions.len() + shelved - self.missions.book.shelved.len();
        if dropped == 0 {
            return false;
        }
        // The history goes with its mission (D-22).
        self.forget_mission_events(&gone);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "missions.deleted_with_session", "count": dropped,
        }));
        // A slot card of a mission that is gone would draw nothing, and the
        // mission screen's detail of it sends nothing.
        let known: Vec<String> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| mission.id.clone())
            .collect();
        self.drop_detail_items(|item| match item {
            DetailItem::Mission { mission } => known.contains(mission),
            _ => true,
        });
        if self
            .missions
            .shown
            .as_ref()
            .is_some_and(|shown| gone.contains(shown))
        {
            self.missions.shown = None;
        }
        self.save_missions();
        true
    }

    /// Compares a new GitHub answer with the facts each mission's pull
    /// request was last seen with, and records a wake for every change that
    /// is its Conductor's to act on (D-39, B36): a merge, a review asking
    /// for changes, an open one's CI ending on its head, each result once
    /// per head, the first answer's included. A done mission reopens with a
    /// review or a CI result, unless its Conductor conducts another open
    /// mission then, and
    /// its merge wakes no one; a cancelled one is not woken (D-51). A stale
    /// read wakes nothing.
    /// Returns the intents of the wakes it records; each is sent as soon as
    /// the Conductor is there and stays recorded until it is.
    pub(crate) fn note_mission_pulls(
        &mut self,
        device: &str,
        github: &crate::model::GithubSnapshot,
    ) -> Vec<String> {
        self.mission_pulls_moved();
        let now = super::memory::now_ms();
        let mut recorded = Vec::new();
        let mut reopened = Vec::new();
        // Missions a merge finished, with what ending their decision records.
        let mut ended = Vec::new();
        let mut changed = false;
        for index in 0..self.missions.book.missions.len() {
            let mission = &self.missions.book.missions[index];
            if mission.device_id != device {
                continue;
            }
            let Some(project) = github.project(&mission.project) else {
                continue;
            };
            if !project.pulls_current() {
                continue;
            }
            // `pr_seen` is the record every landing reads (`mission_landing`),
            // so it follows each fresh list a cancelled mission's map draws
            // too; a cancelled mission is woken, read for CI and closed by
            // nothing (D-51).
            if mission.status == MissionStatus::Cancelled {
                changed |= refresh_seen(&mut self.missions.book.missions[index], project, now);
                continue;
            }
            let (ci_changed, spent) = super::mission_criteria::read_mission_ci(
                &mut self.missions.book.missions[index],
                project,
                now,
            );
            if ci_changed {
                changed = true;
                let mission = &mut self.missions.book.missions[index];
                let mut moved = Vec::new();
                for at in spent {
                    // A CI failing again on the head its rerun was for sends
                    // the node back to work (B13).
                    let node = &mission.nodes[at];
                    if !super::mission_criteria::back_to_work(node.status) {
                        continue;
                    }
                    let patch = crate::missions::NodePatch {
                        status: Some(NodeStatus::Working),
                        ..crate::missions::NodePatch::default()
                    };
                    if let Ok(next) = crate::missions::apply_patch(node, &patch, now) {
                        moved.extend(super::mission_events::node_events(node, &next, now));
                        mission.nodes[at] = next;
                    }
                }
                mission.updated_at_unix_ms = now;
                let id = mission.id.clone();
                self.stage_mission_events(&id, moved);
            }
            let mission = &self.missions.book.missions[index];
            let mut events = Vec::new();
            let mut merged = Vec::new();
            let mut seen_updates = Vec::new();
            for (node_index, node) in mission.nodes.iter().enumerate() {
                for &number in &node.prs {
                    let Some(pull) = project
                        .pull_requests
                        .iter()
                        .find(|pull| pull.number == number)
                    else {
                        continue;
                    };
                    let mut now_seen = seen_of(pull);
                    if let Some(before) = node.seen(number) {
                        keep_told(&mut now_seen, before);
                    }
                    match node.seen(number) {
                        // The first answer about a pull request is where its
                        // wakes start from; it reports nothing of its own.
                        None => {}
                        Some(before) => {
                            if now_seen.merged && !before.merged {
                                // Keyed by the fact, so a read again after a
                                // lost save writes it once (B27).
                                let mut event =
                                    super::mission_events::merged_event(node, number, now);
                                event.id =
                                    Some(format!("{}@{}:merged:{number}", node.key, node.round));
                                merged.push(event);
                                events.push(wake_about(
                                    node.key.clone(),
                                    Some(number),
                                    WakeEvent::Merged,
                                    None,
                                    None,
                                ));
                            } else if !now_seen.merged
                                && now_seen.changes_requested
                                && !before.changes_requested
                            {
                                // Changes asked for again on one head are
                                // news again: GitHub's time of the pull
                                // request tells the asks apart.
                                let key = format!(
                                    "changes_requested:{}:{number}:{}:{}",
                                    node.key,
                                    now_seen
                                        .head
                                        .as_deref()
                                        .map_or("-", |head| head.get(..12).unwrap_or(head)),
                                    pull.updated_at_unix_ms.unwrap_or_default()
                                );
                                events.push(wake_about(
                                    node.key.clone(),
                                    Some(number),
                                    WakeEvent::ChangesRequested,
                                    now_seen.head.clone(),
                                    Some(key),
                                ));
                            }
                        }
                    }
                    // An open pull request's CI result not yet told on its
                    // head is told, the first answer's too; a new head is
                    // told by its result, which names its commit. It counts
                    // as told once its letter is recorded (`tell_ci`).
                    if let Some(result) = ci_of(pull)
                        && !now_seen.ci_told.contains(&result)
                    {
                        let mut wake = wake_about(
                            node.key.clone(),
                            Some(number),
                            WakeEvent::Ci,
                            now_seen.head.clone(),
                            None,
                        );
                        wake.ci = Some(ci_news(pull, result));
                        events.push(wake);
                    }
                    // A node a book from before push times kept starts its
                    // push clock on this read.
                    if node.seen(number) != Some(&now_seen)
                        || (node.pushed_at_unix_ms.is_none() && now_seen.head.is_some())
                    {
                        seen_updates.push((node_index, now_seen));
                    }
                }
            }
            // With nothing new from GitHub, the last answers still decide a
            // waiting stack, as when it was merged before the wait began.
            let closes = |runtime: &Self, mission: &Mission| {
                mission.status == MissionStatus::MergeWait
                    && stack_settled(mission)
                    && runtime.close_held(mission, Some(project)).is_none()
            };
            if seen_updates.is_empty() && events.is_empty() && !closes(self, mission) {
                continue;
            }
            changed = true;
            let id = mission.id.clone();
            self.stage_mission_events(&id, merged);
            let mission = &self.missions.book.missions[index];
            let done = mission.status == MissionStatus::Done;
            // A done mission's merge is the end it was waiting for, not work
            // for its Conductor (B41).
            if done {
                events.retain(|wake| wake.event != WakeEvent::Merged);
            }
            // A Conductor conducts one open mission at a time, so a change
            // to a done one while it conducts another wakes it with the
            // news and leaves the mission done.
            let reopen = done
                && !events.is_empty()
                && self
                    .missions
                    .book
                    .conducted_open(&mission.conductor.actor)
                    .is_none();
            let mission = &mut self.missions.book.missions[index];
            for (node_index, seen) in seen_updates {
                note_seen(&mut mission.nodes[node_index], seen, now);
            }
            // A mission waiting on the operator's merges is done once every
            // pull request of its stack is merged, whether or not a Conductor
            // is there to say so (B41), and only while every done node's
            // lines still hold (B10): a commit no check ran on keeps it
            // waiting, for its Conductor to see.
            let closes = closes(self, &self.missions.book.missions[index]);
            let mission = &mut self.missions.book.missions[index];
            if closes {
                mission.status = MissionStatus::Done;
                mission.closed_at_unix_ms = Some(now);
                mission.reopened_on = None;
                mission.updated_at_unix_ms = now;
                ended.push((
                    mission.id.clone(),
                    super::mission_events::end_decision(mission, now),
                ));
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.closed_by_merge", "mission": mission.id,
                }));
            }
            if events.is_empty() {
                continue;
            }
            if reopen {
                reopened.push(mission.id.clone());
            }
            for wake in events {
                let told = wake
                    .ci
                    .as_ref()
                    .map(|ci| (wake.node.clone(), wake.pr, ci.result));
                // A letter the full queue refused is not told, so the next
                // answer with room records it (engineering #15).
                let Some(intent) = record(mission, wake) else {
                    continue;
                };
                recorded.push(intent);
                if let Some((node, Some(number), result)) = told {
                    tell_ci(mission, &node, number, result);
                }
            }
        }
        for (id, events) in ended {
            self.missions.answers_held.remove(&id);
            self.stage_mission_events(&id, events);
        }
        for id in reopened {
            // Two done missions of one Conductor changed in one read: the
            // first opens and the second stays done.
            if self.reopen_mission(&id, now) == Ok(true) {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.reopened_by_pull", "mission": id,
                }));
            }
        }
        // One save keeps every change of this read with its events (B27).
        if changed {
            self.save_missions();
        }
        // The outside nodes follow the same answer (B19, B20).
        for project in self.outside_projects(device) {
            self.follow_outside_listed(device, &project, github.project(&project));
        }
        self.flush_mission_wakes();
        recorded
    }

    /// The recorded wakes whose Conductor is there to read them, at most
    /// one queue's worth, leaving out those this sending already tried.
    fn ready_wakes(&self, tried: &[String]) -> Vec<Wake> {
        let mut out = Vec::new();
        let ledger = self.delivery_state().ok();
        for mission in &self.missions.book.missions {
            if mission.status == MissionStatus::Cancelled || mission.wakes.is_empty() {
                continue;
            }
            // No Conductor to wake: its wakes wait for the next one, and the
            // change is in what it reads with `hide mission show` (B44).
            let conductor = self
                .delivery_observations
                .get(&mission.conductor.actor.pane_id)
                .filter(|observation| observation.actor.same_identity(&mission.conductor.actor));
            for wake in &mission.wakes {
                let target = match &wake.to {
                    None => conductor.cloned(),
                    Some(agent) => ledger
                        .as_deref()
                        .and_then(|ledger| self.worker_target(ledger, agent)),
                };
                let Some(target) = target else {
                    continue;
                };
                let intent = wake_intent(mission, wake);
                if tried.contains(&intent) {
                    continue;
                }
                if out.len() >= WAKE_QUEUE_LIMIT {
                    return out;
                }
                out.push(Wake {
                    mission: mission.id.clone(),
                    seq: wake.seq,
                    target,
                    intent,
                    body: wake_body(mission, wake),
                    // A worker's letter asks it to act, so a sleeping one is
                    // woken by it (D-13); the Conductor's needs no answer.
                    kind: if wake.to.is_some() {
                        "request"
                    } else {
                        "progress"
                    },
                });
            }
        }
        out
    }

    /// Where a letter to the worker registration `agent` goes now: its pane
    /// while it runs there, or the agent asleep, which a request wakes. None
    /// while neither holds; its wake then waits.
    fn worker_target(
        &self,
        ledger: &crate::delivery::ledger::Ledger,
        agent: &str,
    ) -> Option<super::delivery::Observation> {
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == agent && !record.ended)?;
        self.delivery_observations
            .get(&record.actor.pane_id)
            .filter(|observation| observation.actor.same_identity(&record.actor))
            .cloned()
            .or_else(|| self.sleeping_target(ledger, Some(&record.actor), &record.actor.pane_id))
    }

    /// Sends the recorded wakes whose Conductor is there, from one thread
    /// at a time; a wake leaves the record only once its letter is in the
    /// Conductor's inbox, so one that fails is tried again on the next
    /// GitHub read or takeover.
    pub(super) fn flush_mission_wakes(&mut self) {
        if self.missions.waking {
            self.missions.rewake = true;
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            // A runtime without a worker (a test) sends nothing.
            return;
        };
        let batch = self.ready_wakes(&[]);
        if batch.is_empty() {
            return;
        }
        let (client, authority, actor) = match self.hide_delivery(CONDUCTOR_ROLE) {
            Ok(delivery) => delivery,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.wake_failed",
                    "count": batch.len(), "code": code,
                }));
                return;
            }
        };
        self.missions.waking = true;
        self.missions.rewake = false;
        let spawned = std::thread::Builder::new()
            .name("hide-mission-wakes".to_owned())
            .spawn(move || {
                // A send that panics must not leave the record thinking a
                // sending still runs, or no wake would be sent again.
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if std::thread::panicking()
                            && let Some(runtime) = self.0.upgrade()
                        {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.missions.waking = false;
                        }
                    }
                }
                let _ending = Ending(context.runtime.clone());
                let mut batch = batch;
                let mut tried = Vec::new();
                loop {
                    let mut sent = Vec::new();
                    let mut refused = Vec::new();
                    for wake in batch.drain(..) {
                        tried.push(wake.intent.clone());
                        let prepared = crate::delivery::worker::Prepared::new(
                            client.clone(),
                            crate::delivery::worker::Authority {
                                caller: authority.caller.clone(),
                                context: authority.context.clone(),
                                also_reads: Vec::new(),
                            },
                            actor.clone(),
                            Some(wake.target.clone()),
                            crate::delivery::Command::Send {
                                target: wake.target.actor.pane_id.clone(),
                                intent: wake.intent.clone(),
                                body: wake.body,
                                kind: wake.kind.into(),
                                now: false,
                                force: false,
                            },
                        );
                        let worker = wake.kind == "request";
                        match prepared.run(WAKE_CALL) {
                            Ok(_) => sent.push((wake.mission, wake.seq)),
                            Err(code) => {
                                crate::diagnostic!(json!({
                                    "component": "missions", "kind": "mission.wake_failed",
                                    "intent": wake.intent, "code": code,
                                }));
                                // A worker's letter refused is raised to the
                                // Conductor rather than tried again (B10).
                                if worker {
                                    refused.push((wake.mission, wake.seq));
                                }
                            }
                        }
                    }
                    let Some(runtime) = context.runtime.upgrade() else {
                        return;
                    };
                    let mut guard = runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                    guard.mission_wakes_sent(&sent);
                    guard.mission_wakes_refused(&refused);
                    if guard.missions.rewake {
                        guard.missions.rewake = false;
                        batch = guard.ready_wakes(&tried);
                    }
                    if batch.is_empty() {
                        guard.missions.waking = false;
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            self.missions.waking = false;
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.wake_not_started",
                "message": error.to_string(),
            }));
        }
    }

    /// Takes the wakes whose letters are in their Conductor's inbox out of
    /// the record.
    pub(crate) fn mission_wakes_sent(&mut self, sent: &[(String, u64)]) {
        let mut changed = false;
        for (id, seq) in sent {
            if let Some(mission) = self.missions.book.get_mut(id) {
                let before = mission.wakes.len();
                mission.wakes.retain(|wake| wake.seq != *seq);
                changed |= mission.wakes.len() != before;
            }
        }
        if changed {
            self.save_missions();
        }
    }

    /// Settings › Mission (D-31, B48): a value checked before it is kept;
    /// a mission already planned keeps its own. The cleanup switch applies
    /// to every mission: turned on, the merged worktrees are looked at now.
    pub(super) fn set_conductor_settings(&mut self, settings: ConductorSettings) -> bool {
        let settings = match settings.edited() {
            Ok(settings) => settings,
            Err(code) => {
                self.set_error(&code, "Mission settings were not saved", false);
                return true;
            }
        };
        if self.snapshot.ui_state.conductor_settings == settings {
            return false;
        }
        if settings.clean_merged && !self.snapshot.ui_state.conductor_settings.clean_merged {
            self.missions.cleanup.again = true;
        }
        self.snapshot.ui_state.conductor_settings = settings;
        self.persist_ui_state();
        true
    }

    /// The view of one mission as the map draws it.
    pub(super) fn mission_view_of(&self, id: &str) -> Option<MissionView> {
        let mission = self.missions.book.get(id)?;
        let ledger = self.delivery_state().ok();
        let agents = worker_records(ledger.as_deref(), std::iter::once(mission));
        let sleeping = self.sleeping_registrations(agents.values().copied());
        Some(self.view_mission(mission, &agents, &sleeping, ledger.as_deref()))
    }

    /// The view of mission `id` drawn from the registrations `caller` read
    /// (`MissionCaller::of`), so a command that read them for its caller answers
    /// with no second pass over the ledger's agents.
    pub(super) fn mission_view_for(
        &self,
        id: &str,
        caller: &MissionCaller<'_>,
    ) -> Option<MissionView> {
        let mission = self.missions.book.get(id)?;
        let agents = caller.records_of(mission);
        let sleeping = self.sleeping_registrations(agents.values().copied());
        Some(self.view_mission(mission, &agents, &sleeping, caller.ledger))
    }

    /// The letters to an open mission's Conductor that only the operator can
    /// let through (`delivery::held::letters_held`), and when the next one held
    /// for that comes of age.
    fn conductor_letters(
        &self,
        mission: &Mission,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        now: u64,
    ) -> (Option<crate::delivery::held::HeldLetters>, Option<u64>) {
        match ledger {
            Some(ledger) if mission.status.open() => crate::delivery::held::letters_held(
                ledger,
                &self.delivery_holds,
                &mission.conductor.actor,
                now,
            ),
            _ => (None, None),
        }
    }

    /// What `ViewsMadeFrom::rows_and_holds` compares while a mission runs,
    /// zero with none running: each open mission's workers' and checkers'
    /// activity as a node's `work` reads it (`worker_activity`), and the
    /// doorbell's holds by the operator's part in them, which a Conductor's
    /// held letters read. Rows no mission holds are not read, so an
    /// unrelated agent's turn rebuilds no mission; one pass over the
    /// missions' registrations and the holds, both bounded by the ledger.
    pub(super) fn rows_and_holds_digest(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut open = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .peekable();
        if open.peek().is_none() {
            return 0;
        }
        let records = worker_records(ledger, open);
        let mut workers: Vec<(&str, Option<&str>)> = records
            .iter()
            .map(|(id, record)| (*id, self.worker_activity(record)))
            .collect();
        workers.sort_unstable();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        workers.hash(&mut hasher);
        for (letter, hold) in &self.delivery_holds {
            (letter, crate::delivery::held::held_for(*hold)).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// A mission registration's agent row activity, as its node's `work`
    /// reads it: none while its machine does not answer, since a device
    /// that dropped keeps its last rows and they no longer say what the
    /// agent does (a row Hide cannot see says nothing).
    fn worker_activity(&self, record: &crate::coordination::AgentRecord) -> Option<&str> {
        if !self.device_answers(&record.machine) {
            return None;
        }
        self.agent_row(&record.pane_on(self.node.as_str()))
            .map(|row| row.activity.as_str())
    }

    /// The mission's device does not answer, so its map shows the last
    /// value (B20).
    pub(super) fn mission_stale(&self, mission: &Mission) -> bool {
        !self.device_answers(&mission.device_id)
    }

    fn view_mission(
        &self,
        mission: &Mission,
        agents: &HashMap<&str, &crate::coordination::AgentRecord>,
        sleeping: &crate::coordination::RegistrationFacts,
        ledger: Option<&crate::delivery::ledger::Ledger>,
    ) -> MissionView {
        let github = self.github_project_on(&mission.device_id, &mission.project);
        let blocked = mission.blocked();
        let conductor_present = self.conductor_present(&mission.conductor.actor);
        let nodes = mission
            .nodes
            .iter()
            .map(|node| {
                let record = node.agent_id.as_deref().and_then(|id| agents.get(id));
                let pull_of = |number: u32| {
                    github?
                        .pull_requests
                        .iter()
                        .find(|pull| pull.number == number)
                };
                let pull = node.current_pr().and_then(pull_of);
                let pr_state = pull
                    .map(|pull| PrState::of(pull.badge, pull.is_draft, pull.checks, pull.review));
                let prs = node
                    .prs
                    .iter()
                    .map(|&number| {
                        let pull = pull_of(number);
                        NodePrView {
                            number,
                            state: pull.map(|pull| {
                                PrState::of(pull.badge, pull.is_draft, pull.checks, pull.review)
                            }),
                            url: pull.map(|pull| pull.url.clone()),
                        }
                    })
                    .collect();
                let mut criteria: Vec<Criterion> = if node.criteria.is_empty() {
                    Vec::new()
                } else {
                    self.shown_criteria(mission, node)
                };
                for checker in criteria.iter_mut().flat_map(|line| &mut line.checkers) {
                    let record = checker.agent_id.as_deref().and_then(|id| agents.get(id));
                    checker.pane_id = record.map(|record| record.pane_on(self.node.as_str()));
                    checker.machine = record.map(|record| record.machine.clone());
                }
                let count = |pick: fn(CriterionState) -> bool| {
                    u32::try_from(criteria.iter().filter(|line| pick(line.state)).count())
                        .unwrap_or(u32::MAX)
                };
                let criteria_met =
                    count(|state| matches!(state, CriterionState::Pass | CriterionState::Skipped));
                let criteria_skipped = count(|state| state == CriterionState::Skipped);
                let mut view = NodeView {
                    key: node.key.clone(),
                    title: node.title.clone(),
                    kind: node.kind,
                    depends_on: node.depends_on.clone(),
                    status: node.status,
                    blocked: blocked.get(&node.key).copied(),
                    delivery: node.delivery,
                    worker: node.worker.clone(),
                    agent_id: node.agent_id.clone(),
                    agent_runtime: record.map(|record| worker_life(record, sleeping)),
                    launched: node.launched.clone().filter(|launch| {
                        node.agent_id.as_deref() == Some(launch.agent_id.as_str())
                    }),
                    pane_id: record.map(|record| record.pane_on(self.node.as_str())),
                    machine: record.map(|record| record.machine.clone()),
                    pr: node.current_pr(),
                    prs,
                    base_pr: node.base_pr,
                    branch: node.branch.clone(),
                    pr_state,
                    pr_url: pull.map(|pull| pull.url.clone()),
                    stage: node_stage(node, pr_state),
                    attempts: node.attempts,
                    line: node.line.clone(),
                    change: node.change.clone(),
                    review: node.review.clone(),
                    criteria,
                    steps: node.steps.clone(),
                    brief_bytes: node
                        .brief
                        .as_ref()
                        .map(|brief| u32::try_from(brief.len()).unwrap_or(u32::MAX)),
                    criteria_met,
                    criteria_skipped,
                    external: node.external.clone(),
                    unread: self.missions.outside.unread(&mission.id, &node.key),
                    reopened: node.reopened,
                    started_at_unix_ms: node.started_at_unix_ms,
                    finished_at_unix_ms: node.finished_at_unix_ms,
                    commit: node.commit.clone(),
                    landing: node_landing(
                        mission,
                        node,
                        self.missions.outside.unread(&mission.id, &node.key),
                    ),
                    verdict: node.verdict,
                    checks: node.checks,
                    plan: record.and_then(|record| self.worker_plan(record)),
                    attention: node.attention.as_ref().map(|attention| {
                        let operator = attention.reason.operator_first();
                        let on_row =
                            operator && self.mission_attention_has_row(ledger, mission, node);
                        AttentionView {
                            reason: attention.reason,
                            pr: attention.pr,
                            to: if operator {
                                AttentionTo::Operator
                            } else {
                                AttentionTo::Conductor
                            },
                            since_unix_ms: attention.since_unix_ms,
                            on_row,
                            // A cancelled mission and a dropped node keep
                            // their record but ask nothing now.
                            item: operator
                                && !on_row
                                && mission.status.asks()
                                && node.status != NodeStatus::Dropped,
                            choices: attention.reason.choices(conductor_present),
                            unmet_nodes: if attention.reason
                                == crate::missions::AttentionReason::CloseRefused
                            {
                                u32::try_from(self.unmet_done_nodes(mission).len())
                                    .unwrap_or(u32::MAX)
                            } else {
                                0
                            },
                        }
                    }),
                    hide: node.hide.clone(),
                    review_fails: node.review_fails(),
                    pushed_at_unix_ms: node.pushed_at_unix_ms,
                    asks_push: node.asks_push(),
                    warnings: node.warnings.clone(),
                    hold: node.hold.clone(),
                    waiting_on: None,
                    work: None,
                    gate: self.gate_view(mission, node),
                };
                view.waiting_on = waiting_on(mission, &view);
                let activity = record.and_then(|record| self.worker_activity(record));
                view.work = node_work(mission, &view, activity);
                view
            })
            .collect();
        let closed = sleeping.closed.iter().cloned().collect::<Vec<_>>();
        let running = ledger.map_or(0, |ledger| {
            crate::coordination::mission_workers(
                ledger,
                &mission.id,
                &closed,
                crate::delivery::worker::now(),
            )
        });
        MissionView {
            id: mission.id.clone(),
            title: mission.title.clone(),
            workspace_id: mission.workspace_id.clone(),
            project: mission.project.clone(),
            checkout_path: mission.checkout_path.clone(),
            device_id: mission.device_id.clone(),
            status: mission.status,
            scope: mission.scope,
            cap: mission.cap,
            exempt: mission.exempt,
            strength: mission.strength,
            checkers: mission.checkers.clone(),
            conductor: ConductorView {
                pane_id: mission.conductor.actor.pane_id.clone(),
                name: mission.conductor.actor.name.clone(),
                agent_kind: mission.conductor.actor.kind.clone(),
                session: mission.conductor.session.clone(),
                present: conductor_present,
                takes_answers: crate::delivery::doorbell::bell_target(
                    &mission.conductor.actor.kind,
                ),
            },
            nodes,
            landing_order: landing_order(mission, |node| {
                node_landing(
                    mission,
                    node,
                    self.missions.outside.unread(&mission.id, &node.key),
                )
            }),
            stack_waiting: stack_waiting(mission),
            merge_order: mission.merge_order.clone(),
            check: mission.check.clone(),
            github: mission.github.clone(),
            decision: mission.decision.clone(),
            // A decision from before questions has nothing beside its line
            // and no time it was asked (answered under key 0).
            question: mission.decision.as_ref().and(mission.question.clone()).map(
                |mut question| {
                    // The links are the book's; the map draws `decision_views`.
                    question.views.clear();
                    question
                },
            ),
            decision_views: self.decision_views(mission),
            answer: mission.answer.as_ref().map(|answer| AnswerView {
                key: answer.key,
                text: answer.text.clone(),
                option: answer.option,
                state: if self.missions.answers_typing.contains(&mission.id) {
                    AnswerState::Sending
                } else if answer.unconfirmed {
                    AnswerState::Unconfirmed
                } else if self.missions.answers_held.contains_key(&mission.id) {
                    AnswerState::Waiting
                } else {
                    AnswerState::Sending
                },
                hold: (!answer.unconfirmed && !self.missions.answers_typing.contains(&mission.id))
                    .then(|| {
                        if self.missions.answers_forced.get(&mission.id) == Some(&answer.key) {
                            Some(AnswerHold::Forced)
                        } else {
                            self.missions.answers_held.get(&mission.id).copied()
                        }
                    })
                    .flatten(),
            }),
            // A reason holds only while its cause does: a Conductor that took
            // the mission over, or a device back, can be answered again (B39).
            answer_failed: mission
                .answer_failed
                .filter(|failure| match failure {
                    AnswerFailure::ConductorGone => {
                        !self.conductor_present(&mission.conductor.actor)
                    }
                    AnswerFailure::DeviceDisconnected => self.mission_stale(mission),
                    AnswerFailure::SendFailed => true,
                    AnswerFailure::SessionUnreadable => false,
                })
                .or_else(|| {
                    // An answer the doorbell holds on a session it cannot
                    // read says so, for as long as the read refuses.
                    (mission.answer.is_some()
                        && self.missions.answers_held.contains_key(&mission.id)
                        && self.delivery_unreadable(&mission.conductor.actor).is_some())
                    .then_some(AnswerFailure::SessionUnreadable)
                }),
            end_asked: mission.end_asked_at_unix_ms.is_some(),
            conductor_letters: self
                .conductor_letters(mission, ledger, crate::delivery::worker::now())
                .0,
            delay: mission.delay.clone(),
            delay_until_unix_ms: mission.delay.as_ref().and(mission.delay_until_unix_ms),
            report: mission.report.clone(),
            safety_net: self
                .mission_safety_net(mission, crate::delivery::worker::now())
                .map(|net| SafetyNetView {
                    conductor_present: net.conductor_present,
                    since_unix_ms: net.since_unix_ms,
                    nodes: net
                        .nodes
                        .iter()
                        .filter_map(|&at| {
                            let node = &mission.nodes[at];
                            let attention = node.attention.as_ref()?;
                            Some(HeldNodeView {
                                key: node.key.clone(),
                                title: node.title.clone(),
                                reason: attention.reason,
                                pr: attention.pr,
                                since_unix_ms: attention.since_unix_ms,
                            })
                        })
                        .collect(),
                }),
            skipped_alone: skipped_alone(mission),
            created_at_unix_ms: mission.created_at_unix_ms,
            started_at_unix_ms: mission.started_at_unix_ms,
            closed_at_unix_ms: mission.closed_at_unix_ms,
            running: u32::try_from(running).unwrap_or(u32::MAX),
            stale: self.mission_stale(mission),
            github_stale: github.is_some_and(|github| github.status.stale),
            github_read_at_unix_ms: github.and_then(|github| github.status.last_success_at_unix_ms),
        }
    }

    /// Makes the published missions what the record and the facts beside
    /// it say now; the snapshot sends them again only when they moved.
    ///
    /// It runs on every snapshot read, under the lock. With no mission it
    /// does nothing; otherwise it compares two counters, one ledger pointer,
    /// two bits per mission and the workers asleep, and makes views again
    /// only when one moved or a worker being made stops counting. While any
    /// agent sleeps, reading the workers asleep looks each mission's worker
    /// registrations up in the ledger. A closed mission's view is made again
    /// only when the book, its pull requests, its device's answer or a
    /// worker's sleep moved: its Conductor's presence and its workers' count no
    /// longer show, and a worker pane that moves after the close is followed
    /// on the next of those.
    pub(super) fn refresh_missions(&mut self) {
        let failed = self
            .missions
            .unreadable
            .as_ref()
            .map(|_| "unreadable".to_owned());
        if self.missions.book.missions.is_empty() {
            if !self.snapshot.missions.items.is_empty() || self.snapshot.missions.failed != failed {
                self.snapshot.missions = MissionsSnapshot {
                    items: Vec::new(),
                    failed,
                };
                self.publish_project_tasks();
            }
            self.missions.views = ViewsMadeFrom::default();
            return;
        }
        let ledger = self.delivery_state().ok();
        let now = crate::delivery::worker::now();
        let present: Vec<bool> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| {
                mission.status.open() && self.conductor_present(&mission.conductor.actor)
            })
            .collect();
        let stale: Vec<bool> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| self.mission_stale(mission))
            .collect();
        let unreadable: Vec<bool> = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| self.delivery_unreadable(&mission.conductor.actor).is_some())
            .collect();
        // A worker falls asleep or wakes with no other input moving, so which
        // workers sleep is an input of its own, a closed mission's as much as
        // an open one's; with no sleep record the workers are never looked up
        // (the records are read lazily).
        let sleeping = self.sleeping_registrations(std::iter::once(()).flat_map(|()| {
            worker_records(ledger.as_deref(), self.missions.book.missions.iter()).into_values()
        }));
        let laid = self.laid_panes();
        let rows_and_holds = self.rows_and_holds_digest(ledger.as_deref());
        let views_generation = self.decision_views_generation();
        let before = &self.missions.views;
        let same_ledger = match (&before.ledger, &ledger) {
            (Some(weak), Some(ledger)) => std::ptr::eq(weak.as_ptr(), Arc::as_ptr(ledger)),
            (None, None) => true,
            _ => false,
        };
        let closed_kept = before.made
            && before.book_edit == self.missions.book_edit
            && before.plans_edit == self.plans.edit
            && before.github_edit == self.missions.github_edit
            && before.stale == stale
            && before.sleeping == sleeping
            && before.laid == laid;
        if closed_kept
            && same_ledger
            && before.present == present
            && before.unreadable == unreadable
            && before.rows_and_holds == rows_and_holds
            && before.views_generation == views_generation
            && now < before.until
            && self.snapshot.missions.failed == failed
        {
            return;
        }
        // A closed mission whose node waits on the operator is drawn again
        // with the rows, which decide where it shows in Needs You (B14).
        let kept_closed =
            |mission: &Mission| closed_kept && !mission.status.open() && !operator_waits(mission);
        let made: Vec<&Mission> = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| !kept_closed(mission))
            .collect();
        let agents = worker_records(ledger.as_deref(), made.iter().copied());
        let items = self
            .missions
            .book
            .missions
            .iter()
            .map(|mission| {
                let kept = kept_closed(mission)
                    .then(|| {
                        self.snapshot
                            .missions
                            .items
                            .iter()
                            .find(|view| view.id == mission.id)
                    })
                    .flatten();
                match kept {
                    Some(view) => view.clone(),
                    None => self.view_mission(mission, &agents, &sleeping, ledger.as_deref()),
                }
            })
            .collect();
        let held_due = self
            .missions
            .book
            .missions
            .iter()
            .filter_map(|mission| self.conductor_letters(mission, ledger.as_deref(), now).1)
            .min()
            .unwrap_or(u64::MAX);
        // A safety net that comes up with nothing else moving is drawn when
        // its 20 minutes pass.
        let net_due = self
            .missions
            .book
            .missions
            .iter()
            .filter_map(|mission| self.mission_safety_net_due(mission, now))
            .min()
            .unwrap_or(u64::MAX);
        let held_due = held_due.min(net_due);
        let until = ledger.as_deref().map_or(u64::MAX, |ledger| {
            crate::coordination::mission_reservations_end(ledger, now)
        });
        let until = until.min(held_due);
        let next = MissionsSnapshot { items, failed };
        if self.snapshot.missions != next {
            self.snapshot.missions = next;
            // The 작업 비용 tasks read the missions' nodes and PRs.
            self.publish_project_tasks();
        }
        self.missions.views = ViewsMadeFrom {
            book_edit: self.missions.book_edit,
            plans_edit: self.plans.edit,
            github_edit: self.missions.github_edit,
            ledger: ledger.as_ref().map(Arc::downgrade),
            present,
            stale,
            unreadable,
            until,
            sleeping,
            laid,
            rows_and_holds,
            views_generation,
            made: true,
        };
    }
}

impl Runtime {
    /// Lays each device's missions that are their own Needs You item on that
    /// device's scope, oldest first, so its Needs You number counts them: a
    /// mission whose safety net holds, and one with a raise only the
    /// operator settles that no Conductor or worker row carries (B14). One
    /// per mission, however many nodes it holds. Run after the mission
    /// views, which decide `on_row` and the safety net.
    pub(super) fn publish_mission_asks(&mut self) -> bool {
        let mut waiting: std::collections::BTreeMap<String, Vec<(u64, String)>> =
            std::collections::BTreeMap::new();
        for mission in &self.snapshot.missions.items {
            let oldest = mission
                .nodes
                .iter()
                .filter_map(|node| node.attention.as_ref())
                .filter(|attention| attention.item)
                .map(|attention| attention.since_unix_ms)
                .chain(mission.safety_net.as_ref().map(|net| net.since_unix_ms))
                .min();
            if let Some(since) = oldest {
                waiting
                    .entry(mission.device_id.clone())
                    .or_default()
                    .push((since, mission.id.clone()));
            }
        }
        let asks = waiting
            .into_iter()
            .map(|(device, mut missions)| {
                missions.sort();
                (device, missions.into_iter().map(|(_, id)| id).collect())
            })
            .collect();
        self.agent_scope_cache.note_mission_asks(asks) && self.refresh_agent_scopes()
    }
}

/// Whether a node of a closed `mission` may wait on the operator: any raise
/// it holds, since its Conductor has finished with it and the safety net
/// holds it at once.
fn operator_waits(mission: &Mission) -> bool {
    mission.nodes.iter().any(|node| node.attention.is_some())
}

/// The newest letters `node`'s worker sent or got, newest first, at most
/// `NODE_LETTER_LIMIT` (with-hide/hide-private#552), its registration read
/// from the mission's `agents` (`worker_records`): none for a node with no
/// worker.
fn node_letters(
    ledger: &crate::delivery::ledger::Ledger,
    agents: &HashMap<&str, &crate::coordination::AgentRecord>,
    node: &MissionNode,
) -> Vec<NodeLetter> {
    let Some(worker) = node.agent_id.as_deref().and_then(|agent| agents.get(agent)) else {
        return Vec::new();
    };
    newest_letters(ledger, |letter| {
        letter.sender.same_identity(&worker.actor) || letter.recipient.same_identity(&worker.actor)
    })
}

/// The newest letters `node`'s checkers wrote to its Conductor, newest
/// first, at most `NODE_LETTER_LIMIT`: the letters its worker may read
/// (`Mission::checker_letters`), listed with their ids so the worker finds
/// them without being told (with-hide/hide-private#668), each checker's
/// registration read from the mission's `agents`.
fn node_checker_letters(
    ledger: &crate::delivery::ledger::Ledger,
    agents: &HashMap<&str, &crate::coordination::AgentRecord>,
    mission: &Mission,
    node: &MissionNode,
) -> Vec<NodeLetter> {
    let pairs: Vec<(&crate::delivery::Actor, &crate::delivery::Actor)> = mission
        .checker_letters(node)
        .filter_map(|(checker, conductor)| Some((&agents.get(checker)?.actor, conductor)))
        .collect();
    newest_letters(ledger, |letter| {
        pairs.iter().any(|(checker, conductor)| {
            letter.sender.same_identity(checker) && letter.recipient.same_identity(conductor)
        })
    })
}

/// A worker's life as its mission reads it (`coordination::life`): a
/// worker whose pane was closed is no worker to the mission, as when its
/// registration ended, until a reopen brings it back.
pub(super) fn worker_life(
    record: &crate::coordination::AgentRecord,
    sleeping: &crate::coordination::RegistrationFacts,
) -> crate::delivery::answer::Runtime {
    use crate::delivery::answer::Runtime as Life;
    match crate::coordination::life(record, sleeping) {
        Life::Closed => Life::Ended,
        life => life,
    }
}

/// The registrations `missions` name: their nodes' workers and bound
/// checkers (B29).
pub(super) fn named_agents<'m>(
    missions: impl Iterator<Item = &'m Mission>,
) -> std::collections::HashSet<&'m str> {
    missions
        .flat_map(|mission| &mission.nodes)
        .flat_map(|node| {
            node.agent_id.as_deref().into_iter().chain(
                node.criteria
                    .iter()
                    .flat_map(|line| &line.checkers)
                    .filter_map(|checker| checker.agent_id.as_deref()),
            )
        })
        .collect()
}

/// The registrations of the workers and checkers `missions` name, read in
/// one pass.
fn worker_records<'a, 'm>(
    ledger: Option<&'a crate::delivery::ledger::Ledger>,
    missions: impl Iterator<Item = &'m Mission>,
) -> HashMap<&'a str, &'a crate::coordination::AgentRecord> {
    let wanted = named_agents(missions);
    let Some(ledger) = ledger.filter(|_| !wanted.is_empty()) else {
        return HashMap::new();
    };
    ledger
        .agents
        .iter()
        .filter(|record| wanted.contains(record.id.as_str()))
        .map(|record| (record.id.as_str(), record))
        .collect()
}

/// The launch `record`'s pane says Hide started the registration's own
/// conversation with: the pane's record names that conversation, so a later
/// session in the same pane, or the same pane id on another machine, is
/// never taken for it.
fn proven_launch(
    launches: &std::collections::BTreeMap<String, crate::model::PaneLaunch>,
    node: &str,
    record: &crate::coordination::AgentRecord,
) -> Option<crate::missions::NodeLaunch> {
    let launch = launches
        .get(&record.pane_on(node))
        .filter(|launch| launch.session.as_deref() == Some(record.session.as_str()))?;
    Some(crate::missions::NodeLaunch {
        agent_id: record.id.clone(),
        agent: launch.kind.clone(),
        profile: launch
            .profile
            .as_ref()
            .map(|profile| crate::missions::NodeProfile {
                id: profile.id.clone(),
                name: profile.name.clone(),
            }),
        model: launch.choice.model.clone(),
        effort: launch.choice.effort.clone(),
    })
}

/// The node's step on its four-step bar (D-42, D-48): a pull request's is
/// the task stage the core gives every task; a direct change's follows its
/// branch and review; a node that changes no code has none (D-53).
fn node_stage(node: &MissionNode, pr_state: Option<PrState>) -> Option<u8> {
    if node.kind != NodeKind::Change {
        return None;
    }
    if let Some(state) = pr_state {
        return Some(crate::agent_state::tally::tasks::stage_of(state));
    }
    match (node.delivery, node.status) {
        (_, NodeStatus::Planned) => None,
        (Some(Delivery::Direct), NodeStatus::Done) => Some(4),
        (Some(Delivery::Direct), _) if node.branch.is_some() => Some(2),
        _ => Some(1),
    }
}

/// An item as a refusal names it, with the model and effort a start of it
/// names (`Claude · work (opus, high)`).
fn item_words(shown: String, launch: &hide_agent_adapter::LaunchChoice) -> String {
    let choice = [&launch.model, &launch.effort]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if choice.is_empty() {
        shown
    } else {
        format!("{shown} ({})", choice.join(", "))
    }
}
