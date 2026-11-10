//! A Conductor's mission (PRD software-factory-conductor D-02, D-22): one
//! agent session plans a piece of work with the operator, starts the workers
//! that carry it out, replans by what it is told, and reports. The core keeps
//! the record so that it outlives the Conductor's pane (D-11) and draws it in
//! the 보기 slot as a map (D-14).
//!
//! The record is the core's: the Conductor changes it only through the
//! pane-bound `hide mission` commands (D-23), each one event applied under
//! the runtime lock, and the file is written by its own save thread
//! (`LatestFile`). What a node should do next, how a failure is retried and
//! when to ask the operator are the Conductor's judgement (D-37); the record
//! holds only the facts the map shows and the contracts Hide enforces: one
//! Conductor per mission, no worker before the plan is approved, and the
//! mission's concurrency cap (D-45).
//!
//! Since schema 2 every node carries its finish lines
//! (`mission_criteria`, PRD mission-map-git-criteria D-09), and the core
//! refuses a node's done and a mission's close while a line is unmet
//! (D-12). A mission read from a schema 1 book is `exempt` from that gate
//! (D-19) and keeps the sentence it had as its node's one line.
//!
//! Since schema 4 a node may carry one `human` line, the place the operator
//! looks before it is done (PRD mission-human-gate D-02); a schema 3 book
//! has none and reads as it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::delivery::Actor;
use crate::latest_file::read_capped;
use crate::mission_criteria::{CheckInput, Criterion};

const SCHEMA_VERSION: u32 = 4;
/// The schema before node criteria, read and converted (D-19).
const SCHEMA_BEFORE_CRITERIA: u32 = 1;
/// The schema before a node's three states and its pull request list (PRD
/// mission-node-simple D-16).
const SCHEMA_BEFORE_SIMPLE: u32 = 2;
/// The schema before a node's human gate (PRD mission-human-gate D-02): a
/// book of it has no `human` line and reads unchanged, but a build that knows
/// only it cannot read a gate, so it reads no schema 4 book.
const SCHEMA_BEFORE_GATES: u32 = 3;
/// Answered decisions a mission keeps for a skip of a protected line; past
/// this the oldest goes and is logged.
pub const DECIDED_LIMIT: usize = 32;
/// Missions kept at once, closed ones included. A mission is deleted with
/// its Conductor's session (D-22), so a full book is refused, never evicted.
pub const MISSION_LIMIT: usize = 128;
/// Nodes in one mission's plan, the cap every dependency graph shares.
pub use crate::dag::NODE_LIMIT;
/// The highest concurrency cap a mission or the settings may name.
pub const CAP_LIMIT: u8 = 16;
/// The concurrency cap a fresh install starts with (D-08).
pub const DEFAULT_CAP: u8 = 4;
/// Worker candidates in Settings › Mission (D-31).
pub const CANDIDATE_LIMIT: usize = 16;
/// Dependencies one node may name.
const DEPENDENCY_LIMIT: usize = 16;
/// Issue numbers and paths a node may declare it touches (B6).
const TOUCH_LIMIT: usize = 32;
const TITLE_LIMIT: usize = 200;
const LINE_LIMIT: usize = 600;
/// Steps a node's plan entry names: as many as a worker's own `hide task`
/// plan takes, which starts from them.
pub const STEP_LIMIT: usize = 8;
/// A step's longest line, counted in characters: a card shows it whole.
pub const STEP_CHARS: usize = 80;
/// A record's summary, one line the map shows whole, counted in characters.
const SUMMARY_LIMIT: usize = 160;
const REPORT_LIMIT: usize = 16 * 1024;
/// Choices one decision offers the operator (PRD mission-plan-visibility
/// B32), each short enough to be a button.
pub const OPTION_LIMIT: usize = 5;
const OPTION_TEXT_LIMIT: usize = 120;
/// The longest answer the operator writes in the banner (B27), counted in
/// characters as the banner counts them.
pub const ANSWER_LIMIT: usize = 500;

/// The operator's written answer as a Conductor receives it: one line without
/// the spaces around it, not empty and at most `ANSWER_LIMIT` characters, or
/// `None`. The one judgement the core and a phone's request both make, so the
/// two cannot take different answers.
pub fn answer_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()
        && text.chars().count() <= ANSWER_LIMIT
        && !text.chars().any(char::is_control))
    .then(|| text.to_owned())
}
const READ_CAP: u64 = 8 * 1024 * 1024;
/// The size a change may bring the saved book to: under the read cap with
/// room for the file's own framing, so what is written is always read back.
const WRITE_BUDGET: usize = 7 * 1024 * 1024;

/// Where a mission stands. A cancelled mission is an end that nothing
/// reopens (D-51); a done one reopens on a further instruction or a change
/// to one of its pull requests (D-12, D-39).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Planning,
    Running,
    /// A PR-scope mission whose report is given while a stack still waits
    /// for the operator's merge (D-28, B41).
    MergeWait,
    Done,
    Cancelled,
}

impl MissionStatus {
    /// Whether the mission still runs work, which is when its cap applies.
    pub fn open(self) -> bool {
        matches!(self, Self::Planning | Self::Running | Self::MergeWait)
    }

    /// Whether a node's attention still reaches someone: an open mission's,
    /// and a done one's, so a raise it still holds keeps reaching the
    /// operator after the close; a cancelled mission asks nothing.
    pub(crate) fn asks(self) -> bool {
        self.open() || self == Self::Done
    }
}

/// How far the mission goes (D-04, D-05): up to ready pull requests or
/// branches, or through the merge into main.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionScope {
    #[default]
    Pr,
    Merge,
}

/// The kinds of node, each finished by its own rule (D-15). An external
/// node is a pull request or a branch outside the mission that Hide watches
/// until it reaches main (PRD mission-map-git-criteria D-20).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Change,
    Research,
    Verify,
    External,
}

/// What an external node watches (D-20, B19).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum External {
    Pr(u32),
    Branch(String),
}

/// How a change node reaches main (D-16).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Pr,
    Direct,
}

/// A node's state, one of three the map shows (PRD mission-node-simple
/// D-01, D-16): waiting to start, in progress (writing code, CI, review and
/// the merge wait alike), done; and left out by a replan. What waits on
/// someone is the node's `attention`, never a state of its own.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Planned,
    Working,
    Done,
    /// Left out of the mission by a replan (D-30) or by the operator.
    Dropped,
}

impl NodeStatus {
    pub(crate) fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Dropped)
    }

    /// The statuses a Conductor's `hide mission node --status` sets (D-16):
    /// a plan sets `planned`.
    pub const SET_BY_CONDUCTOR: [NodeStatus; 3] = [Self::Working, Self::Done, Self::Dropped];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Working => "working",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }
}

/// Why a node cannot start, derived from its dependencies (D-46, B51).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    UpstreamDropped,
}

/// Who a node waits on (D-02, D-05).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionTo {
    Conductor,
    Operator,
}

/// Who or what a mission node waits on now, decided once by the
/// mission's view (`runtime::missions`) so `hide mission show`'s list, its
/// node and the map's card read the same answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitingOn {
    /// Its raise, on whom it reached and why.
    Attention {
        to: AttentionTo,
        reason: AttentionReason,
    },
    /// The operator's answer to the decision that stopped it.
    Decision,
    /// A node it depends on was dropped.
    Blocked,
    /// The nodes it depends on that are not done, in its order.
    Nodes {
        keys: Vec<String>,
    },
    /// Nothing before it: it waits to be started.
    Start,
    /// Its worker: a line failed, or its first open line is the worker's.
    Worker,
    Review,
    Verify,
    Ci,
    /// The merge of its pull request.
    Merge,
    /// Every line is met and nothing waits on GitHub: the Conductor's move.
    Conductor,
}

/// Why a node waits on someone to look (PRD mission-node-simple D-04): the
/// one `!` its card carries, and the line its hover and the operator's
/// item say.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionReason {
    /// Every pull request is merged but a finish line is missing or failed
    /// (B6, B13).
    MergedUnproven,
    /// The worker ended with the node not done (B11).
    WorkerEnded,
    /// A pull request closed without its merge and no other is open (B12).
    PrClosed,
    /// The worker opened a pull request no node holds (B9).
    UnplannedPr,
    /// The Conductor recorded the node failed before this change (B26).
    ConductorFailed,
    /// A sleeping worker's wake for its pull request was refused (B10).
    WakeRefused,
    /// Hide could not close the mission with every node finished (B19).
    CloseRefused,
    /// A node's human gate waits on the operator and no Conductor stands to
    /// ask it on its row (PRD mission-human-gate D-23): the mission's own
    /// item, answered on the decision card.
    HumanGate,
}

impl AttentionReason {
    /// A reason only the operator can settle goes to them at once (B26);
    /// the others ask the Conductor first (D-05).
    pub fn operator_first(self) -> bool {
        matches!(self, Self::ConductorFailed | Self::HumanGate)
    }

    /// The operator's choices for a raise of this reason, in the order the
    /// screen draws them (B15, B16): `leave` first while a Conductor holds
    /// the mission, which hands the raise to it, or for an unplanned pull
    /// request, which it leaves outside the nodes; then what the reason
    /// takes; `drop` last, which the screen sends only on a second press. A
    /// failed Conductor's raise has no one to hand it to. A refused close
    /// has its one way out first (`done`, which closes the mission as done
    /// with the empty lines passed on the operator's word) and no `drop`,
    /// since every node's work is finished. The one list the view sends and
    /// the operator's answer is checked against.
    pub fn choices(self, conductor: bool) -> Vec<OperatorAction> {
        use OperatorAction::{Attach, Done, Drop, Leave, Resume};
        let own: &[OperatorAction] = match self {
            Self::MergedUnproven | Self::CloseRefused => &[Done],
            Self::WorkerEnded | Self::PrClosed | Self::WakeRefused => &[Drop],
            Self::UnplannedPr => &[Attach],
            Self::ConductorFailed => &[Resume, Drop],
            // Answered on the gate's decision card, never past it here.
            Self::HumanGate => return Vec::new(),
        };
        let leave = self == Self::UnplannedPr || (conductor && self != Self::ConductorFailed);
        let leave = leave.then_some(Leave).into_iter();
        if self == Self::CloseRefused {
            return own.iter().copied().chain(leave).collect();
        }
        leave.chain(own.iter().copied()).collect()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MergedUnproven => "merged_unproven",
            Self::WorkerEnded => "worker_ended",
            Self::PrClosed => "pr_closed",
            Self::UnplannedPr => "unplanned_pr",
            Self::ConductorFailed => "conductor_failed",
            Self::WakeRefused => "wake_refused",
            Self::CloseRefused => "close_refused",
            Self::HumanGate => "human_gate",
        }
    }
}

/// What waits on someone for a node (D-02, D-05): raised to the Conductor
/// at `since`, and to the operator at `operator_at` once it stood ten
/// minutes, or at once with no Conductor there.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Attention {
    pub reason: AttentionReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    /// The worker registration it is about, for a worker that ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The mission's count of raises, which names this one in its letters.
    pub key: u64,
    pub since_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_at_unix_ms: Option<u64>,
    /// The operator was told outside Needs You, because no row stood to lay
    /// it on: once per cause, kept across a restart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notified: bool,
}

impl Attention {
    /// The cause it was raised for: the same cause is raised once (B14,
    /// B27), whoever settles it.
    pub fn cause(&self) -> String {
        cause_key(self.reason, self.pr, self.agent.as_deref())
    }
}

pub(crate) fn cause_key(reason: AttentionReason, pr: Option<u32>, agent: Option<&str>) -> String {
    let mut key = reason.as_str().to_owned();
    if let Some(pr) = pr {
        key.push_str(&format!(":{pr}"));
    }
    if let Some(agent) = agent {
        key.push(':');
        key.push_str(agent);
    }
    key
}

/// Why Hide woke a sleeping worker (D-13).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixReason {
    CiFailed,
    Conflict,
    ChangesRequested,
}

impl FixReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CiFailed => "ci_failed",
            Self::Conflict => "conflict",
            Self::ChangesRequested => "changes_requested",
        }
    }
}

/// One wake Hide sent a worker, kept so the same fact wakes once (B10,
/// B27): a pull request, its reason and the head it was read on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Woken {
    pub pr: u32,
    pub why: FixReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// What a node's warning is about (with-hide/hide-private#552): a sign that
/// its worker is stuck, for its Conductor to judge whether to hand the node
/// to a fresh worker. A warning goes to the Conductor alone, never to the
/// operator, and is no attention: it waits on no one's choice.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// The review line failed `mission_health::REVIEW_FAILS_THAT_WARN` times
    /// in a row.
    ReviewFails,
    /// The node worked `mission_health::QUIET_THAT_WARNS_MS` with no push and its pull
    /// request does not just wait on others.
    NoPush,
}

impl WarningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewFails => "review_fails",
            Self::NoPush => "no_push",
        }
    }
}

/// A warning a node carries now: raised once with one letter to its
/// Conductor, and gone when its fact goes (the review passes, a new push
/// arrives, the node finishes).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeWarning {
    pub kind: WarningKind,
    /// The fact it stands on, which keys its letter: for failed reviews when
    /// the failure that reached the count was recorded, for no push the time
    /// the quiet is counted from.
    pub from_unix_ms: u64,
    /// When Hide raised it.
    pub since_unix_ms: u64,
    /// When its letter joined the Conductor's queue; none while the queue
    /// was full, so each evaluation after asks again until it is told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub told_unix_ms: Option<u64>,
}

/// What Hide itself did to a node last (D-07, D-15): the card's small Hide
/// mark and the panel's line.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HideAction {
    /// Finished the node: its pull requests and lines met.
    Done,
    /// Attached a follow-up pull request of the node's branch.
    Attached,
    /// Woke the sleeping worker about its pull request.
    Woke,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HideMark {
    pub action: HideAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<FixReason>,
    pub at_unix_ms: u64,
}

/// What the operator chose for a node that waits on them (B15, B16).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorAction {
    /// 끝으로: past the lines left open, as the operator's decision; for a
    /// refused close, 완료로 닫기, which does it for every finished node and
    /// closes the mission.
    Done,
    /// 노드 제외.
    Drop,
    /// 노드에 붙이기: the unplanned pull request joins the node.
    Attach,
    /// Conductor에게 맡기기, or 그대로 두기 with no Conductor: settles the
    /// raise and leaves the rest to the Conductor.
    Leave,
    /// 다시 진행: the node works again.
    Resume,
}

impl OperatorAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Drop => "drop",
            Self::Attach => "attach",
            Self::Leave => "leave",
            Self::Resume => "resume",
        }
    }
}

/// The agent item the plan assigns to a node or to one of its checkers
/// (D-08, D-34): the agent, its profile, its model and its effort.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    pub agent: String,
    /// The profile, by its id on the mission's machine, as a candidate names
    /// one (`Candidate::profile`); `None` for the agent's built-in default,
    /// which is left out of what is kept, so a plan of built-in defaults
    /// only is the bytes a build before it wrote and reads. A plan names it
    /// by the name `--profile` takes, which the core turns into its id
    /// (`Runtime::plan_profiles`) before the plan is applied. A build before
    /// it refuses a book that names one (`deny_unknown_fields`):
    /// `docs/INSTALL.md`, What each system supports, says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<hide_agent_adapter::ProfileId>,
    #[serde(default)]
    pub model: Option<String>,
    /// None leaves the effort to the agent's default; `effort_auto` says
    /// the Conductor chose it rather than the operator's candidate.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub effort_auto: bool,
}

/// The GitHub facts of a node's pull request as Hide last told the
/// Conductor about them, so a restart neither repeats nor misses a wake
/// (D-39, engineering #11).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrSeen {
    /// The pull request these facts are about.
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub changes_requested: bool,
    /// Closed without its merge: nothing is left to wait on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    /// The CI results the Conductor was told on `head`, each once for good
    /// (at most one of each); a new head empties it, so its results are
    /// news again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ci_told: Vec<CiResult>,
}

/// What a pull request's CI ended as on one head commit: its checks as the
/// pull request list judges them (`github::runs`), the node's `ci` line's
/// value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CiResult {
    Passed,
    /// Every run of a workflow was cancelled and none replaced it.
    Cancelled,
    Failed,
}

impl CiResult {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// The checks a CI letter names at most; past it, it counts the rest.
pub const CI_FAILED_NAMED: usize = 8;

/// A CI result as its letter tells it (`WakeEvent::Ci`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CiNews {
    pub result: CiResult,
    /// The checks that failed, by the name GitHub shows, at most
    /// `CI_FAILED_NAMED`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
    /// How many more failed than `failed` names.
    #[serde(default)]
    pub more: usize,
}

/// One change of a node's pull request the Conductor is still to be told
/// about. It stays in the record until its letter is in the Conductor's
/// inbox, so a failed send, a crash or no Conductor present delays the wake
/// and never loses it (D-39, engineering #11).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingWake {
    /// The mission's count of wakes, which names the wake in the record
    /// while its letter is sent; its letter's intent is `key`.
    pub seq: u64,
    pub node: String,
    /// The pull request the news is about; none for an outside branch.
    pub pr: Option<u32>,
    pub event: WakeEvent,
    #[serde(default)]
    pub head: Option<String>,
    /// The worker registration the letter goes to; none for the Conductor
    /// (D-13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AttentionReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<FixReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<OperatorAction>,
    /// What a `warning` wake warns of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<WarningKind>,
    /// The CI result a `ci` wake tells, on `head`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<CiNews>,
    /// The facts the wake stands on (`MissionEvent::id`'s kind of key), its
    /// letter's intent with the mission and the recipient: the same facts
    /// evaluated again, after a restart or a lost save, are the same letter
    /// (B27, engineering #11). None for a wake recorded before keys, whose
    /// intent names `seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeEvent {
    Merged,
    ChangesRequested,
    /// A new head commit, which a build before CI letters told; a book from
    /// then may still hold one waiting, sent as it was. None is recorded now:
    /// the head's CI result (`Ci`) names the commit.
    NewHead,
    /// An open pull request's CI ended on its head; `ci` says how.
    Ci,
    /// An outside node's pull request merged or its branch reached main, so
    /// the nodes waiting on it can start (B19).
    OutsideDone,
    /// An outside pull request closed without a merge (B20).
    OutsideFailed,
    /// The Conductor's own conversation came back in another pane and
    /// conducts the mission again (PRD mission-tool D-11, B14): a resumed
    /// conversation gets no prompt of its own, so this letter tells it to
    /// read the mission and go on.
    Resumed,
    /// The operator asked from the map to end the mission (`미션 종료`):
    /// the Conductor asks about its open pull requests and cancels it.
    EndAsked,
    /// A node waits on the Conductor to look (PRD mission-node-simple
    /// D-05); `reason` says why.
    Attention,
    /// The operator settled what waited on a node; `action` says how (B17).
    Resolved,
    /// Hide closed the mission with every node finished (B18).
    Closed,
    /// The operator opened the closed mission again from the map (B18).
    Reopened,
    /// A sleeping worker's pull request needs it; `why` says what (B10).
    Fix,
    /// A node shows signs its worker is stuck; `warning` says which
    /// (with-hide/hide-private#552).
    Warning,
    /// The operator asked for something to look at before answering the
    /// open question (`자료 요청`).
    MaterialAsked,
    /// The operator passed a node's human gate (PRD mission-human-gate
    /// D-11); its words are on the line.
    GatePassed,
    /// The operator's answer to a node's gate sent it back with their words
    /// as a defect: to the Conductor, and to the node's worker (`to`).
    GateFix,
}

impl WakeEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::ChangesRequested => "changes_requested",
            Self::NewHead => "new_head",
            Self::Ci => "ci",
            Self::OutsideDone => "outside_done",
            Self::OutsideFailed => "outside_failed",
            Self::Resumed => "resumed",
            Self::EndAsked => "end_asked",
            Self::Attention => "attention",
            Self::Resolved => "resolved",
            Self::Closed => "closed",
            Self::Fix => "fix",
            Self::Reopened => "reopened",
            Self::Warning => "warning",
            Self::MaterialAsked => "material_asked",
            Self::GatePassed => "gate_passed",
            Self::GateFix => "gate_fix",
        }
    }
}

/// A verify node's result (PRD mission-plan-visibility B33).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
}

/// How many of a verify node's checks passed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    pub passed: u32,
    pub total: u32,
}

/// What the Conductor asks the operator beside its question (PRD
/// mission-plan-visibility D-13, B25, B32). `key` names this question, so an
/// answer to one the Conductor has since changed is refused (B26).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Question {
    pub key: u64,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The recommended option, counted from 1 as the command names it.
    #[serde(default)]
    pub recommend: Option<u8>,
    /// The node the decision stopped.
    #[serde(default)]
    pub node: Option<String>,
    pub asked_at_unix_ms: u64,
    /// The Views the Conductor linked to this question: the ones `--show`
    /// named and the ones it opened since the question before was asked
    /// that were still open, oldest first, at most [`VIEW_LINK_LIMIT`]. The map lists them with the node's
    /// own (`MissionNode::views`) under the question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<ViewLink>,
    /// The operator asked the Conductor for something to look at before
    /// answering (`볼 자료 요청`): asked once per question.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub material_asked: bool,
}

/// The most Views one question or one node links; past it the oldest goes.
pub const VIEW_LINK_LIMIT: usize = 16;
/// The longest `decide --show` value: a View id, or a path the CLI opens as
/// a View before the core sees it.
const SHOW_LIMIT: usize = 4096;

/// A View a decision shows the operator to look at, by the checkout that
/// holds it: a link is a fact recorded when the View was opened or named,
/// never a guess from its name or its time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ViewLink {
    pub device_id: String,
    pub checkout_path: String,
    pub display_id: String,
    /// The node's attempt the View was opened in (`MissionNode::attempts`);
    /// a question's own link keeps 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt: u8,
    /// When it was opened or named, which orders a question's opened Views
    /// newest first.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub at_unix_ms: u64,
    /// On a question: the stopped node's worker, or an agent under it,
    /// opened it, where every other link is the Conductor's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worker: bool,
    /// On a question: what the View was when the decision linked it, so the
    /// card keeps the row after the View closes and opens it again from
    /// here (PRD mission-human-gate D-20). Absent on a node's or the
    /// Conductor's own link, on a link recorded before it was kept, and for
    /// a View no address can open again (an outside file's grant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<KeptView>,
}

/// The longest page or file name a link keeps; a row shows far fewer.
const KEPT_LABEL_LIMIT: usize = 256;

/// A linked View's kind, name and address, as the decision linked it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeptView {
    pub kind: crate::view_layout::DisplayKind,
    pub label: String,
    /// A file's or a diff's path, or a page's address.
    pub address: String,
    /// The Changes group a diff compares against.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub committed: bool,
    /// The agent that had it open when the decision linked it, which the
    /// View opened again is stamped with, so it is listed in that agent's
    /// 볼 것 as before; none for the operator's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<crate::view_feedback::Opener>,
}

impl KeptView {
    /// What `display`, titled `label`, can be opened again from; none for
    /// an outside file, which no address names, or an address past
    /// [`SHOW_LIMIT`].
    pub fn of(display: &crate::view_layout::Display, label: String) -> Option<Self> {
        use crate::view_layout::DisplayKind;
        let address = match display.kind {
            DisplayKind::File | DisplayKind::Diff => display.path.clone(),
            DisplayKind::Browser => display.url.clone()?,
            DisplayKind::External => return None,
        };
        if address.is_empty() || address.len() > SHOW_LIMIT {
            return None;
        }
        Some(Self {
            kind: display.kind,
            label: label.chars().take(KEPT_LABEL_LIMIT).collect(),
            address,
            committed: display.committed.unwrap_or(false),
            opener: display.opener.clone(),
        })
    }

    /// Whether `display` shows what this kept: the same document, or the
    /// same page address.
    pub fn shown_by(&self, display: &crate::view_layout::Display) -> bool {
        use crate::view_layout::DisplayKind;
        match self.kind {
            DisplayKind::Browser => {
                display.kind == DisplayKind::Browser
                    && display.url.as_deref() == Some(self.address.as_str())
            }
            DisplayKind::File | DisplayKind::Diff => {
                display.shows(&self.address, self.kind, Some(self.committed))
            }
            DisplayKind::External => false,
        }
    }
}

impl ViewLink {
    pub fn same_view(&self, other: &Self) -> bool {
        self.device_id == other.device_id
            && self.checkout_path == other.checkout_path
            && self.display_id == other.display_id
    }

    /// The name a decision card's row asks for this link by: it tells two
    /// checkouts' Views with the same display id apart without saying
    /// either path, which a phone never receives.
    pub fn token(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(
            format!(
                "{}\0{}\0{}",
                self.device_id, self.checkout_path, self.display_id
            )
            .as_bytes(),
        );
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Adds `link` to `links` as the newest, once: the same View again moves
/// to the end, and the oldest goes past [`VIEW_LINK_LIMIT`]. Whether the
/// list changed.
pub fn push_view_link(links: &mut Vec<ViewLink>, link: ViewLink) -> bool {
    if links
        .last()
        .is_some_and(|last| last.same_view(&link) && last.attempt == link.attempt)
    {
        return false;
    }
    links.retain(|known| !known.same_view(&link));
    links.push(link);
    if links.len() > VIEW_LINK_LIMIT {
        links.remove(0);
    }
    true
}

/// The operator's answer to the question with `key`, from the banner (B26):
/// kept until it is typed into the Conductor's conversation, so a restart
/// neither loses nor repeats it (engineering #11); typing it takes the
/// decision away with it (B29).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Answer {
    pub key: u64,
    pub text: String,
    /// The option it chose, counted from 1; none for a written answer.
    #[serde(default)]
    pub option: Option<u8>,
    pub at_unix_ms: u64,
    /// Read only from a book written while a typed answer stayed until the
    /// Conductor cleared its decision: reading the book settles that
    /// decision (`settle_typed_answers`), so it is never typed again.
    #[serde(default, skip_serializing)]
    pub delivered: bool,
    /// Typing it may have reached the conversation without the engine saying so:
    /// it is not typed again until the operator sends it again (B26).
    #[serde(default)]
    pub unconfirmed: bool,
}

/// Why the operator's last answer from the banner did not reach the
/// Conductor (B28, B39).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerFailure {
    /// No Conductor runs the mission to read it.
    ConductorGone,
    /// The device the mission's project is on does not answer.
    DeviceDisconnected,
    /// Typing it into the conversation failed.
    SendFailed,
    /// The answer waits on the Conductor's session, which Hide cannot read,
    /// so it cannot tell when the answer would not interrupt. Never stored:
    /// shown while the answer waits on it.
    SessionUnreadable,
}

/// Wakes a mission keeps unsent; past this the oldest is dropped and
/// logged, and the Conductor's own `hide mission show` still reads the
/// pull request as it is.
pub const PENDING_WAKE_LIMIT: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionNode {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: NodeStatus,
    #[serde(default)]
    pub delivery: Option<Delivery>,
    #[serde(default)]
    pub worker: Option<Worker>,
    /// The worker's registration in the delivery ledger, once spawned.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The node's pull requests, the current one last (D-10): the one the
    /// Conductor named and the follow-ups of its branch Hide attached. The
    /// node is done when every one is (D-11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prs: Vec<u32>,
    /// The pull request a stacked one is based on (B15).
    #[serde(default)]
    pub base_pr: Option<u32>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub attempts: u8,
    /// The Views the node's worker and the agents under it opened, each with
    /// the attempt it was opened in; a decision that stops the node lists
    /// the current attempt's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<ViewLink>,
    /// What the node does now, its conclusion or its verification result.
    #[serde(default)]
    pub line: Option<String>,
    /// What a replan changed in this node and why (B18).
    #[serde(default)]
    pub change: Option<String>,
    #[serde(default)]
    pub review: Option<String>,
    /// The node's finish lines (PRD mission-map-git-criteria D-09).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Criterion>,
    /// How the Conductor planned the node's work, a few lines it wrote with
    /// the plan: the worker's own plan starts from them and, once it
    /// exists, replaces them on the map (never a second list).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// The first prompt the node's worker was spawned on, kept when the
    /// Conductor binds the worker: the delivery ledger lets its letter go a
    /// day after it finished, and the panel shows the brief as long as the
    /// node is on the map. At most the ledger's letter size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<External>,
    /// How often a later node's defect opened it again (D-21).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reopened: u8,
    #[serde(default)]
    pub issues: Vec<u32>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub started_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_unix_ms: Option<u64>,
    /// What GitHub last said of each of `prs`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_seen: Vec<PrSeen>,
    /// A direct change's commit on main (B22, B33).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The checkout on the mission's machine the node's check command last
    /// passed or failed in: its head now is what the line is compared with,
    /// whoever works the node then (B7, D-16). Read through `check_folder`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The machine and path of the checkout on another machine the check
    /// command last ran in, in place of `folder`, which then stays empty: a
    /// build from before workers ran elsewhere does not know this field, so
    /// it reads the check as open, never a device's path as one on the
    /// mission's machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_folder: Option<(String, String)>,
    /// The commit the node's work folder was on when it was done, for a
    /// node without a pull request: the commit its check command is judged
    /// against from then on, because the folder keeps moving after a finish
    /// (the worker merges into main, later commits, the folder goes away)
    /// and a finish does not lose its proof to that. Gone once the node is
    /// not done; a node done before this field existed has none, and its
    /// check is judged against the folder as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_head: Option<String>,
    /// The proof that a direct change's branch is in main, which a mission
    /// waiting for its branches waits on (B17, B18): of that branch only,
    /// while the node stays done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_main: Option<Landed>,
    /// An outside branch's head when Hide last read it with commits main
    /// lacks: work of its own, which makes the node done once main holds
    /// it. A branch only cut from main has none (B19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_ahead: Option<String>,
    /// An outside node's read round, from its mission's `round_seq`: a new
    /// one each time it names another branch or goes back to work after it
    /// settled, so a read asked in an earlier round is dropped. Apart from
    /// `round`, the work round Hide's act keys name.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub outside_round: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Checks>,
    /// What waits on someone to look (D-02).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    /// The causes already raised once (`Attention::cause`), so a settled
    /// one is not raised again (B14, B27); at most `HANDLED_LIMIT`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handled: Vec<String>,
    /// The mark a person left by taking the node back to work from a
    /// finish (`resumed_cause`), kept apart from `handled` so a full cause
    /// list never loses it (B16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed: Option<String>,
    /// How often the node went back to work from a finish: the keys of
    /// Hide's acts on it name it, so an act of a later round is its own
    /// (`MissionEvent::id`, B27).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub round: u32,
    /// What Hide itself did to the node last (D-07, D-15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide: Option<HideMark>,
    /// The wakes Hide sent the sleeping worker (B10); at most `WOKEN_LIMIT`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub woken: Vec<Woken>,
    /// What the node's worker was proven launched with, kept with the
    /// registration it was proven for, so the map names what ran after the
    /// worker's pane is gone and never another session's launch in the same
    /// pane (`Runtime::record_node_launches`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched: Option<NodeLaunch>,
    /// When Hide read a new head on one of the node's pull requests last,
    /// the first head it read of one included: the node's last push as
    /// GitHub shows it. A book from before it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at_unix_ms: Option<u64>,
    /// When the node last went to work, from planned or from a finish: a
    /// node back at work has gone without a push only since then. A book
    /// from before it has none, and `started_at_unix_ms` stands in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worked_at_unix_ms: Option<u64>,
    /// The warnings the node carries now, at most one of each kind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<NodeWarning>,
}

/// The agent item, model and effort Hide started a node's worker with,
/// proven for one registration: its pane's launch record named the
/// registration's own conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeLaunch {
    /// The worker's registration the launch was proven for.
    pub agent_id: String,
    /// The agent kind, in the engine's spelling.
    pub agent: String,
    /// The profile it started with (PRD agent-profiles-phase2 B12); none
    /// for the built-in default, as in every launch recorded before it. A
    /// build before it reads a launch without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<NodeProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// A profile a node's worker started with: its id on the worker's machine,
/// by which the map names it as it is called now, and its name then, which
/// the map shows once the profile is gone (B23) and `hide mission show`
/// prints. Never its account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeProfile {
    pub id: hide_agent_adapter::ProfileId,
    pub name: String,
}

/// Causes a node remembers as raised; past this a new cause is refused
/// with what it would have done, and none is forgotten.
pub const HANDLED_LIMIT: usize = 128;
/// Worker wakes a node remembers, one per pull request and reason; past
/// this a new one is refused.
pub const WOKEN_LIMIT: usize = 32;

impl MissionNode {
    /// The node's current pull request: the last one it holds.
    pub fn current_pr(&self) -> Option<u32> {
        self.prs.last().copied()
    }

    /// Whether the node was started: past planned, or a planned node that
    /// already names its worker, pull request or branch, whatever its status
    /// still says.
    pub(crate) fn started(&self) -> bool {
        self.status != NodeStatus::Planned
            || self.agent_id.is_some()
            || !self.prs.is_empty()
            || self.branch.is_some()
    }

    /// The registrations the Conductor bound as checkers to the node's lines
    /// (a reviewer or a verifier), once each.
    pub(crate) fn checker_agents(&self) -> impl Iterator<Item = &str> {
        self.criteria
            .iter()
            .flat_map(|line| line.checkers.iter())
            .filter_map(|checker| checker.agent_id.as_deref())
    }

    /// The registrations bound to the node: its worker and its checkers.
    fn bound_agents(&self) -> impl Iterator<Item = &str> {
        self.agent_id
            .as_deref()
            .into_iter()
            .chain(self.checker_agents())
    }

    /// Whether the node's worker and checkers may run on another machine
    /// than the mission's: only a change that reaches main through a pull
    /// request, whose work GitHub carries back. A direct change's branch
    /// would stay on that machine, and a research or verify node answers
    /// on the mission's.
    pub fn runs_elsewhere(&self) -> bool {
        self.kind == NodeKind::Change && self.delivery == Some(Delivery::Pr)
    }

    /// The machine and path of the checkout the node's check command last
    /// ran in: `folder` on `mission_device`, or `remote_folder`.
    pub(crate) fn check_folder<'a>(
        &'a self,
        mission_device: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        match &self.remote_folder {
            Some((device, path)) => Some((device, path)),
            None => Some((mission_device, self.folder.as_deref()?)),
        }
    }

    /// Records the checkout the check command ran in, `folder` on the
    /// mission's machine and `remote_folder` on another; answers whether it
    /// is another one than before.
    pub(crate) fn set_check_folder(
        &mut self,
        mission_device: &str,
        device: &str,
        path: &str,
    ) -> bool {
        let moved = self.check_folder(mission_device) != Some((device, path));
        let remote = device != mission_device;
        self.folder = (!remote).then(|| path.to_owned());
        self.remote_folder = remote.then(|| (device.to_owned(), path.to_owned()));
        moved
    }

    /// Forgets the checkout the check command ran in.
    pub(crate) fn clear_check_folder(&mut self) {
        self.folder = None;
        self.remote_folder = None;
    }

    pub fn seen(&self, number: u32) -> Option<&PrSeen> {
        self.pr_seen.iter().find(|seen| seen.number == number)
    }

    /// Makes `number` the node's current pull request, keeping the others.
    pub(crate) fn attach_pr(&mut self, number: u32) {
        self.prs.retain(|pr| *pr != number);
        self.prs.push(number);
    }

    /// The mark a person leaves by taking the node back to work from done
    /// or dropped: Hide does not finish it again on the facts it was taken
    /// back on, only once it holds another pull request or a line is
    /// recorded again (B7, B16).
    pub(crate) fn resumed_cause(&self) -> String {
        let prs: Vec<String> = self.prs.iter().map(u32::to_string).collect();
        let lines: Vec<String> = self
            .criteria
            .iter()
            .map(|line| {
                format!(
                    "{}={}@{}",
                    line.key,
                    line.state.as_str(),
                    line.by.as_ref().map_or(0, |by| by.at_unix_ms)
                )
            })
            .collect();
        format!("{}|{}", prs.join(","), lines.join(","))
    }

    /// Whether a person took the node back to work on the facts it holds
    /// now.
    pub(crate) fn resumed_on_these_facts(&self) -> bool {
        self.resumed
            .as_deref()
            .is_some_and(|mark| mark == self.resumed_cause())
    }

    /// Remembers `cause` as raised; whether it is remembered. Past
    /// `HANDLED_LIMIT` nothing is forgotten and the new cause is not
    /// admitted, so its caller refuses what it would have done rather than
    /// do it with no memory of it (engineering #11, #15).
    #[must_use]
    pub(crate) fn note_handled(&mut self, cause: String) -> bool {
        if self.handled.contains(&cause) {
            return true;
        }
        if self.handled.len() >= HANDLED_LIMIT {
            crate::diagnostic!(serde_json::json!({
                "component": "missions", "kind": "mission.handled_full",
                "node": self.key, "cap": HANDLED_LIMIT,
            }));
            return false;
        }
        self.handled.push(cause);
        true
    }

    /// The node goes back to work from a finish at `now`: a new round for
    /// the keys of Hide's acts (B27), and the time its quiet without a push
    /// counts from.
    pub(crate) fn back_to_work(&mut self, now: u64) {
        self.round = self.round.saturating_add(1);
        self.worked_at_unix_ms = Some(now);
        self.done_head = None;
    }

    /// A change at work delivered by pull request: the node a push is asked
    /// of, whose quiet without one can warn its Conductor.
    pub fn asks_push(&self) -> bool {
        self.status == NodeStatus::Working
            && self.kind == NodeKind::Change
            && self.delivery == Some(Delivery::Pr)
    }

    /// When the node's time without a push counts from: its last push, or
    /// when it went to work if that is later. None for a node that never
    /// worked.
    pub fn quiet_since(&self) -> Option<u64> {
        let worked = self.worked_at_unix_ms.or(self.started_at_unix_ms);
        match (self.pushed_at_unix_ms, worked) {
            (Some(pushed), Some(worked)) => Some(pushed.max(worked)),
            (pushed, worked) => pushed.or(worked),
        }
    }

    /// The node's review line's failures in a row; 0 with no review line.
    pub fn review_fails(&self) -> u32 {
        self.criteria
            .iter()
            .filter(|line| line.kind == crate::mission_criteria::CriterionKind::Review)
            .map(|line| line.fails_in_a_row)
            .max()
            .unwrap_or(0)
    }
}

/// The session that conducts a mission (D-23). `actor` is the delivery
/// identity a command is checked against; `session` names the native
/// conversation, which the archive deletes the mission with (D-22).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conductor {
    pub actor: Actor,
    #[serde(default)]
    pub session: Option<crate::links::ledger::SessionIdentity>,
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The profile the session's start ran with, as its pane's launch record
    /// kept it, which 이어받기's resume of the session runs with again
    /// (PRD agent-profiles D-27); `None` for the built-in default, as for
    /// every mission recorded before profiles, unless `profile_unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::launch::LaunchProfile>,
    /// No launch record describes the session's start (a Conductor started
    /// by hand), so the account it runs in is not known: 이어받기 asks the
    /// session's machine, as a resume that names the session by id alone
    /// does, never assuming the built-in default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub profile_unknown: bool,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// A question the operator answered that named a node: the decision a skip
/// of that node's check command or CI rests on (D-13, B12).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Decided {
    pub key: u64,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Mission {
    pub id: String,
    pub title: String,
    /// The main worktree of the project the mission belongs to, on
    /// `device_id`, the key GitHub answers are read by.
    pub project: String,
    /// The catalog project the map's pull request cards open in. A core
    /// move that finds the registration gone writes `null`, which reads as
    /// none rather than making the whole book unreadable.
    #[serde(default, deserialize_with = "null_is_empty")]
    pub workspace_id: String,
    /// The checkout the mission was created in, where a takeover starts.
    #[serde(default)]
    pub checkout_path: String,
    pub device_id: String,
    pub conductor: Conductor,
    pub status: MissionStatus,
    pub scope: MissionScope,
    pub cap: u8,
    /// Settings › Mission's worker candidates when the mission was
    /// created (D-34, B48); a book written before them has none, which
    /// admits any agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// How strongly the Conductor has the nodes checked: Settings ›
    /// Conductor's default when the mission was created, then what its plan
    /// names. None for a mission from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<Strength>,
    /// The review and verify defaults a mission created before verification
    /// strength recorded (PRD mission-map-git-criteria D-22), kept as
    /// written for its Conductor to read; no mission records them now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkers: Option<serde_json::Value>,
    /// Made before node criteria: no node needs one and nothing is gated
    /// on them (D-19, B23).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exempt: bool,
    /// The answered questions that named a node, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decided: Vec<Decided>,
    #[serde(default)]
    pub nodes: Vec<MissionNode>,
    /// The order the plan merges in, by node key (D-09).
    #[serde(default)]
    pub merge_order: Vec<String>,
    /// The plan's check command (D-26).
    #[serde(default)]
    pub check: Option<String>,
    /// The GitHub account the plan uses, as `gh auth status` named it (D-54).
    #[serde(default)]
    pub github: Option<String>,
    /// What the operator has to decide now; the map's one line (B17).
    #[serde(default)]
    pub decision: Option<String>,
    /// What the Conductor said beside that line; none for a mission written
    /// before it, whose decision is a line alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    /// How many questions this mission has asked; the next one's key.
    #[serde(default)]
    pub question_seq: u64,
    /// The Views its Conductor opened since its last question was asked,
    /// or since it was created, oldest first, at most [`VIEW_LINK_LIMIT`]:
    /// the next question takes the ones still open (`mission_views`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conductor_views: Vec<ViewLink>,
    /// The operator's answer from the banner to the current question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    /// Why the last answer from the banner did not reach the Conductor,
    /// shown once beside the question until the next answer (B28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_failed: Option<AnswerFailure>,
    /// Why new nodes wait on usage (B13).
    #[serde(default)]
    pub delay: Option<String>,
    #[serde(default)]
    pub report: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub started_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub closed_at_unix_ms: Option<u64>,
    /// The wakes not yet in the Conductor's inbox, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wakes: Vec<PendingWake>,
    /// How many wakes this mission has had; the next one's `seq`.
    #[serde(default)]
    pub wake_seq: u64,
    /// The last round given to one of its outside nodes
    /// (`MissionNode::round`), never given again.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub round_seq: u64,
    /// When the operator asked the Conductor to end the mission (`미션
    /// 종료`), until it closes; a second press asks nothing more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_asked_at_unix_ms: Option<u64>,
    /// The count of raises, which keys each `Attention`.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub attention_seq: u64,
    /// The nodes as a reopen found them (`Mission::node_marks`): Hide does
    /// not close the mission again on them, only once a node moved (PRD
    /// mission-node-simple B18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopened_on: Option<String>,
    /// The operator's one handover of a Conductor that is still there
    /// (B15): good for one `hide mission take --handover` from the session
    /// it was made for, while the Conductor it was made on holds the
    /// mission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handover: Option<Handover>,
}

/// How long the operator's handover waits for the session it started.
pub const HANDOVER_TTL_MS: u64 = 10 * 60_000;

/// Whether `token` can name a handover: 16 to 64 letters, digits and `-`.
pub(crate) fn valid_handover(token: &str) -> bool {
    (16..=64).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The operator's choice to hand a mission whose Conductor is still there
/// to a new session (PRD mission-node-simple B15).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Handover {
    /// The secret the new session's take names; only that session was
    /// told it.
    pub token: String,
    /// The Conductor it was made on; another one holding the mission by
    /// then voids it.
    pub from: Actor,
    pub at_unix_ms: u64,
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

impl Mission {
    /// The (checker registration, Conductor) pairs of `node`: the checkers
    /// bound to its lines, each with this mission's Conductor. A letter from
    /// one to the other is what `hide mission show <node>` lists for its
    /// worker (with-hide/hide-private#668), and one the worker reads
    /// (`MissionBook::node_letters_read_by`).
    pub(crate) fn checker_letters<'a>(
        &'a self,
        node: &'a MissionNode,
    ) -> impl Iterator<Item = (&'a str, &'a Actor)> {
        node.checker_agents()
            .map(|checker| (checker, &self.conductor.actor))
    }

    /// Each node's key, status and pull requests, which a reopen keeps so
    /// the mission is not closed again until one of them moves.
    pub(crate) fn node_marks(&self) -> String {
        self.nodes
            .iter()
            .map(|node| {
                let prs: Vec<String> = node.prs.iter().map(u32::to_string).collect();
                format!("{}={}:{}", node.key, node.status.as_str(), prs.join(","))
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    pub(crate) fn node(&self, key: &str) -> Option<&MissionNode> {
        self.nodes.iter().find(|node| node.key == key)
    }

    /// The node a spawn's intent names as its worker's,
    /// `<mission id>:<node key>` (`hide mission help`); none for another
    /// intent, such as a checker's.
    pub(crate) fn intent_node(&self, intent: &str) -> Option<&MissionNode> {
        intent
            .strip_prefix(&self.id)
            .and_then(|rest| rest.strip_prefix(':'))
            .and_then(|key| self.node(key))
    }

    /// Why each node cannot start, from a dropped node anywhere upstream of
    /// it.
    pub(crate) fn blocked(&self) -> BTreeMap<String, Blocked> {
        let mut out = BTreeMap::new();
        for node in &self.nodes {
            if node.status.finished() {
                continue;
            }
            let mut found = None;
            crate::dag::walk_upstream(&self.nodes, node, |upstream| match upstream.status {
                // A dropped node is the one cause; the first ends the walk.
                NodeStatus::Dropped => {
                    found = Some(Blocked::UpstreamDropped);
                    crate::dag::Step::Stop
                }
                _ => crate::dag::Step::Continue,
            });
            if let Some(found) = found {
                out.insert(node.key.clone(), found);
            }
        }
        out
    }
}

impl crate::dag::Node for MissionNode {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

impl crate::dag::Node for NodeInput {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

/// Settings › Mission (D-31): the values a new plan starts from. A plan
/// changed in conversation keeps its own values (B3, B48); the cleanup of
/// merged worktrees and the way every mission's map is drawn apply to every
/// mission at once.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConductorSettings {
    #[serde(default = "default_cap")]
    pub concurrency: u8,
    #[serde(default)]
    pub default_scope: MissionScope,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// How strongly a new mission's nodes are checked.
    #[serde(default)]
    pub default_strength: Strength,
    /// Remove a done node's worktree once its pull request merged, when the
    /// tree is clean and nothing runs in it (`runtime/mission_cleanup.rs`).
    /// Read live by every mission; a value kept before it reads as on.
    #[serde(default = "default_clean_merged")]
    pub clean_merged: bool,
    /// Which way every mission's map runs, chosen on the map itself. Left
    /// out while it is the default, so a build from before it still reads
    /// the rest of Settings › Mission until the operator turns a map.
    #[serde(default, skip_serializing_if = "MapFlow::is_down")]
    pub map_flow: MapFlow,
}

/// Which way a mission's map runs: the work a node waits on above it
/// (`down`), or to its left (`across`), for a mission whose many
/// independent nodes would draw one row too wide to read.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapFlow {
    #[default]
    Down,
    Across,
}

impl MapFlow {
    fn is_down(&self) -> bool {
        *self == Self::Down
    }
}

impl Default for ConductorSettings {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CAP,
            default_scope: MissionScope::Pr,
            candidates: Vec::new(),
            default_strength: Strength::Normal,
            clean_merged: default_clean_merged(),
            map_flow: MapFlow::Down,
        }
    }
}

/// How strongly the Conductor has a mission's nodes checked: which nodes
/// get a review or a run-it check, and how many review rounds. The Conductor
/// follows it by `hide mission help`; the gate does not read it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Fast,
    #[default]
    Normal,
    Thorough,
}

fn default_clean_merged() -> bool {
    true
}

fn default_cap() -> u8 {
    DEFAULT_CAP
}

/// One worker candidate (D-31, D-34): an agent item (the agent and one of
/// its profiles on the mission's machine, PRD agent-profiles-phase2 D-08),
/// its model or the CLI's default, a fixed effort or none for the Conductor
/// to choose, and what it is for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub agent: String,
    /// The profile, by its id on the mission's machine; `None` for the
    /// agent's built-in default, which is left out of what is kept, so a
    /// value of built-in defaults only is the bytes a build before profiles
    /// wrote and reads (B14). A build before it refuses a value that names
    /// one (`deny_unknown_fields`): `docs/INSTALL.md`, What each system
    /// supports, says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<hide_agent_adapter::ProfileId>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub purpose: String,
}

impl Candidate {
    /// Whether a worker started as `target` with `launch` is this candidate
    /// (D-34, PRD agent-profiles-phase2 D-08, B13): the same agent and
    /// profile, the one the start actually runs with (`launch::pick`'s
    /// answer, never the profile the spawn named alone), its model where the
    /// candidate fixes one or the CLI's default where it names none, and its
    /// effort only where the candidate fixes one; an automatic effort is the
    /// Conductor's choice.
    pub(crate) fn admits(
        &self,
        target: &hide_agent_adapter::LaunchTarget,
        launch: &hide_agent_adapter::LaunchChoice,
    ) -> bool {
        hide_agent_adapter::canonical_kind(&self.agent)
            == hide_agent_adapter::canonical_kind(&target.kind)
            && self.profile_id() == target.profile
            && self.model == launch.model
            && self
                .effort
                .as_ref()
                .is_none_or(|effort| launch.effort.as_ref() == Some(effort))
    }

    /// The profile the candidate names, the built-in default for none.
    pub(crate) fn profile_id(&self) -> hide_agent_adapter::ProfileId {
        item_profile(self.profile.as_ref())
    }

    /// The row as it is kept: the built-in default named by its id is the
    /// one left out, so both spellings are one row.
    fn kept(mut self) -> Self {
        self.profile = self.profile.filter(|id| !id.is_default());
        self
    }
}

/// The profile an item names, the built-in default for none.
fn item_profile(profile: Option<&hide_agent_adapter::ProfileId>) -> hide_agent_adapter::ProfileId {
    profile
        .cloned()
        .unwrap_or_else(hide_agent_adapter::ProfileId::default_profile)
}

impl Worker {
    /// The item the plan names, as a start of it is asked for: its agent and
    /// its profile, the built-in default for none.
    pub(crate) fn target(&self) -> hide_agent_adapter::LaunchTarget {
        hide_agent_adapter::LaunchTarget {
            kind: self.agent.clone(),
            profile: item_profile(self.profile.as_ref()),
        }
    }

    /// Whether `candidate` admits this planned worker, by the test a start is
    /// judged by (`Candidate::admits`) with the plan's item, model and effort
    /// as the start's. A plan names the model of every worker that takes
    /// one (`mission_model_missing`), so a model it names as the agent's
    /// `cli_default_model` is the CLI's default to a candidate that keeps it.
    pub(crate) fn admitted_by(&self, candidate: &Candidate) -> bool {
        let cli_default =
            candidate.model.is_none() && self.model.as_deref() == cli_default_model(&self.agent);
        let launch = hide_agent_adapter::LaunchChoice {
            model: self.model.clone().filter(|_| !cli_default),
            effort: self.effort.clone(),
        };
        candidate.admits(&self.target(), &launch)
    }
}

impl ConductorSettings {
    /// Checks a value before it is kept: a cap in range, known agents, and
    /// each model and effort one the agent takes.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=CAP_LIMIT).contains(&self.concurrency) {
            return Err("conductor_settings.cap_out_of_range".into());
        }
        if self.candidates.len() > CANDIDATE_LIMIT {
            return Err("conductor_settings.too_many_candidates".into());
        }
        for candidate in &self.candidates {
            check_worker(
                &candidate.agent,
                candidate.model.as_deref(),
                candidate.effort.as_deref(),
            )
            .map_err(|code| format!("conductor_settings.{code}"))?;
            if candidate
                .profile
                .as_ref()
                .is_some_and(|id| !crate::project_settings::valid_profile_id(&id.0))
            {
                return Err("conductor_settings.profile_invalid".into());
            }
            if candidate.purpose.len() > LINE_LIMIT
                || candidate.purpose.chars().any(char::is_control)
            {
                return Err("conductor_settings.purpose_invalid".into());
            }
        }
        Ok(())
    }

    /// The value an edit keeps: each row as it is kept, then checked
    /// (`validate`).
    pub(crate) fn edited(mut self) -> Result<Self, String> {
        self.candidates = self.candidates.into_iter().map(Candidate::kept).collect();
        self.validate()?;
        Ok(self)
    }

    /// Reads the value `core-state.json` keeps. Fields a value kept by an
    /// earlier build may carry are left behind: `checkers`, the review and
    /// verify defaults from before verification strength (such a value reads
    /// as `normal`); `warn_review_fails` and `warn_quiet_minutes`, which
    /// nothing could change from their defaults and which are now fixed
    /// (`mission_health::REVIEW_FAILS_THAT_WARN`, `QUIET_THAT_WARNS_MS`); and
    /// `other_vendor`, the switch Settings › Mission had for a checker of
    /// another kind than the worker, which the Conductor now always prefers
    /// by `hide mission help`.
    pub(crate) fn from_stored(mut value: serde_json::Value) -> Result<Self, String> {
        if let Some(fields) = value.as_object_mut() {
            for field in [
                "checkers",
                "warn_review_fails",
                "warn_quiet_minutes",
                "other_vendor",
            ] {
                fields.remove(field);
            }
        }
        serde_json::from_value(value).map_err(|error| error.to_string())
    }
}

/// Whether `agent` takes a model at its start, by its own declaration.
pub(crate) fn takes_model(agent: &str) -> bool {
    hide_agent_adapter::adapter(agent)
        .and_then(|row| row.start)
        .is_some_and(|dialect| !dialect.options().model.is_empty())
}

/// The model `agent` runs with when a start names none, as Hide AI measured
/// it, which `hide agent catalog` offers as `cli_default_model` so a plan
/// can name one while the list is unread; none for an agent that takes no
/// model or whose own CLI keeps that default.
pub(crate) fn cli_default_model(agent: &str) -> Option<&'static str> {
    let adapter = hide_agent_adapter::adapter(agent)?;
    takes_model(agent)
        .then(|| hide_ai::ProviderId::for_agent(adapter.id))
        .flatten()
        .map(hide_ai::ProviderId::default_model)
        .filter(|model| !model.is_empty())
}

/// Whether `agent` is an agent Hide starts, and `model` and `effort` are
/// values it takes, by the agent's own declaration.
pub(crate) fn check_worker(
    agent: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<(), String> {
    let Some(dialect) = hide_agent_adapter::adapter(agent).and_then(|row| row.start) else {
        return Err("agent_unknown".into());
    };
    let options = dialect.options();
    if model
        .is_some_and(|model| options.model.is_empty() || !hide_agent_adapter::valid_model(model))
    {
        return Err("model_invalid".into());
    }
    if effort.is_some_and(|effort| !options.efforts.contains(&effort)) {
        return Err("effort_invalid".into());
    }
    Ok(())
}

/// Every mission the core keeps, and the next id to give.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionBook {
    #[serde(default)]
    pub missions: Vec<Mission>,
    #[serde(default)]
    pub next: u64,
}

impl MissionBook {
    pub(crate) fn get(&self, id: &str) -> Option<&Mission> {
        self.missions.iter().find(|mission| mission.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut Mission> {
        self.missions.iter_mut().find(|mission| mission.id == id)
    }

    /// The other registrations bound to the nodes registration `reader` is
    /// bound to, as their worker or a checker of one of their lines, each
    /// with the Conductor of its mission: the pairs whose letters, either
    /// way, `reader` may read (`hide request show`). A Conductor points a
    /// node's reviewer at the worker's design and its verifier at the
    /// review by the letter's id, so each of them reads what the others of
    /// its node exchanged with the Conductor (with-hide/hide#2298,
    /// with-hide/hide-private#656).
    pub(crate) fn node_letters_read_by<'a>(
        &'a self,
        reader: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a Actor)> {
        self.missions
            .iter()
            .flat_map(|mission| mission.nodes.iter().map(move |node| (mission, node)))
            .filter(move |(_, node)| node.bound_agents().any(|agent| agent == reader))
            .flat_map(move |(mission, node)| {
                node.bound_agents()
                    .filter(move |agent| *agent != reader)
                    .map(move |agent| (agent, &mission.conductor.actor))
            })
    }

    /// The mission `actor` conducts and still runs, if any.
    pub(crate) fn conducted_open(&self, actor: &Actor) -> Option<&Mission> {
        self.missions
            .iter()
            .find(|mission| mission.status.open() && mission.conductor.actor.same_identity(actor))
    }

    /// The mission a command from `actor` means when it names none: the one
    /// it conducts that still runs, else its newest done one.
    pub(crate) fn conducted(&self, actor: &Actor) -> Option<&Mission> {
        self.conducted_open(actor).or_else(|| {
            self.missions
                .iter()
                .filter(|mission| {
                    mission.status == MissionStatus::Done
                        && mission.conductor.actor.same_identity(actor)
                })
                .max_by_key(|mission| mission.updated_at_unix_ms)
        })
    }

    pub(crate) fn allocate(&mut self) -> String {
        self.next += 1;
        format!("mission-{}", self.next)
    }
}

/// A command a Conductor's pane sends (`hide mission …`). A command that
/// names no mission means the one the caller conducts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionCommand {
    Create {
        title: String,
        #[serde(default)]
        scope: Option<MissionScope>,
        #[serde(default)]
        cap: Option<u8>,
        #[serde(default)]
        github: Option<String>,
    },
    Plan {
        #[serde(default)]
        mission: Option<String>,
        plan: PlanInput,
    },
    Node {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        patch: NodePatch,
    },
    Start {
        #[serde(default)]
        mission: Option<String>,
    },
    /// Sets or clears the operator's one line (B17), with its reason,
    /// choices, recommended choice and stopped node (B32).
    Decide {
        #[serde(default)]
        mission: Option<String>,
        /// Named every time: `null` clears, so a request that leaves it
        /// out is refused rather than read as a clear.
        #[serde(deserialize_with = "present")]
        line: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommend: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<String>,
        /// Views of the Conductor's checkout to show with the question, by
        /// id; the CLI opens a path as a View first and sends its id.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        show: Vec<String>,
    },
    /// Sets or clears why new nodes wait on usage (B13).
    Delay {
        #[serde(default)]
        mission: Option<String>,
        /// Named every time: `null` clears, so a request that leaves it
        /// out is refused rather than read as a clear.
        #[serde(deserialize_with = "present")]
        line: Option<String>,
    },
    Close {
        #[serde(default)]
        mission: Option<String>,
        end: MissionEnd,
        #[serde(default)]
        report: Option<String>,
    },
    Reopen {
        #[serde(default)]
        mission: Option<String>,
    },
    /// The owner's result for one finish line of a node (PRD
    /// mission-map-git-criteria D-23, B5): the node's worker, a checker the
    /// Conductor bound, or the Conductor for a node it works itself.
    Record {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        line: String,
        pass: bool,
        /// The one line the result rests on, which every record carries
        /// (B5); a frame without one is the core's own refusal, with its
        /// way on, rather than one it cannot read.
        #[serde(default)]
        evidence: String,
        /// The result in one short line of the operator's words, which the
        /// map shows before the evidence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        /// The commit a check command ran on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
        /// This run is the rerun of a failure on the same commit (D-14).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        rerun: bool,
    },
    /// The Conductor goes past a line with its reason (B11, B12); a check
    /// command or CI line also names the operator's answered decision.
    Skip {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        line: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<u64>,
    },
    /// The Conductor relays the operator's answer to `node`'s human gate,
    /// given in its pane, with their words quoted (PRD mission-human-gate
    /// D-21): `pass` passes it, otherwise the words go back as a defect.
    Gate {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        pass: bool,
        said: String,
    },
    /// A later node found a defect in `node`: it works again with the
    /// defect as a line of its own (D-21, B21).
    Defect {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        found_by: String,
        line: String,
    },
    Take {
        mission: String,
        /// The operator's handover of a Conductor that is still there
        /// (`Mission::handover`), named by the session it was made for.
        #[serde(default)]
        handover: Option<String>,
    },
    /// The mission's view, or with `node` that node's with its worker's
    /// newest letters (with-hide/hide-private#552).
    Show {
        #[serde(default)]
        mission: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
    List,
    Settings,
}

impl MissionCommand {
    /// Whether the command can change the record; the others only read it.
    pub(crate) fn mutates(&self) -> bool {
        !matches!(self, Self::Show { .. } | Self::List | Self::Settings)
    }

    /// A command small enough and well-formed enough to take to the core.
    pub fn valid(&self) -> bool {
        let id_ok = |id: &Option<String>| id.as_deref().is_none_or(valid_key);
        match self {
            Self::Create { title, github, .. } => {
                valid_text(title, TITLE_LIMIT)
                    && github.as_deref().is_none_or(|g| valid_text(g, TITLE_LIMIT))
            }
            Self::Plan { mission, .. } | Self::Start { mission } | Self::Reopen { mission } => {
                id_ok(mission)
            }
            Self::Show { mission, node } => id_ok(mission) && id_ok(node),
            Self::Node { mission, node, .. } => id_ok(mission) && valid_key(node),
            Self::Record {
                mission,
                node,
                line,
                evidence,
                commit,
                ..
            } => {
                // A blank line is the core's own refusal, with its way on.
                id_ok(mission)
                    && valid_key(node)
                    && valid_key(line)
                    && (evidence.trim().is_empty() || valid_text(evidence, LINE_LIMIT))
                    && commit.as_deref().is_none_or(valid_commit)
            }
            Self::Skip {
                mission,
                node,
                line,
                reason,
                ..
            } => id_ok(mission) && valid_key(node) && valid_key(line) && reason.len() <= LINE_LIMIT,
            Self::Defect {
                mission,
                node,
                found_by,
                line,
            } => {
                id_ok(mission)
                    && valid_key(node)
                    && valid_key(found_by)
                    && valid_text(line, LINE_LIMIT)
            }
            // Words left out are the core's own refusal, with its way on.
            Self::Gate {
                mission,
                node,
                said,
                ..
            } => id_ok(mission) && valid_key(node) && said.len() <= LINE_LIMIT,
            Self::Decide {
                mission,
                line,
                why,
                options,
                node,
                show,
                ..
            } => {
                // The choices' count and length are refused with their own
                // reasons (`check_question`); here only a frame no command
                // could mean is.
                id_ok(mission)
                    && line
                        .as_deref()
                        .is_none_or(|line| valid_text(line, LINE_LIMIT))
                    && valid_line(why, LINE_LIMIT)
                    && options.len() <= NODE_LIMIT
                    && node.as_deref().is_none_or(valid_key)
                    && show.len() <= VIEW_LINK_LIMIT
                    && show
                        .iter()
                        .all(|value| valid_text(value, SHOW_LIMIT) && !value.contains(['\n', '\t']))
            }
            Self::Delay { mission, line } => {
                id_ok(mission)
                    && line
                        .as_deref()
                        .is_none_or(|line| valid_text(line, LINE_LIMIT))
            }
            Self::Close {
                mission, report, ..
            } => {
                id_ok(mission)
                    && report
                        .as_deref()
                        .is_none_or(|report| valid_text(report, REPORT_LIMIT))
            }
            // A token that names no handover is refused by the take itself
            // (`mission_conducted`).
            Self::Take { mission, handover } => {
                valid_key(mission) && handover.as_deref().is_none_or(|token| token.len() <= 64)
            }
            Self::List | Self::Settings => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEnd {
    Done,
    MergeWait,
    Cancelled,
}

/// A whole plan, which replaces the one before it (D-10): a node keeps what
/// its worker did when its key stays, a planned node that is left out goes,
/// and a node that already ran must be dropped first rather than vanish.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub scope: Option<MissionScope>,
    #[serde(default)]
    pub cap: Option<u8>,
    #[serde(default)]
    pub github: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    /// The verification strength the Conductor proposes or the operator
    /// asked for, with why in one line when it changes.
    #[serde(default)]
    pub strength: Option<Strength>,
    #[serde(default)]
    pub strength_why: Option<String>,
    #[serde(default)]
    pub merge_order: Vec<String>,
    pub nodes: Vec<NodeInput>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeInput {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub delivery: Option<Delivery>,
    #[serde(default)]
    pub worker: Option<Worker>,
    /// The node's own finish lines, at most three (PRD
    /// mission-map-git-criteria D-09). One sentence, as a plan written
    /// before them names it, is one line.
    #[serde(default, deserialize_with = "lines")]
    pub criteria: Vec<String>,
    /// How the node's work goes, three to five short lines (at most
    /// `STEP_LIMIT`, each `STEP_CHARS` characters). Left out, a replan keeps
    /// the lines the node has; an empty list clears them.
    #[serde(default)]
    pub steps: Option<Vec<String>>,
    /// The reviews and verifications other agents give (D-07).
    #[serde(default)]
    pub checks: Vec<CheckInput>,
    /// What an external node watches (D-20).
    #[serde(default)]
    pub external: Option<External>,
    #[serde(default)]
    pub issues: Vec<u32>,
    #[serde(default)]
    pub paths: Vec<String>,
    /// What a replan changed here and why (B18).
    #[serde(default)]
    pub change: Option<String>,
    /// The place the operator looks before the node is done, in at most
    /// `GATE_CHARS` characters (PRD mission-human-gate D-01, D-13): one
    /// text, or a list a node may hold one of. Left out, a replan keeps the
    /// gate the node has; `null` or an empty list takes it away, so a
    /// gate left out is never written as `null`.
    #[serde(
        default,
        deserialize_with = "gate_input",
        skip_serializing_if = "Option::is_none"
    )]
    pub gate: Option<Vec<String>>,
}

/// A change to one node. An absent field is kept, `null` clears it, so a
/// field the sender leaves unset is never written: `null` would clear it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatus>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_id: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub pr: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_pr: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub branch: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u8>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub line: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub change: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub review: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub worker: Option<Option<Worker>>,
    /// A direct change's commit on main (B33).
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub commit: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub verdict: Option<Option<Verdict>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub checks: Option<Option<Checks>>,
    /// The registration the Conductor started as the node's next reviewer
    /// or verifier (PRD mission-map-git-criteria B3, B4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<String>,
}

impl NodePatch {
    /// Whether it sets only what a node's worker may set on its own node
    /// (with-hide/hide#2302): the pull request and the branch it delivers
    /// on, and the node's one line. Its status, stack base, agents, words
    /// and verdicts stay its Conductor's.
    pub fn workers_own(&self) -> bool {
        let Self {
            status,
            agent_id,
            pr: _,
            base_pr,
            branch: _,
            attempts,
            line: _,
            change,
            review,
            worker,
            commit,
            verdict,
            checks,
            reviewer,
            verifier,
        } = self;
        status.is_none()
            && agent_id.is_none()
            && base_pr.is_none()
            && attempts.is_none()
            && change.is_none()
            && review.is_none()
            && worker.is_none()
            && commit.is_none()
            && verdict.is_none()
            && checks.is_none()
            && reviewer.is_none()
            && verifier.is_none()
    }
}

/// A node's criteria: a list of lines, one sentence, or none.
fn lines<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lines {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<Lines>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(Lines::One(line)) => vec![line],
        Some(Lines::Many(lines)) => lines,
    })
}

/// A node's gate as the plan names it, present: one text, a list, or
/// `null` for none.
fn gate_input<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<String>>, D::Error> {
    lines(deserializer).map(Some)
}

fn null_is_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

pub use crate::dag::valid_key;

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= limit
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn valid_line(value: &Option<String>, limit: usize) -> bool {
    value
        .as_deref()
        .is_none_or(|value| valid_text(value, limit))
}

/// The plan `input` would make of `mission`, or why it is refused.
pub(crate) fn apply_plan(
    mission: &Mission,
    input: &PlanInput,
    now: u64,
) -> Result<Mission, String> {
    if input.nodes.len() > NODE_LIMIT {
        return Err("mission_too_many_nodes".into());
    }
    if !valid_line(&input.title, TITLE_LIMIT)
        || !valid_line(&input.github, TITLE_LIMIT)
        || !valid_line(&input.check, LINE_LIMIT)
        || !valid_line(&input.strength_why, LINE_LIMIT)
    {
        return Err("mission_text_invalid".into());
    }
    if input.cap.is_some_and(|cap| !(1..=CAP_LIMIT).contains(&cap)) {
        return Err("mission_cap_out_of_range".into());
    }
    let mut keys = BTreeSet::new();
    for node in &input.nodes {
        if crate::dag::admit_key(&mut keys, &node.key).is_err() {
            return Err("mission_node_key_invalid".into());
        }
        if !valid_text(&node.title, TITLE_LIMIT)
            || node
                .criteria
                .iter()
                .any(|line| !valid_text(line, LINE_LIMIT))
            || !valid_line(&node.change, LINE_LIMIT)
            || node.depends_on.len() > DEPENDENCY_LIMIT
            || node.issues.len() > TOUCH_LIMIT
            || node.paths.len() > TOUCH_LIMIT
            || node.paths.iter().any(|path| !valid_text(path, LINE_LIMIT))
        {
            return Err("mission_text_invalid".into());
        }
        check_node_kind(node)?;
        if node.criteria.len() > crate::mission_criteria::OWN_LIMIT {
            return Err(crate::coordination::refusal(
                "mission_criteria_too_many",
                &node.key,
            ));
        }
        if node.criteria.is_empty() && node.kind != NodeKind::External && !mission.exempt {
            return Err(crate::coordination::refusal(
                "mission_criteria_missing",
                &node.key,
            ));
        }
        if node.steps.as_ref().is_some_and(|steps| !valid_steps(steps)) {
            return Err(crate::coordination::refusal(
                "mission_steps_invalid",
                &node.key,
            ));
        }
        check_gate(mission, node)?;
        check_checkers(node)?;
        if node.kind == NodeKind::Change && node.delivery.is_none() {
            return Err("mission_delivery_required".into());
        }
        if node.kind != NodeKind::Change && node.delivery.is_some() {
            return Err("mission_delivery_invalid".into());
        }
        if let Some(worker) = &node.worker {
            check_worker(
                &worker.agent,
                worker.model.as_deref(),
                worker.effort.as_deref(),
            )
            .map_err(|code| format!("mission_{code}"))?;
            // A worker that takes a model names one, so the map and the
            // start never read an empty choice; an agent that takes none
            // keeps `null` (PRD mission fixes (b)).
            if worker.model.is_none() && takes_model(&worker.agent) {
                return Err("mission_model_missing".into());
            }
        }
    }
    let edges = crate::dag::check_edges(&input.nodes);
    if matches!(edges, Err(crate::dag::Refusal::Unknown { .. })) {
        return Err("mission_dependency_unknown".into());
    }
    // Each node at most once, so the order is never longer than the plan.
    let mut ordered = BTreeSet::new();
    if input
        .merge_order
        .iter()
        .any(|key| !keys.contains(key.as_str()) || !ordered.insert(key.as_str()))
    {
        return Err("mission_dependency_unknown".into());
    }
    if edges.is_err() {
        return Err("mission_dependency_cycle".into());
    }
    // A node that ran keeps its record; one that is gone from the plan must
    // be dropped first, so a replan cannot hide work that was done (B28).
    for node in &mission.nodes {
        if node.started() && !keys.contains(node.key.as_str()) {
            return Err("mission_node_in_progress".into());
        }
    }
    let mut next = mission.clone();
    if input.check.is_some() {
        next.check.clone_from(&input.check);
    }
    let with_check = next.check.is_some() && !mission.exempt;
    let same_check = next.check == mission.check;
    next.nodes = input
        .nodes
        .iter()
        .map(|input| {
            let mut node = mission.node(&input.key).cloned().unwrap_or(MissionNode {
                key: input.key.clone(),
                title: input.title.clone(),
                kind: input.kind,
                depends_on: Vec::new(),
                status: NodeStatus::Planned,
                delivery: None,
                worker: None,
                agent_id: None,
                prs: Vec::new(),
                base_pr: None,
                branch: None,
                attempts: 0,
                views: Vec::new(),
                line: None,
                change: None,
                review: None,
                criteria: Vec::new(),
                steps: Vec::new(),
                brief: None,
                external: None,
                reopened: 0,
                issues: Vec::new(),
                paths: Vec::new(),
                started_at_unix_ms: None,
                finished_at_unix_ms: None,
                pr_seen: Vec::new(),
                commit: None,
                folder: None,
                remote_folder: None,
                done_head: None,
                in_main: None,
                branch_ahead: None,
                outside_round: 0,
                verdict: None,
                checks: None,
                attention: None,
                handled: Vec::new(),
                resumed: None,
                round: 0,
                hide: None,
                woken: Vec::new(),
                launched: None,
                pushed_at_unix_ms: None,
                worked_at_unix_ms: None,
                warnings: Vec::new(),
            });
            node.title.clone_from(&input.title);
            node.kind = input.kind;
            node.depends_on.clone_from(&input.depends_on);
            node.delivery = input.delivery;
            node.worker.clone_from(&input.worker);
            node.criteria = crate::mission_criteria::plan_lines(
                &node.criteria,
                &input.criteria,
                &input.checks,
                with_check && input.kind == NodeKind::Change,
                same_check,
            );
            if let Some(gate) = &input.gate {
                crate::mission_criteria::set_gate(
                    &mut node.criteria,
                    gate.first().map(|text| text.trim()),
                );
            }
            if let Some(steps) = &input.steps {
                node.steps = steps.iter().map(|step| step.trim().to_owned()).collect();
            }
            node.external.clone_from(&input.external);
            node.issues.clone_from(&input.issues);
            node.paths.clone_from(&input.paths);
            if input.change.is_some() {
                node.change.clone_from(&input.change);
            }
            node
        })
        .collect();
    next.merge_order.clone_from(&input.merge_order);
    if let Some(title) = &input.title {
        next.title.clone_from(title);
    }
    if let Some(scope) = input.scope {
        next.scope = scope;
    }
    if let Some(cap) = input.cap {
        next.cap = cap;
    }
    if input.strength.is_some() {
        next.strength = input.strength;
    }
    if input.github.is_some() {
        next.github.clone_from(&input.github);
    }
    next.updated_at_unix_ms = now;
    Ok(next)
}

/// A node's steps: at most `STEP_LIMIT`, each one line of text.
fn valid_steps(steps: &[String]) -> bool {
    steps.len() <= STEP_LIMIT
        && steps.iter().all(|step| {
            let step = step.trim();
            !step.is_empty()
                && step.chars().count() <= STEP_CHARS
                && !step.chars().any(char::is_control)
        })
}

/// A node's gate as the plan names it (PRD mission-human-gate D-13, D-15):
/// at most one, one line of at most `GATE_CHARS` characters, and none on a
/// node already done, whose work the operator can no longer look at before
/// it finishes; a mission made before node criteria gates nothing, so it
/// takes none.
fn check_gate(mission: &Mission, node: &NodeInput) -> Result<(), String> {
    let Some(gate) = &node.gate else {
        return Ok(());
    };
    let refused = |code: &str| Err(crate::coordination::refusal(code, &node.key));
    if gate.len() > 1 {
        return refused("mission_gate_too_many");
    }
    let text = gate.first().map(|text| text.trim());
    if let Some(text) = text {
        if text.is_empty() || text.chars().any(char::is_control) {
            return refused("mission_gate_invalid");
        }
        if text.chars().count() > crate::mission_criteria::GATE_CHARS {
            return refused("mission_gate_too_long");
        }
        if mission.exempt {
            return refused("mission_gate_exempt");
        }
    }
    let now = mission
        .node(&node.key)
        .and_then(|before| crate::mission_criteria::gate(&before.criteria))
        .and_then(|line| line.text.as_deref());
    let done = mission
        .node(&node.key)
        .is_some_and(|before| before.status == NodeStatus::Done);
    if done && now != text {
        return refused("mission_gate_node_done");
    }
    Ok(())
}

/// An external node names one pull request or branch and nothing a worker
/// would do (B19); no other node names one.
fn check_node_kind(node: &NodeInput) -> Result<(), String> {
    let external = node.kind == NodeKind::External;
    let named = match &node.external {
        None => false,
        Some(External::Pr(number)) => *number > 0,
        Some(External::Branch(branch)) => {
            valid_text(branch, TITLE_LIMIT) && !branch.chars().any(char::is_whitespace)
        }
    };
    let bare = node.worker.is_none()
        && node.delivery.is_none()
        && node.criteria.is_empty()
        && node.steps.as_ref().is_none_or(Vec::is_empty)
        && node.checks.is_empty()
        && node.gate.as_ref().is_none_or(Vec::is_empty);
    if external != node.external.is_some() || (external && (!named || !bare)) {
        return Err(crate::coordination::refusal(
            "mission_external_invalid",
            &node.key,
        ));
    }
    Ok(())
}

/// A node's reviews and verifications: each kind at most once, each with
/// one to `CHECKER_LIMIT` agents the machine takes (B3).
fn check_checkers(node: &NodeInput) -> Result<(), String> {
    let mut kinds = BTreeSet::new();
    for check in &node.checks {
        if !kinds.insert(check.kind.kind() as u8)
            || check.by.is_empty()
            || check.by.len() > crate::mission_criteria::CHECKER_LIMIT
        {
            return Err(crate::coordination::refusal(
                "mission_checkers_invalid",
                &node.key,
            ));
        }
        for worker in &check.by {
            check_worker(
                &worker.agent,
                worker.model.as_deref(),
                worker.effort.as_deref(),
            )
            .map_err(|code| format!("mission_{code}"))?;
            if worker.model.is_none() && takes_model(&worker.agent) {
                return Err("mission_model_missing".into());
            }
        }
    }
    Ok(())
}

/// `patch` applied to `node`, or why it is refused.
pub(crate) fn apply_patch(
    node: &MissionNode,
    patch: &NodePatch,
    now: u64,
) -> Result<MissionNode, String> {
    let text_ok = |value: &Option<Option<String>>| {
        value
            .as_ref()
            .is_none_or(|value| valid_line(value, LINE_LIMIT))
    };
    if !text_ok(&patch.line) || !text_ok(&patch.change) || !text_ok(&patch.review) {
        return Err("mission_text_invalid".into());
    }
    if patch
        .branch
        .as_ref()
        .is_some_and(|branch| !valid_line(branch, TITLE_LIMIT))
        || patch.agent_id.as_ref().is_some_and(|id| {
            id.as_deref()
                .is_some_and(|id| !crate::delivery::valid_key(id))
        })
    {
        return Err("mission_text_invalid".into());
    }
    // A commit is a direct change's and a verdict a verify node's (B33).
    let direct = node.kind == NodeKind::Change && node.delivery == Some(Delivery::Direct);
    if (patch.commit.is_some() && !direct)
        || ((patch.verdict.is_some() || patch.checks.is_some()) && node.kind != NodeKind::Verify)
    {
        return Err("mission_field_kind_invalid".into());
    }
    if patch.commit.as_ref().is_some_and(|commit| {
        commit
            .as_deref()
            .is_some_and(|commit| !valid_commit(commit))
    }) {
        return Err("mission_commit_invalid".into());
    }
    if patch.checks.as_ref().is_some_and(|checks| {
        checks.is_some_and(|checks| checks.total == 0 || checks.passed > checks.total)
    }) {
        return Err("mission_checks_invalid".into());
    }
    if let Some(Some(worker)) = &patch.worker {
        check_worker(
            &worker.agent,
            worker.model.as_deref(),
            worker.effort.as_deref(),
        )
        .map_err(|code| format!("mission_{code}"))?;
    }
    // An external node is done when Hide reads it in main (B19).
    if node.kind == NodeKind::External
        && (patch.status == Some(NodeStatus::Done)
            || patch.worker.is_some()
            || patch.agent_id.is_some())
    {
        return Err(crate::coordination::refusal(
            "mission_external_invalid",
            &node.key,
        ));
    }
    let mut next = node.clone();
    if let Some(status) = patch.status {
        if status == NodeStatus::Working && next.started_at_unix_ms.is_none() {
            next.started_at_unix_ms = Some(now);
        }
        if status == NodeStatus::Working && next.status != NodeStatus::Working {
            next.worked_at_unix_ms = Some(now);
        }
        if status.finished() && !next.status.finished() {
            next.finished_at_unix_ms = Some(now);
        } else if !status.finished() {
            next.finished_at_unix_ms = None;
        }
        next.status = status;
    }
    macro_rules! take {
        ($field:ident) => {
            if let Some(value) = &patch.$field {
                next.$field.clone_from(value);
            }
        };
    }
    take!(agent_id);
    // `--pr <n>` makes `n` the node's current pull request beside those it
    // holds; `--no-pr` lets go of every one.
    match patch.pr {
        Some(Some(number)) => next.attach_pr(number),
        Some(None) => next.prs.clear(),
        None => {}
    }
    take!(base_pr);
    take!(branch);
    take!(line);
    take!(change);
    take!(review);
    take!(worker);
    take!(commit);
    take!(verdict);
    take!(checks);
    // A pull request the node no longer holds keeps no record; a new one
    // starts its own wakes from what GitHub says now.
    let held = next.prs.clone();
    next.pr_seen.retain(|seen| held.contains(&seen.number));
    if let Some(attempts) = patch.attempts {
        next.attempts = attempts;
    }
    // A branch read in main proves that branch for that finish only: another
    // branch, or the node at work again, waits for a read of its own (B17).
    if next.branch != node.branch || next.status != NodeStatus::Done {
        next.in_main = None;
    }
    // The proof frozen at a finish goes with it: work taken up again is
    // judged against its folder, and a fresh check.
    if next.status != NodeStatus::Done {
        next.done_head = None;
    }
    Ok(next)
}

/// A direct change's branch as Hide last read it in main: the branch, and
/// its commit then, which is in main.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Landed {
    pub branch: String,
    pub commit: String,
}

/// A record's summary as the book keeps it: trimmed, none when blank, and
/// refused when it is not one line within `SUMMARY_LIMIT` characters.
pub(crate) fn summary_line(summary: Option<String>) -> Result<Option<String>, String> {
    let Some(summary) = summary.map(|summary| summary.trim().to_owned()) else {
        return Ok(None);
    };
    if summary.is_empty() {
        return Ok(None);
    }
    if summary.chars().count() > SUMMARY_LIMIT || summary.chars().any(char::is_control) {
        return Err("mission_summary_invalid".into());
    }
    Ok(Some(summary))
}

/// A commit as Git names one: 7 to 64 hexadecimal digits.
pub fn valid_commit(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The question a `decide` asks, or why it is refused (B32): its choices
/// within their count and length, a recommendation that names one of them,
/// and a node of the plan. A clear carries nothing beside it.
pub(crate) fn check_question(
    mission: &Mission,
    line: Option<&str>,
    why: Option<&str>,
    options: &[String],
    recommend: Option<u8>,
    node: Option<&str>,
) -> Result<(), String> {
    if line.is_none()
        && (why.is_some() || !options.is_empty() || recommend.is_some() || node.is_some())
    {
        return Err("mission_question_without_line".into());
    }
    if options.len() > OPTION_LIMIT {
        return Err("mission_options_too_many".into());
    }
    if options
        .iter()
        .any(|option| !valid_text(option, OPTION_TEXT_LIMIT) || option.contains('\n'))
    {
        return Err("mission_option_invalid".into());
    }
    if recommend.is_some_and(|choice| choice == 0 || usize::from(choice) > options.len()) {
        return Err("mission_recommend_invalid".into());
    }
    if node.is_some_and(|key| mission.node(key).is_none()) {
        return Err("mission_node_unknown".into());
    }
    Ok(())
}

/// Where another open mission touches the same issue or path as one of this
/// mission's nodes (D-18, B6): `(other mission, its title, what overlaps)`.
pub(crate) fn overlaps(book: &MissionBook, mission: &Mission) -> Vec<Overlap> {
    let mut out = Vec::new();
    for other in &book.missions {
        // A path names a project on one machine only.
        if other.id == mission.id
            || !other.status.open()
            || other.project != mission.project
            || other.device_id != mission.device_id
        {
            continue;
        }
        let issues: BTreeSet<u32> = other
            .nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Dropped)
            .flat_map(|node| node.issues.iter().copied())
            .collect();
        let paths: Vec<&str> = other
            .nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Dropped)
            .flat_map(|node| node.paths.iter().map(String::as_str))
            .collect();
        let mut shared_issues = BTreeSet::new();
        let mut shared_paths = BTreeSet::new();
        for node in &mission.nodes {
            shared_issues.extend(
                node.issues
                    .iter()
                    .copied()
                    .filter(|issue| issues.contains(issue)),
            );
            for path in &node.paths {
                if paths.iter().any(|theirs| overlapping(path, theirs)) {
                    shared_paths.insert(path.clone());
                }
            }
        }
        if !shared_issues.is_empty() || !shared_paths.is_empty() {
            out.push(Overlap {
                mission: other.id.clone(),
                title: other.title.clone(),
                issues: shared_issues.into_iter().collect(),
                paths: shared_paths.into_iter().collect(),
            });
        }
    }
    out
}

/// Whether two declared paths name the same file or one holds the other.
fn overlapping(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    a == b
        || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Overlap {
    pub mission: String,
    pub title: String,
    pub issues: Vec<u32>,
    pub paths: Vec<String>,
}

/// What an agent should do after a refusal, by its code.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "mission_unknown" => "Run hide mission list to see the missions of this project",
        "mission_none" => "Start one with hide mission create --title <title>",
        "mission_conducted" => {
            "Another session conducts this mission; ask the operator before taking it over"
        }
        "mission_not_conductor" => {
            "Only the mission's Conductor can change it, but a node's worker sets its own node's --pr, --branch and --line; take it over with hide mission take <id> once it has none"
        }
        "mission_already_conducting" => {
            "Close or cancel the mission you conduct before creating another"
        }
        "mission_cancelled" => "A cancelled mission does not reopen; create a new mission",
        "mission_closed" => "Reopen the mission with hide mission reopen before changing it",
        "mission_not_started" => {
            "Show the plan to the operator and run hide mission start after they approve it"
        }
        "mission_no_nodes" => "Set a plan with hide mission plan before starting",
        "mission_full" => "Ask the operator to delete old Conductor sessions from the archive",
        "mission_show_unknown" => {
            "Name a View of your own checkout from hide view list, or give --show a path to open"
        }
        "mission_too_many_nodes" => "Plan at most 64 nodes; merge or split the work differently",
        "mission_node_unknown" => "Run hide mission show to see the plan's node keys",
        "mission_ambiguous" => {
            "You work or check nodes of more than one open mission: name the one you mean with --mission <id> (hide mission list)"
        }
        "mission_node_in_progress" => {
            "Drop the node with hide mission node <key> --status dropped before removing it from the plan"
        }
        "mission_dependency_unknown" | "mission_dependency_cycle" => {
            "Check depends_on and merge_order: they must name other nodes of the plan, merge_order each at most once, without a cycle"
        }
        "mission_delivery_required" | "mission_delivery_invalid" => {
            "Give each change node a delivery (pr or direct) and no other node one"
        }
        "mission_cap_out_of_range" => "Use a concurrency cap from 1 to 16",
        "mission_agent_unknown" | "mission_model_invalid" | "mission_effort_invalid" => {
            "Run hide agent catalog to see the agents, models and efforts this machine takes"
        }
        "mission_model_missing" => {
            "Name the model of every worker that takes one: fill it from the candidate's fixed model, your choice from the catalog, or the catalog's cli_default_model"
        }
        "mission_question_without_line" => {
            "Give --why, --option, --recommend, --node and --show with --line; --clear takes none of them"
        }
        "mission_options_too_many" => "Offer at most 5 options; leave the rest to the conversation",
        "mission_option_invalid" => "Write each option as one line of at most 120 characters",
        "mission_recommend_invalid" => {
            "Recommend an option by its number, counted from 1 in the order of --option"
        }
        "mission_field_kind_invalid" => {
            "Record --commit on a direct change node and --verdict or --checks on a verify node"
        }
        "mission_commit_invalid" => "Give the commit as 7 to 64 hexadecimal digits",
        "mission_checks_invalid" => {
            "Give --checks as <passed>/<total> with total above 0 and passed at most total"
        }
        "mission_text_invalid" | "mission_plan_invalid" => {
            "Check the command's text and JSON and retry"
        }
        "mission_criteria_missing" => {
            "Give the node one to three criteria of its own in its plan entry (\"criteria\": [\"...\"]); an external node takes none"
        }
        "mission_criteria_too_many" => {
            "Give the node at most three criteria of its own; Hide adds the check command, CI and the reviews itself"
        }
        "mission_steps_invalid" => {
            "Give the node at most 8 steps in its plan entry (\"steps\": [\"...\"]), three to five usually, each one line of at most 80 characters with no line break"
        }
        "mission_checkers_invalid" => {
            "Name each of review and verify at most once per node, each with one to five agents in \"by\""
        }
        "mission_external_invalid" => {
            "An external node names one pull request (\"external\": {\"pr\": <n>}) or branch (\"external\": {\"branch\": \"<name>\"}) and no worker, delivery, criteria or checks; Hide marks it done when it reaches main, and you can only drop it"
        }
        "mission_checker_is_worker" => {
            "A node's checker is another agent than its worker: start a new agent for the review or verification and bind that one"
        }
        "mission_checker_unknown" => {
            "Bind a checker the node's plan names: its review or verify lists an unbound agent of the same kind as the registration (hide mission show)"
        }
        "mission_criterion_unknown" => {
            "Run hide mission show to see the node's lines and their keys (c1, check, ci, review, verify, d1…)"
        }
        "mission_criterion_not_yours" => {
            "Record only your own lines: a worker its node's own lines, check command and defects, a checker its review or verification; Hide records the CI, and the Conductor can only skip"
        }
        "mission_commit_required" => {
            "Give --commit <sha>: the commit the check command ran on, or the head of the pull request a review or verification read (git rev-parse HEAD in the worktree it read)"
        }
        "mission_rerun_spent" => {
            "This commit already had its one rerun: fix the failure, commit, and run the check again on the new commit"
        }
        "mission_summary_invalid" => {
            "Give --summary as one line of at most 160 characters in the operator's language: what the result means for them, the details staying in --evidence"
        }
        "mission_evidence_missing" => {
            "Give --evidence with the one line the result rests on: the command and what it printed, or what the review read"
        }
        "mission_checkout_unknown" => {
            "Record the check command from a pane in the checkout it ran in: Hide holds the line against that checkout's later commits"
        }
        "mission_machine_unavailable" => {
            "That agent's machine is not connected: reconnect it in Settings > Devices and bind again, or bind an agent that runs on the project's machine"
        }
        "mission_machine_no_repository" => {
            "Hide knows no project of the mission's GitHub repository on that agent's machine: register a checkout of it there as a project, or wait while GitHub is still being read for that project or for the mission's own, then bind again"
        }
        "mission_machine_node_refused" => {
            "On another machine than the project's, a mission runs only a pull-request change node's worker and its checkers: start this node's worker or checker on the project's machine, or bind one that runs there"
        }
        "mission_skip_reason_missing" => "Give the reason with --reason <text>",
        "mission_criteria_protected" => {
            "The check command and CI are skipped only on the operator's answer: ask with hide mission decide --line <question> --node <key>, and once answered skip with --decision <its key>"
        }
        "mission_criteria_unmet" => {
            "Finish the remaining lines first: have the owner finish the check and record it, skip a line with hide mission skip <node> <line> --reason <text>, or ask the operator with hide mission decide --node <key>"
        }
        "mission_nodes_unfinished" => {
            "Every node is done or dropped before the mission closes as done or merge_wait: finish them, drop them with hide mission node <key> --status dropped, or close as cancelled"
        }
        "mission_gate_too_many" => {
            "Give the node at most one gate (\"gate\": \"<what the operator looks at>\"): the one place they look before it is done"
        }
        "mission_gate_too_long" | "mission_gate_invalid" => {
            "Write the gate as one line of at most 80 characters naming what the operator looks at, such as the map screen"
        }
        "mission_gate_exempt" => {
            "A mission made before node criteria takes no gate: ask the operator with hide mission decide instead"
        }
        "mission_gate_node_done" => {
            "A done node's gate cannot change, since the operator can no longer look before it finishes: put the check on a node not yet done"
        }
        "mission_gate_operator_only" => {
            "A gate passes only on the operator's answer: they answer on the decision card or the phone, or you relay what they said in your pane with hide mission gate <node> --pass|--fix --said \"<their words>\""
        }
        "mission_gate_words_missing" => {
            "Quote the operator's own words with --said \"<what they said>\"; a gate never passes or reopens without them"
        }
        "mission_gate_none" => "The node has no gate; hide mission show <node> lists its lines",
        "mission_gate_not_open" => {
            "The node's gate does not wait on the operator now: its other lines are not all met yet, or it already passed (hide mission show <node>)"
        }
        "mission_defects_too_many" => {
            "The node holds 16 defects; ask the operator how to go on with it"
        }
        "mission_candidate_refused" => {
            "Plan and start each worker as one of the operator's candidates: run hide mission settings for the agent items (each with its --profile value, which a plan's worker names as profile), models and efforts it allows"
        }
        "mission_profile_unknown" => {
            "Name the worker's profile as hide agent catalog lists it for that agent (its profile value), or leave profile out for the agent's built-in default"
        }
        "mission_cap_reached" => {
            "Wait for a worker to finish (end its registration with hide agent end <id>) and retry; a start that failed stops counting after 10 minutes"
        }
        "mission_projects_loading" => {
            "Retry in a few seconds; Hide is still reading that machine's projects"
        }
        "mission_project_unknown" => {
            "Run the command from a pane in a checkout of a registered project"
        }
        "mission_unavailable" => {
            "Stop and tell the operator: Hide could not read its mission record (the reason is in Hide's diagnostic log); it is kept as it is, and Hide reads it again when it restarts"
        }
        "agent_pane_required" => "Run the command from an agent's pane",
        "native_identity_required" => "Retry after the agent's session is observed",
        "caller_context_changed" => "Run hide workspace info and retry",
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    #[serde(flatten)]
    book: MissionBook,
}

pub(crate) fn encode(book: &Arc<MissionBook>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        book: MissionBook::clone(book),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

/// A book read back, and the schema it was written in.
struct Decoded {
    book: MissionBook,
    from: u32,
}

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let decoded = |error: serde_json::Error| format!("could not be decoded: {error}");
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(decoded)?;
    let from = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or("has no schema version")?;
    match from {
        // A schema 3 book holds no human line: every node reads with no
        // gate, as it ran before.
        SCHEMA_VERSION | SCHEMA_BEFORE_GATES => {}
        SCHEMA_BEFORE_SIMPLE => convert_before_simple(&mut value)?,
        SCHEMA_BEFORE_CRITERIA => {
            convert_before_criteria(&mut value)?;
            convert_before_simple(&mut value)?;
        }
        other => {
            return Err(format!(
                "has schema version {other}, expected {SCHEMA_VERSION}"
            ));
        }
    }
    let mut stored: Stored = serde_json::from_value(value).map_err(decoded)?;
    for line in stored
        .book
        .missions
        .iter_mut()
        .flat_map(|mission| mission.nodes.iter_mut())
        .flat_map(|node| node.criteria.iter_mut())
    {
        line.carry_spent_rerun();
        line.count_tally();
    }
    if stored.book.missions.len() > MISSION_LIMIT
        || stored
            .book
            .missions
            .iter()
            .any(|mission| mission.nodes.len() > NODE_LIMIT)
    {
        return Err("holds more missions or nodes than the limits allow".to_owned());
    }
    Ok(Decoded {
        book: stored.book,
        from,
    })
}

/// A schema 1 book as schema 2 reads it (D-19, B23): every mission was made
/// before node criteria and is exempt from their gate, and a node's one
/// sentence, where it had one, is its one line.
fn convert_before_criteria(value: &mut serde_json::Value) -> Result<(), String> {
    let missions = match value.get_mut("missions") {
        None | Some(serde_json::Value::Null) => return Ok(()),
        Some(serde_json::Value::Array(missions)) => missions,
        Some(_) => return Err("has a mission list that is not a list".to_owned()),
    };
    for mission in missions {
        let mission = mission
            .as_object_mut()
            .ok_or("has a mission that is not an object")?;
        mission.insert("exempt".into(), serde_json::Value::Bool(true));
        let Some(nodes) = mission
            .get_mut("nodes")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for node in nodes {
            let node = node
                .as_object_mut()
                .ok_or("has a node that is not an object")?;
            let lines = match node.remove("criteria") {
                None | Some(serde_json::Value::Null) => Vec::new(),
                Some(serde_json::Value::String(text)) => {
                    let mut line =
                        Criterion::new("c1", crate::mission_criteria::CriterionKind::Own);
                    line.text = Some(text);
                    vec![line]
                }
                Some(_) => return Err("has a node criteria that is not a sentence".to_owned()),
            };
            node.insert(
                "criteria".into(),
                serde_json::to_value(lines).map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(())
}

/// A schema 2 book as schema 3 reads it (PRD mission-node-simple D-16,
/// B26), once: a node in review or held is in progress, one waiting has not
/// started, and one the Conductor recorded failed is in progress with the
/// operator asked to look, since the Conductor already let it go. A node's
/// one pull request is its list, and what GitHub said of it names it.
fn convert_before_simple(value: &mut serde_json::Value) -> Result<(), String> {
    let missions = match value.get_mut("missions") {
        None | Some(serde_json::Value::Null) => return Ok(()),
        Some(serde_json::Value::Array(missions)) => missions,
        Some(_) => return Err("has a mission list that is not a list".to_owned()),
    };
    for mission in missions {
        let mission = mission
            .as_object_mut()
            .ok_or("has a mission that is not an object")?;
        let at = mission
            .get("updated_at_unix_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let mut seq = mission
            .get("attention_seq")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let Some(nodes) = mission
            .get_mut("nodes")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for node in nodes {
            let node = node
                .as_object_mut()
                .ok_or("has a node that is not an object")?;
            let status = node
                .get("status")
                .and_then(serde_json::Value::as_str)
                .ok_or("has a node with no status")?
                .to_owned();
            let next = match status.as_str() {
                "planned" | "waiting" => "planned",
                "working" | "review" | "held" | "failed" => "working",
                "done" => "done",
                "dropped" => "dropped",
                other => return Err(format!("has a node status {other} it does not know")),
            };
            node.insert("status".into(), next.into());
            // A schema 2 node's `round` is an outside node's read round
            // (#1961); schema 3 names that `outside_round` and counts the
            // work rounds in `round` from none.
            if let Some(round) = node.remove("round") {
                node.insert("outside_round".into(), round);
            }
            // In a closed mission too: the failure is the operator's to
            // read, and the mission's Needs You carries it once it asks.
            if status == "failed" {
                seq += 1;
                let attention = Attention {
                    reason: AttentionReason::ConductorFailed,
                    pr: None,
                    agent: None,
                    key: seq,
                    since_unix_ms: at,
                    operator_at_unix_ms: Some(at),
                    notified: false,
                };
                node.insert(
                    "attention".into(),
                    serde_json::to_value(attention).map_err(|error| error.to_string())?,
                );
                node.insert("finished_at_unix_ms".into(), serde_json::Value::Null);
            }
            let pr = match node.remove("pr") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::Number(number)) => Some(
                    number
                        .as_u64()
                        .and_then(|number| u32::try_from(number).ok())
                        .ok_or("has a pull request number out of range")?,
                ),
                Some(_) => return Err("has a pull request that is not a number".to_owned()),
            };
            node.insert(
                "prs".into(),
                serde_json::Value::Array(pr.into_iter().map(Into::into).collect()),
            );
            let seen = match (node.remove("pr_seen"), pr) {
                (Some(serde_json::Value::Object(mut seen)), Some(pr)) => {
                    seen.insert("number".into(), pr.into());
                    vec![serde_json::Value::Object(seen)]
                }
                _ => Vec::new(),
            };
            node.insert("pr_seen".into(), serde_json::Value::Array(seen));
        }
        if seq > 0 {
            mission.insert("attention_seq".into(), seq.into());
        }
    }
    Ok(())
}

/// The size of the book as it would be saved, counted without keeping the
/// bytes and stopped just past `limit`, so a check costs at most that much
/// encoding.
fn saved_size(book: &MissionBook, limit: usize) -> usize {
    struct Counter(usize, usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            if self.0 > self.1 {
                return Err(std::io::Error::other("over budget"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0, limit);
    let _ = serde_json::to_writer(&mut counter, book);
    counter.0
}

/// Whether the book, as it would be saved, stays within what a start reads
/// back.
pub(crate) fn fits(book: &MissionBook) -> bool {
    saved_size(book, WRITE_BUDGET) <= WRITE_BUDGET
}

/// Whether a change from `before` to `after` may be kept: one that stays in
/// the budget, or one that makes a book already past it smaller, so a
/// close or a smaller plan is never refused.
pub(crate) fn may_keep(before: &MissionBook, after: &MissionBook) -> bool {
    // A book past the budget came from a file the read cap let in, so both
    // are counted to just past that cap.
    let limit = READ_CAP as usize;
    fits(after) || saved_size(after, limit) < saved_size(before, limit)
}

/// Where a restored book came from: a file this build cannot use is kept
/// aside rather than replaced, so a downgrade does not erase missions.
pub(crate) enum Restored {
    Fresh,
    Read(MissionBook),
    Unreadable(String),
}

/// Where the book lives and how it is read at launch; it is written by the
/// missions' one writer, after the histories it goes with
/// (`mission_events::MissionEventStore`).
pub(crate) struct MissionStore {
    path: PathBuf,
}

impl MissionStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Reads the book at launch, before the runtime lock is taken. An older
    /// schema's book is converted, and the file it came from is copied aside
    /// once before anything writes the current schema over it.
    pub(crate) fn restore(&self) -> Restored {
        match read_capped(&self.path, READ_CAP) {
            Ok(None) => Restored::Fresh,
            Ok(Some(bytes)) => match decode(&bytes) {
                Ok(Decoded { book, from }) => {
                    // With no copy kept the converted book is not written
                    // over the old one: it is read as unreadable, which
                    // keeps the file as it is.
                    if from != SCHEMA_VERSION
                        && let Err(reason) = self.keep_before_conversion(&bytes, from)
                    {
                        return Restored::Unreadable(reason);
                    }
                    Restored::Read(book)
                }
                Err(reason) => Restored::Unreadable(reason),
            },
            Err(reason) => Restored::Unreadable(reason),
        }
    }

    /// The copy a downgrade is restored from; an existing copy is the
    /// earliest one and is kept.
    fn keep_before_conversion(&self, bytes: &[u8], from: u32) -> Result<(), String> {
        let Some(directory) = self.path.parent() else {
            return Err("has no folder to keep its copy before conversion in".to_owned());
        };
        let backup = match from {
            SCHEMA_BEFORE_CRITERIA => hide_kit::layout::missions_before_criteria(directory),
            SCHEMA_BEFORE_SIMPLE => hide_kit::layout::missions_before_simple(directory),
            _ => hide_kit::layout::missions_before_gates(directory),
        };
        let outcome = crate::latest_file::keep_first_copy(&backup, bytes);
        crate::diagnostic!(serde_json::json!({
            "component": "missions",
            "kind": "missions.converted",
            "from": from,
            "to": SCHEMA_VERSION,
            "backup": backup.to_string_lossy(),
            "written": outcome.as_ref().ok(),
            "message": outcome.as_ref().err().map(ToString::to_string),
        }));
        outcome
            .map(|_| ())
            .map_err(|error| format!("could not keep its copy before conversion: {error}"))
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
pub(crate) mod tests {

    /// B15, B16 (with-hide/hide#2227): `leave` comes first while a Conductor
    /// holds the mission, and for an unplanned pull request also without
    /// one; `drop` comes last; a failed Conductor's raise is never handed on.
    #[test]
    fn each_raise_offers_its_choices_handing_it_on_first_and_dropping_last() {
        use OperatorAction::{Attach, Done, Drop, Leave, Resume};
        assert_eq!(AttentionReason::WorkerEnded.choices(true), [Leave, Drop]);
        assert_eq!(AttentionReason::WorkerEnded.choices(false), [Drop]);
        assert_eq!(AttentionReason::MergedUnproven.choices(true), [Leave, Done]);
        assert_eq!(AttentionReason::UnplannedPr.choices(false), [Leave, Attach]);
        assert_eq!(
            AttentionReason::ConductorFailed.choices(true),
            [Resume, Drop]
        );
        // A refused close closes as done first, hands on after, and never
        // drops a node whose work is finished.
        assert_eq!(AttentionReason::CloseRefused.choices(true), [Done, Leave]);
        assert_eq!(AttentionReason::CloseRefused.choices(false), [Done]);
    }
    use super::*;

    #[test]
    fn a_written_answer_is_judged_after_the_spaces_around_it_go() {
        let full = "가".repeat(ANSWER_LIMIT);
        assert_eq!(answer_text(&format!("  {full} \n")), Some(full.clone()));
        assert_eq!(
            answer_text(" merge after lunch\t"),
            Some("merge after lunch".into())
        );
        for refused in [
            String::new(),
            " \n ".into(),
            "two\nlines".into(),
            "bell\u{7}".into(),
            format!("{full}가"),
        ] {
            assert_eq!(answer_text(&refused), None, "{refused:?}");
        }
    }

    fn actor(pane: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: "conductor".into(),
            kind: "claude".into(),
            device_id: "node-a".into(),
            session: Some("s".into()),
        }
    }

    pub(crate) fn mission() -> Mission {
        Mission {
            id: "mission-1".into(),
            title: "issues".into(),
            project: "/repo".into(),
            workspace_id: "ws-1".into(),
            checkout_path: "/repo".into(),
            device_id: "node-a".into(),
            conductor: Conductor {
                actor: actor("w1:p1"),
                session: None,
                agent_id: None,
                profile: None,
                profile_unknown: false,
            },
            status: MissionStatus::Planning,
            scope: MissionScope::Pr,
            cap: 4,
            candidates: Vec::new(),
            strength: None,
            checkers: None,
            exempt: false,
            decided: Vec::new(),
            nodes: Vec::new(),
            merge_order: Vec::new(),
            check: None,
            github: None,
            decision: None,
            question: None,
            question_seq: 0,
            conductor_views: Vec::new(),
            answer: None,
            answer_failed: None,
            delay: None,
            report: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            started_at_unix_ms: None,
            closed_at_unix_ms: None,
            wakes: Vec::new(),
            wake_seq: 0,
            round_seq: 0,
            end_asked_at_unix_ms: None,
            attention_seq: 0,
            reopened_on: None,
            handover: None,
        }
    }

    /// A book is only kept when its save stays under what a start reads
    /// back, so the file Hide writes never comes back unreadable.
    #[test]
    fn a_book_past_what_a_start_reads_back_does_not_fit() {
        let mut large = mission();
        large.report = Some("x".repeat(WRITE_BUDGET / 2));
        let mut book = MissionBook::default();
        book.missions.push(large.clone());
        assert!(fits(&book));
        let before = book.clone();
        book.missions.push(large);
        assert!(!fits(&book));
        assert!(!may_keep(&before, &book));
        // A change that makes a book already past the budget smaller is kept.
        let mut over = book.clone();
        over.missions.push(over.missions[0].clone());
        let mut smaller = over.clone();
        smaller.missions[2].report = Some("x".into());
        assert!(!fits(&smaller));
        assert!(may_keep(&over, &smaller));
        // A move that finds a mission's registration gone writes null.
        let mut stored = serde_json::to_value(&book.missions[0]).unwrap();
        stored["workspace_id"] = serde_json::Value::Null;
        // A book saved while the first plan still opened the map carries
        // its `opened` mark, which reads as nothing now (mission-pane-chip D-11).
        stored["opened"] = serde_json::Value::Bool(true);
        let read: Mission = serde_json::from_value(stored).unwrap();
        assert_eq!(read.workspace_id, "");
        assert!((WRITE_BUDGET as u64) < READ_CAP);
    }

    pub(crate) fn node(key: &str, kind: NodeKind, depends: &[&str]) -> NodeInput {
        NodeInput {
            key: key.into(),
            title: format!("node {key}"),
            kind,
            depends_on: depends.iter().map(|key| (*key).to_owned()).collect(),
            delivery: (kind == NodeKind::Change).then_some(Delivery::Pr),
            worker: None,
            criteria: vec![format!("{key} works")],
            steps: None,
            checks: Vec::new(),
            external: None,
            issues: Vec::new(),
            paths: Vec::new(),
            change: None,
            gate: None,
        }
    }

    pub(crate) fn plan(nodes: Vec<NodeInput>) -> PlanInput {
        PlanInput {
            title: None,
            scope: None,
            cap: None,
            github: None,
            check: None,
            strength: None,
            strength_why: None,
            merge_order: Vec::new(),
            nodes,
        }
    }

    #[test]
    fn a_plan_with_a_cycle_or_an_unknown_dependency_is_refused() {
        let cycle = plan(vec![
            node("a", NodeKind::Change, &["b"]),
            node("b", NodeKind::Change, &["a"]),
        ]);
        assert_eq!(
            apply_plan(&mission(), &cycle, 2).unwrap_err(),
            "mission_dependency_cycle"
        );
        let unknown = plan(vec![node("a", NodeKind::Change, &["z"])]);
        assert_eq!(
            apply_plan(&mission(), &unknown, 2).unwrap_err(),
            "mission_dependency_unknown"
        );
        let mut repeated = plan(vec![node("a", NodeKind::Change, &[])]);
        repeated.merge_order = vec!["a".into(), "a".into()];
        assert_eq!(
            apply_plan(&mission(), &repeated, 2).unwrap_err(),
            "mission_dependency_unknown"
        );
    }

    #[test]
    fn only_a_change_node_carries_a_delivery() {
        let mut research = node("r", NodeKind::Research, &[]);
        research.delivery = Some(Delivery::Pr);
        assert_eq!(
            apply_plan(&mission(), &plan(vec![research]), 2).unwrap_err(),
            "mission_delivery_invalid"
        );
        let mut change = node("c", NodeKind::Change, &[]);
        change.delivery = None;
        assert_eq!(
            apply_plan(&mission(), &plan(vec![change]), 2).unwrap_err(),
            "mission_delivery_required"
        );
    }

    #[test]
    fn a_replan_keeps_what_a_node_did_and_refuses_to_drop_a_running_node_silently() {
        let first = apply_plan(
            &mission(),
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("b", NodeKind::Change, &["a"]),
            ]),
            2,
        )
        .unwrap();
        let mut running = first.clone();
        running.nodes[0] = apply_patch(
            &running.nodes[0],
            &NodePatch {
                status: Some(NodeStatus::Working),
                pr: Some(Some(7)),
                ..NodePatch::default()
            },
            3,
        )
        .unwrap();
        let again = apply_plan(
            &running,
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("c", NodeKind::Research, &[]),
            ]),
            4,
        )
        .unwrap();
        assert_eq!(again.nodes[0].prs, vec![7]);
        assert_eq!(again.nodes[0].status, NodeStatus::Working);
        assert!(again.node("b").is_none(), "a planned node left out goes");
        let without = apply_plan(&running, &plan(vec![node("c", NodeKind::Research, &[])]), 4);
        assert_eq!(without.unwrap_err(), "mission_node_in_progress");
        // A node whose status still says planned but that names its pull
        // request ran too.
        let mut named = first.clone();
        named.nodes[1].prs = vec![8];
        let dropped = apply_plan(&named, &plan(vec![node("a", NodeKind::Change, &[])]), 4);
        assert_eq!(dropped.unwrap_err(), "mission_node_in_progress");
    }

    #[test]
    fn a_dropped_node_holds_everything_downstream_of_it() {
        let mut m = apply_plan(
            &mission(),
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("b", NodeKind::Change, &["a"]),
                node("c", NodeKind::Verify, &["b"]),
                node("d", NodeKind::Change, &[]),
            ]),
            2,
        )
        .unwrap();
        assert!(m.blocked().is_empty());
        m.nodes[0].status = NodeStatus::Dropped;
        let blocked = m.blocked();
        assert_eq!(blocked.get("b"), Some(&Blocked::UpstreamDropped));
        assert_eq!(blocked.get("c"), Some(&Blocked::UpstreamDropped));
        assert_eq!(blocked.get("d"), None);
    }

    #[test]
    fn a_decision_or_delay_without_its_line_is_refused_and_null_clears() {
        for op in ["decide", "delay"] {
            assert!(
                serde_json::from_value::<MissionCommand>(serde_json::json!({"op": op})).is_err()
            );
            assert!(
                serde_json::from_value::<MissionCommand>(
                    serde_json::json!({"op": op, "line": null})
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn a_patch_null_clears_and_an_absent_field_is_kept() {
        let m = apply_plan(&mission(), &plan(vec![node("a", NodeKind::Change, &[])]), 2).unwrap();
        let set: NodePatch =
            serde_json::from_value(serde_json::json!({"line": "running tests", "pr": 3})).unwrap();
        let after = apply_patch(&m.nodes[0], &set, 3).unwrap();
        assert_eq!(after.line.as_deref(), Some("running tests"));
        let clear: NodePatch = serde_json::from_value(serde_json::json!({"line": null})).unwrap();
        let cleared = apply_patch(&after, &clear, 4).unwrap();
        assert_eq!(cleared.line, None);
        assert_eq!(cleared.prs, vec![3]);
    }

    #[test]
    fn a_patch_sent_as_the_cli_sends_it_keeps_what_it_does_not_name() {
        let mut planned = node("a", NodeKind::Change, &[]);
        planned.worker = Some(Worker {
            agent: "claude".into(),
            profile: None,
            model: Some("opus".into()),
            effort: None,
            effort_auto: false,
        });
        let m = apply_plan(&mission(), &plan(vec![planned]), 2).unwrap();
        // The CLI builds the patch in Rust and the daemon reads it back from JSON.
        let wire = |patch: NodePatch| -> NodePatch {
            serde_json::from_value(serde_json::to_value(patch).unwrap()).unwrap()
        };
        let set = wire(NodePatch {
            pr: Some(Some(3)),
            agent_id: Some(Some("agent-3".into())),
            ..Default::default()
        });
        let after = apply_patch(&m.nodes[0], &set, 3).unwrap();
        let status = wire(NodePatch {
            status: Some(NodeStatus::Working),
            ..Default::default()
        });
        let reviewed = apply_patch(&after, &status, 4).unwrap();
        assert_eq!(
            reviewed.worker.as_ref().map(|worker| worker.agent.as_str()),
            Some("claude")
        );
        assert_eq!(reviewed.prs, vec![3]);
        assert_eq!(reviewed.agent_id.as_deref(), Some("agent-3"));
        let cleared = apply_patch(
            &reviewed,
            &wire(NodePatch {
                pr: Some(None),
                ..Default::default()
            }),
            5,
        )
        .unwrap();
        assert!(cleared.prs.is_empty());
        assert_eq!(cleared.agent_id.as_deref(), Some("agent-3"));
    }

    /// D-10: `--pr` adds a pull request as the node's current one and keeps
    /// the others; naming one it holds makes it current again.
    #[test]
    fn a_named_pull_request_joins_the_node_as_its_current_one() {
        let m = apply_plan(&mission(), &plan(vec![node("a", NodeKind::Change, &[])]), 2).unwrap();
        let name = |pr: u32| NodePatch {
            pr: Some(Some(pr)),
            ..Default::default()
        };
        let one = apply_patch(&m.nodes[0], &name(3), 3).unwrap();
        let two = apply_patch(&one, &name(5), 4).unwrap();
        assert_eq!((two.prs.clone(), two.current_pr()), (vec![3, 5], Some(5)));
        let back = apply_patch(&two, &name(3), 5).unwrap();
        assert_eq!((back.prs.clone(), back.current_pr()), (vec![5, 3], Some(3)));
    }

    #[test]
    fn another_open_mission_on_the_same_issue_or_folder_is_an_overlap() {
        let mut book = MissionBook::default();
        let mut other = mission();
        other.id = "mission-2".into();
        other.title = "other".into();
        other.status = MissionStatus::Running;
        let mut theirs = node("x", NodeKind::Change, &[]);
        theirs.issues = vec![12];
        theirs.paths = vec!["web/src".into()];
        other = apply_plan(&other, &plan(vec![theirs]), 2).unwrap();
        book.missions.push(other);
        let mut mine = node("y", NodeKind::Change, &[]);
        mine.paths = vec!["web/src/App.tsx".into()];
        let m = apply_plan(&mission(), &plan(vec![mine]), 2).unwrap();
        let found = overlaps(&book, &m);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].paths, vec!["web/src/App.tsx".to_owned()]);
        // The same path on another machine is another folder.
        let mut elsewhere = m.clone();
        elsewhere.device_id = "node-b".into();
        assert!(overlaps(&book, &elsewhere).is_empty());
        let mut unrelated = node("z", NodeKind::Change, &[]);
        unrelated.paths = vec!["web/srcs".into()];
        let m = apply_plan(&mission(), &plan(vec![unrelated]), 2).unwrap();
        assert!(overlaps(&book, &m).is_empty());
    }

    #[test]
    fn a_book_past_its_limits_or_of_another_version_is_not_used() {
        let book = MissionBook {
            missions: vec![mission(); MISSION_LIMIT + 1],
            next: 1,
        };
        let bytes = encode(&Arc::new(book)).unwrap();
        assert!(decode(&bytes).is_err());
        let other = serde_json::to_vec(
            &serde_json::json!({"schema_version": 99, "missions": [], "next": 0}),
        )
        .unwrap();
        assert!(decode(&other).is_err());
        let ok = encode(&Arc::new(MissionBook::default())).unwrap();
        assert_eq!(decode(&ok).unwrap().book, MissionBook::default());
    }

    #[test]
    fn a_plan_keeps_a_nodes_steps_and_refuses_too_many_or_unreadable_ones() {
        let refused = |plan: PlanInput| {
            let code = apply_plan(&mission(), &plan, 2).unwrap_err();
            crate::coordination::split_refusal(&code).0.to_owned()
        };
        let with = |steps: Vec<String>| {
            let mut one = node("a", NodeKind::Change, &[]);
            one.steps = Some(steps);
            plan(vec![one])
        };
        let lines = |count: usize| (0..count).map(|n| format!("step {n}")).collect::<Vec<_>>();
        let planned = apply_plan(&mission(), &with(lines(STEP_LIMIT)), 2).unwrap();
        assert_eq!(planned.nodes[0].steps, lines(STEP_LIMIT));
        assert_eq!(
            refused(with(lines(STEP_LIMIT + 1))),
            "mission_steps_invalid"
        );
        assert_eq!(
            refused(with(vec!["x".repeat(STEP_CHARS + 1)])),
            "mission_steps_invalid"
        );
        assert_eq!(refused(with(vec!["  ".into()])), "mission_steps_invalid");
        assert_eq!(refused(with(vec!["a\nb".into()])), "mission_steps_invalid");
        // The limit counts characters, so a Korean line of the same length fits.
        assert!(apply_plan(&mission(), &with(vec!["가".repeat(STEP_CHARS)]), 2).is_ok());
        // A replan that leaves the steps out keeps them; an empty list clears them.
        let kept = apply_plan(&planned, &plan(vec![node("a", NodeKind::Change, &[])]), 3).unwrap();
        assert_eq!(kept.nodes[0].steps, lines(STEP_LIMIT));
        let cleared = apply_plan(&planned, &with(Vec::new()), 3).unwrap();
        assert!(cleared.nodes[0].steps.is_empty());
        // An outside node has no work to plan.
        let mut outside = node("pr", NodeKind::External, &[]);
        outside.delivery = None;
        outside.criteria.clear();
        outside.external = Some(External::Pr(7));
        outside.steps = Some(lines(3));
        assert_eq!(refused(plan(vec![outside])), "mission_external_invalid");
    }

    #[test]
    fn a_plan_names_one_to_three_criteria_per_node_and_an_external_node_none() {
        let refused = |plan: PlanInput| {
            let code = apply_plan(&mission(), &plan, 2).unwrap_err();
            crate::coordination::split_refusal(&code).0.to_owned()
        };
        let mut bare = node("a", NodeKind::Change, &[]);
        bare.criteria.clear();
        assert_eq!(
            refused(plan(vec![bare.clone()])),
            "mission_criteria_missing"
        );
        let mut four = node("a", NodeKind::Change, &[]);
        four.criteria = vec!["1".into(), "2".into(), "3".into(), "4".into()];
        assert_eq!(refused(plan(vec![four])), "mission_criteria_too_many");
        // A mission made before criteria plans as it did.
        let mut old = mission();
        old.exempt = true;
        assert!(apply_plan(&old, &plan(vec![bare]), 2).is_ok());
        let mut external = node("pr", NodeKind::External, &[]);
        external.delivery = None;
        external.criteria.clear();
        external.external = Some(External::Pr(1232));
        let mut with_worker = external.clone();
        with_worker.criteria = vec!["x".into()];
        assert_eq!(refused(plan(vec![with_worker])), "mission_external_invalid");
        let mut unnamed = external.clone();
        unnamed.external = None;
        assert_eq!(refused(plan(vec![unnamed])), "mission_external_invalid");
        let mut named = node("a", NodeKind::Change, &["pr"]);
        named.external = Some(External::Branch("feat/x".into()));
        assert_eq!(
            refused(plan(vec![external.clone(), named])),
            "mission_external_invalid"
        );
        let waits = node("a", NodeKind::Change, &["pr"]);
        let planned = apply_plan(&mission(), &plan(vec![external, waits]), 2).unwrap();
        assert!(planned.nodes[0].criteria.is_empty());
        // Each of review and verify at most once.
        let mut twice = node("a", NodeKind::Change, &[]);
        let review = CheckInput {
            kind: crate::mission_criteria::CheckKind::Review,
            rule: crate::mission_criteria::Rule::All,
            by: vec![Worker {
                agent: "codex".into(),
                profile: None,
                model: Some("gpt-6-astra".into()),
                effort: None,
                effort_auto: false,
            }],
        };
        twice.checks = vec![review.clone(), review];
        assert_eq!(refused(plan(vec![twice])), "mission_checkers_invalid");
    }

    #[test]
    fn a_plan_with_a_check_command_adds_it_to_change_nodes_and_keeps_what_was_recorded() {
        let mut input = plan(vec![
            node("a", NodeKind::Change, &[]),
            node("r", NodeKind::Research, &[]),
        ]);
        input.check = Some("cargo test".into());
        let mut planned = apply_plan(&mission(), &input, 2).unwrap();
        let keys = |node: &MissionNode| -> Vec<String> {
            node.criteria.iter().map(|line| line.key.clone()).collect()
        };
        assert_eq!(keys(&planned.nodes[0]), ["c1", "check"]);
        assert_eq!(keys(&planned.nodes[1]), ["c1"]);
        planned.nodes[0].criteria[0].state = crate::mission_criteria::CriterionState::Pass;
        // A plan given as one sentence, as one written before criteria is.
        let again: NodeInput = serde_json::from_value(serde_json::json!({
            "key": "a", "title": "node a", "kind": "change", "delivery": "pr",
            "criteria": "a works"
        }))
        .unwrap();
        let replanned = apply_plan(&planned, &plan(vec![again]), 3).unwrap();
        assert_eq!(
            replanned.nodes[0].criteria[0].state,
            crate::mission_criteria::CriterionState::Pass
        );
    }

    /// with-hide/hide#2080 review: a ci line an earlier build saved with its
    /// rerun spent (failed, its rerun counted) had sent its node back, so
    /// after a restart a later failure on that head does not send it again.
    #[test]
    fn a_ci_line_saved_spent_before_a_restart_does_not_send_its_node_back_again() {
        let head = "a".repeat(40);
        let planned =
            apply_plan(&mission(), &plan(vec![node("a", NodeKind::Change, &[])]), 2).unwrap();
        let mut value = serde_json::to_value(Stored {
            schema_version: SCHEMA_VERSION,
            book: MissionBook {
                missions: vec![planned],
                next: 2,
            },
        })
        .unwrap();
        value["missions"][0]["nodes"][0]["criteria"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "key": "ci", "kind": "ci", "state": "fail", "commit": head, "reruns": 1
            }));
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let Restored::Read(mut book) = MissionStore::new(path).restore() else {
            panic!("the book reads");
        };
        let line = book.missions[0].nodes[0]
            .criteria
            .iter_mut()
            .find(|line| line.key == "ci")
            .expect("the ci line");
        line.read_ci(&head, crate::model::CiEnd::Ended(CiResult::Passed), 1);
        line.read_ci(&head, crate::model::CiEnd::Running, 2);
        assert!(
            !line
                .read_ci(&head, crate::model::CiEnd::Ended(CiResult::Failed), 3)
                .spent
        );
    }

    /// D-19, B23: a schema 1 book reads as exempt missions whose node keeps
    /// its sentence as its one line, and its file is copied aside once.
    #[test]
    fn a_schema_one_book_is_converted_and_its_file_kept_aside() {
        let mut node = serde_json::to_value(mission()).unwrap();
        node["nodes"] = serde_json::json!([
            {"key": "a", "title": "A", "kind": "change", "depends_on": [], "status": "done",
             "delivery": "pr", "worker": null, "agent_id": null, "pr": 3, "base_pr": null,
             "branch": null, "attempts": 0, "line": null, "change": null, "review": null,
             "criteria": "CI and review", "issues": [], "paths": [],
             "started_at_unix_ms": null, "finished_at_unix_ms": null, "pr_seen": null},
            {"key": "b", "title": "B", "kind": "research", "depends_on": [], "status": "planned",
             "criteria": null}
        ]);
        let mut v1 = node.clone();
        v1.as_object_mut().unwrap().remove("exempt");
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version": 1, "next": 1, "missions": [v1]}),
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, &bytes).unwrap();
        let store = MissionStore::new(path.clone());
        let Restored::Read(book) = store.restore() else {
            panic!("a schema 1 book reads");
        };
        let read = &book.missions[0];
        assert!(read.exempt);
        assert_eq!(read.nodes[0].criteria.len(), 1);
        assert_eq!(
            read.nodes[0].criteria[0].text.as_deref(),
            Some("CI and review")
        );
        assert!(read.nodes[1].criteria.is_empty());
        let backup = hide_kit::layout::missions_before_criteria(folder.path());
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        // The first copy is the one kept.
        std::fs::write(&path, b"{\"schema_version\": 1, \"missions\": []}").unwrap();
        assert!(matches!(store.restore(), Restored::Read(_)));
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        // What is saved is the current schema and reads back the same.
        let saved = encode(&Arc::new(book.clone())).unwrap();
        let again = decode(&saved).unwrap();
        assert_eq!((again.from, again.book), (SCHEMA_VERSION, book));
    }

    /// PRD mission-node-simple D-16, B26: a schema 2 book reads in the
    /// three states once: review and held are in progress, waiting is not
    /// started, failed is in progress with the operator asked to look; a
    /// node's one pull request becomes its list, and the file is copied
    /// aside once.
    #[test]
    fn a_schema_two_book_moves_to_three_states_and_a_pull_request_list() {
        let mut stored = serde_json::to_value(mission()).unwrap();
        stored.as_object_mut().unwrap().remove("attention_seq");
        stored["updated_at_unix_ms"] = serde_json::json!(50);
        let node = |key: &str, status: &str| {
            serde_json::json!({"key": key, "title": key, "kind": "change", "status": status,
                "delivery": "pr", "pr": null, "pr_seen": null})
        };
        let mut merged = node("m", "review");
        merged["pr"] = serde_json::json!(7);
        merged["pr_seen"] = serde_json::json!({"head": "abc", "merged": true});
        // An outside read round of schema 2 (#1961) is not a work round.
        merged["round"] = serde_json::json!(4);
        stored["nodes"] = serde_json::json!([
            merged,
            node("h", "held"),
            node("w", "waiting"),
            node("f", "failed"),
            node("d", "done"),
        ]);
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version": 2, "next": 1, "missions": [stored]}),
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, &bytes).unwrap();
        let store = MissionStore::new(path);
        let Restored::Read(book) = store.restore() else {
            panic!("a schema 2 book reads");
        };
        let read = &book.missions[0];
        assert_eq!((read.nodes[0].outside_round, read.nodes[0].round), (4, 0));
        let statuses: Vec<NodeStatus> = read.nodes.iter().map(|node| node.status).collect();
        assert_eq!(
            statuses,
            [
                NodeStatus::Working,
                NodeStatus::Working,
                NodeStatus::Planned,
                NodeStatus::Working,
                NodeStatus::Done,
            ]
        );
        assert_eq!(read.nodes[0].prs, vec![7]);
        assert_eq!(read.nodes[0].seen(7).map(|seen| seen.merged), Some(true));
        let failed = read.nodes[3]
            .attention
            .as_ref()
            .expect("a failed node asks the operator");
        assert_eq!(
            (failed.reason, failed.operator_at_unix_ms, failed.key),
            (AttentionReason::ConductorFailed, Some(50), 1)
        );
        assert_eq!(read.attention_seq, 1);
        assert!(
            read.nodes
                .iter()
                .filter(|node| node.key != "f")
                .all(|node| node.attention.is_none())
        );
        let backup = hide_kit::layout::missions_before_simple(folder.path());
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        // Saved again it is schema 3, and it is not moved a second time.
        let saved = encode(&Arc::new(book.clone())).unwrap();
        let again = decode(&saved).unwrap();
        assert_eq!((again.from, again.book), (SCHEMA_VERSION, book));
    }

    /// PRD mission-human-gate D-02, B12: a schema 3 book, written before a
    /// node could carry a gate, reads with no gate and as it was, its file
    /// copied aside once, and is written back as the current schema, which
    /// reads the same again.
    #[test]
    fn a_schema_three_book_reads_with_no_gate_and_is_kept_aside_once() {
        let mut stored = serde_json::to_value(mission()).unwrap();
        stored["nodes"] = serde_json::json!([{"key": "fix", "title": "Fix it",
            "kind": "change", "status": "working", "delivery": "pr",
            "criteria": [{"key": "c1", "kind": "own", "text": "it works", "state": "pass"}]}]);
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version": 3, "next": 1, "missions": [stored]}),
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, &bytes).unwrap();
        let store = MissionStore::new(path.clone());
        let Restored::Read(book) = store.restore() else {
            panic!("a schema 3 book reads");
        };
        let node = &book.missions[0].nodes[0];
        assert_eq!(node.status, NodeStatus::Working);
        assert_eq!(
            node.criteria
                .iter()
                .map(|line| (line.key.as_str(), line.state))
                .collect::<Vec<_>>(),
            [("c1", crate::mission_criteria::CriterionState::Pass)]
        );
        assert!(crate::mission_criteria::gate(&node.criteria).is_none());
        let backup = hide_kit::layout::missions_before_gates(folder.path());
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        std::fs::write(&path, b"{\"schema_version\": 3, \"missions\": []}").unwrap();
        assert!(matches!(store.restore(), Restored::Read(_)));
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            bytes,
            "the first copy is kept"
        );
        let saved = encode(&Arc::new(book.clone())).unwrap();
        let again = decode(&saved).unwrap();
        assert_eq!((again.from, again.book), (SCHEMA_VERSION, book));
    }

    /// PRD mission-human-gate D-13, D-15, B3: a plan names at most one gate
    /// per node, of at most 80 characters, on a node that is not done and
    /// not outside; left out a replan keeps it, `null` takes it away, and
    /// what is refused names its reason and the node.
    #[test]
    fn a_plan_names_one_short_gate_per_node_not_yet_done() {
        let gated = |gate: serde_json::Value| {
            let mut fix = serde_json::to_value(node("fix", NodeKind::Change, &[])).unwrap();
            fix["gate"] = gate;
            plan(vec![serde_json::from_value(fix).unwrap()])
        };
        let refused = |mission: &Mission, gate: serde_json::Value| {
            crate::coordination::split_refusal(&apply_plan(mission, &gated(gate), 2).unwrap_err())
                .0
                .to_owned()
        };
        let base = mission();
        let at = "가".repeat(crate::mission_criteria::GATE_CHARS);
        let planned = apply_plan(&base, &gated(serde_json::json!(at)), 2).unwrap();
        let line = crate::mission_criteria::gate(&planned.nodes[0].criteria).unwrap();
        assert_eq!(
            (line.key.as_str(), line.text.as_deref(), line.state),
            (
                "human",
                Some(at.as_str()),
                crate::mission_criteria::CriterionState::Open
            )
        );
        assert_eq!(
            refused(&base, serde_json::json!(format!("{at}가"))),
            "mission_gate_too_long"
        );
        assert_eq!(
            refused(&base, serde_json::json!(["the map", "the panel"])),
            "mission_gate_too_many"
        );
        assert_eq!(
            refused(&base, serde_json::json!("  ")),
            "mission_gate_invalid"
        );
        let mut exempt = base.clone();
        exempt.exempt = true;
        assert_eq!(
            refused(&exempt, serde_json::json!("the map")),
            "mission_gate_exempt"
        );
        // Left out, a replan keeps it; null takes it away.
        let kept =
            apply_plan(&planned, &plan(vec![node("fix", NodeKind::Change, &[])]), 3).unwrap();
        assert_eq!(kept.nodes[0].criteria, planned.nodes[0].criteria);
        let cleared = apply_plan(&planned, &gated(serde_json::Value::Null), 3).unwrap();
        assert!(crate::mission_criteria::gate(&cleared.nodes[0].criteria).is_none());
        // A done node keeps the gate it has and takes no other.
        let mut done = planned.clone();
        done.nodes[0].status = NodeStatus::Done;
        assert!(apply_plan(&done, &gated(serde_json::json!(at)), 3).is_ok());
        assert_eq!(
            refused(&done, serde_json::json!("the panel")),
            "mission_gate_node_done"
        );
        assert_eq!(
            refused(&done, serde_json::Value::Null),
            "mission_gate_node_done"
        );
        // An outside node names nothing a person or a worker does.
        let mut outside = serde_json::to_value(node("up", NodeKind::External, &[])).unwrap();
        outside["delivery"] = serde_json::Value::Null;
        outside["criteria"] = serde_json::json!([]);
        outside["external"] = serde_json::json!({"pr": 4});
        outside["gate"] = serde_json::json!("the map");
        assert_eq!(
            crate::coordination::split_refusal(
                &apply_plan(
                    &base,
                    &plan(vec![serde_json::from_value(outside).unwrap()]),
                    2
                )
                .unwrap_err()
            )
            .0,
            "mission_external_invalid"
        );
    }

    /// PRD mission-node-simple D-16, B26: a failed node of a closed
    /// mission keeps its meaning too, in progress with the operator asked.
    #[test]
    fn a_closed_missions_failed_node_reads_as_in_progress_with_the_operator_asked() {
        let mut stored = serde_json::to_value(mission()).unwrap();
        stored["status"] = serde_json::json!("done");
        stored["updated_at_unix_ms"] = serde_json::json!(50);
        stored["nodes"] = serde_json::json!([{"key": "f", "title": "f", "kind": "change",
            "status": "failed", "delivery": "pr", "pr": null, "pr_seen": null}]);
        let book = serde_json::json!({"schema_version": 2, "next": 1, "missions": [stored]});
        let read = decode(&serde_json::to_vec(&book).unwrap()).unwrap().book;
        let node = &read.missions[0].nodes[0];
        assert_eq!(node.status, NodeStatus::Working);
        assert_eq!(
            node.attention.as_ref().map(|attention| attention.reason),
            Some(AttentionReason::ConductorFailed)
        );
    }

    /// PRD mission-node-simple B26: an old book whose copy cannot be kept
    /// is not read as converted, so nothing writes the new schema over it.
    #[cfg(unix)]
    #[test]
    fn an_old_book_whose_copy_cannot_be_kept_is_left_as_it_is() {
        use std::os::unix::fs::PermissionsExt;
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version": 2, "next": 1, "missions": []}),
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        // An account that writes past permissions cannot fail the copy.
        if std::fs::write(folder.path().join("probe"), b"").is_ok() {
            std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            return;
        }
        let restored = MissionStore::new(path.clone()).restore();
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(restored, Restored::Unreadable(_)));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    /// B14, B27: a cause raised once is never forgotten; past the cap a new
    /// one is not remembered, and a newer resume mark replaces the older.
    #[test]
    fn a_node_forgets_no_cause_it_raised_and_refuses_one_it_cannot_remember() {
        let one = |key: &str| {
            apply_plan(&mission(), &plan(vec![node(key, NodeKind::Change, &[])]), 2)
                .unwrap()
                .nodes[0]
                .clone()
        };
        let mut node = one("a");
        for index in 0..HANDLED_LIMIT {
            assert!(node.note_handled(format!("cause:{index}")));
        }
        assert!(!node.note_handled("cause:new".to_owned()));
        assert!(node.note_handled("cause:0".to_owned()));
        assert_eq!(node.handled.len(), HANDLED_LIMIT);
        assert_eq!(node.handled[0], "cause:0");
        // The resume mark does not live among the causes: a full list keeps
        // it, and a newer one replaces it.
        node.resumed = Some(node.resumed_cause());
        assert!(node.resumed_on_these_facts());
        node.prs = vec![7];
        assert!(!node.resumed_on_these_facts());
        // Back to work, Hide's acts of the next finish are a new round.
        let mut node = one("b");
        node.back_to_work(5);
        assert_eq!(node.round, 1);
    }

    /// A mission created before verification strength keeps the checkers
    /// it recorded, as written, and has no strength until a plan names one.
    #[test]
    fn a_mission_from_before_strength_keeps_its_recorded_checkers() {
        let recorded = serde_json::json!({
            "review": {"mode": "one", "rule": "all", "count": 2, "agent": "codex",
                       "model": null, "effort": "high", "other_vendor": false},
            "verify": {"mode": "none", "rule": "all", "count": 2, "agent": null,
                       "model": null, "effort": null, "other_vendor": true},
        });
        let mut stored = serde_json::to_value(mission()).unwrap();
        stored["checkers"] = recorded.clone();
        let read: Mission = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(read.strength, None);
        assert_eq!(serde_json::to_value(&read).unwrap(), stored);
        let mut input = plan(Vec::new());
        input.strength = Some(Strength::Thorough);
        let planned = apply_plan(&read, &input, 2).unwrap();
        assert_eq!(planned.strength, Some(Strength::Thorough));
        assert_eq!(planned.checkers, Some(recorded));
    }

    /// A question's View link recorded before links kept their View, or
    /// before they kept its agent, reads with nothing kept and writes back as
    /// it was, without a schema raise (PRD mission-human-gate D-20).
    #[test]
    fn a_view_link_recorded_before_links_kept_their_view_reads_as_it_was() {
        let recorded = serde_json::json!({
            "device_id": "local", "checkout_path": "/repo", "display_id": "d3",
            "at_unix_ms": 10,
        });
        let read: ViewLink = serde_json::from_value(recorded.clone()).unwrap();
        assert_eq!(read.kept, None);
        assert_eq!(serde_json::to_value(&read).unwrap(), recorded);
        let kept = ViewLink {
            kept: Some(KeptView {
                kind: crate::view_layout::DisplayKind::Diff,
                label: "a.rs".into(),
                address: "/repo/a.rs".into(),
                committed: true,
                opener: None,
            }),
            ..read
        };
        let written = serde_json::to_value(&kept).unwrap();
        assert_eq!(
            written["kept"],
            serde_json::json!({"kind": "diff", "label": "a.rs", "address": "/repo/a.rs", "committed": true})
        );
        assert_eq!(serde_json::from_value::<ViewLink>(written).unwrap(), kept);

        // A link kept before it kept its View's agent reads with none and
        // writes back as it was; one that kept it keeps every field.
        let before_opener = serde_json::json!({
            "device_id": "local", "checkout_path": "/repo", "display_id": "d3", "at_unix_ms": 10,
            "kept": {"kind": "file", "label": "notes.md", "address": "/repo/notes.md"},
        });
        let read: ViewLink = serde_json::from_value(before_opener.clone()).unwrap();
        assert_eq!(read.kept.as_ref().unwrap().opener, None);
        assert_eq!(serde_json::to_value(&read).unwrap(), before_opener);
        let opener = serde_json::json!({
            "device_id": "mini", "terminal_id": "t-4", "session": "s-digest", "resume": "s-1",
            "provider": "claude", "name": "dev-a",
        });
        let mut with_opener = before_opener;
        with_opener["kept"]["opener"] = opener.clone();
        let read: ViewLink = serde_json::from_value(with_opener.clone()).unwrap();
        let kept_opener = read.kept.as_ref().unwrap().opener.as_ref().unwrap();
        assert_eq!(
            (
                kept_opener.device_id.as_str(),
                kept_opener.terminal_id.as_str(),
                kept_opener.session.as_deref()
            ),
            ("mini", "t-4", Some("s-digest"))
        );
        assert_eq!(serde_json::to_value(&read).unwrap(), with_opener);
    }

    #[test]
    fn settings_refuse_an_agent_or_effort_the_agent_does_not_take() {
        let mut settings = ConductorSettings::default();
        assert!(settings.validate().is_ok());
        settings.concurrency = 0;
        assert!(settings.validate().is_err());
        settings.concurrency = 4;
        settings.candidates.push(Candidate {
            agent: "opencode".into(),
            profile: None,
            model: None,
            effort: Some("high".into()),
            purpose: String::new(),
        });
        assert_eq!(
            settings.validate().unwrap_err(),
            "conductor_settings.effort_invalid"
        );
        settings.candidates[0] = Candidate {
            agent: "claude".into(),
            profile: None,
            model: Some("opus".into()),
            effort: Some("high".into()),
            purpose: "screens".into(),
        };
        assert!(settings.validate().is_ok());
    }

    /// A candidate kept before profiles names its agent alone and reads as
    /// the agent's built-in default item, written back byte for byte, in
    /// Settings › Mission and in the copy a mission keeps (PRD
    /// agent-profiles-phase2 B14, D-12); one that names a profile keeps it,
    /// and the built-in default named by its id is kept as no profile.
    #[test]
    fn a_candidate_kept_before_profiles_reads_as_its_agents_built_in_default() {
        let kept = r#"{"agent":"claude","model":"opus","effort":null,"purpose":"changes"}"#;
        let candidate: Candidate = serde_json::from_str(kept).unwrap();
        assert_eq!(
            candidate.profile_id(),
            hide_agent_adapter::ProfileId::default_profile()
        );
        assert_eq!(serde_json::to_string(&candidate).unwrap(), kept);
        let mut stored = serde_json::to_value(mission()).unwrap();
        stored["candidates"] =
            serde_json::json!([serde_json::from_str::<serde_json::Value>(kept).unwrap()]);
        let copied: Mission = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(copied.candidates, vec![candidate.clone()]);
        assert_eq!(serde_json::to_value(&copied).unwrap(), stored);

        let work =
            r#"{"agent":"claude","profile":"p-work","model":null,"effort":null,"purpose":""}"#;
        let named: Candidate = serde_json::from_str(work).unwrap();
        assert_eq!(named.profile_id().0, "p-work");
        assert_eq!(serde_json::to_string(&named).unwrap(), work);

        let mut settings = ConductorSettings::default();
        settings.candidates.push(Candidate {
            profile: Some(hide_agent_adapter::ProfileId::default_profile()),
            ..candidate.clone()
        });
        let edited = settings.edited().unwrap();
        assert_eq!(edited.candidates, vec![candidate]);
    }

    /// A Conductor recorded before profiles names none and reads as the
    /// built-in default's; one that names a profile keeps it (PRD
    /// agent-profiles D-27). An older build reads both, the field unknown to
    /// it left out.
    #[test]
    fn a_conductor_recorded_before_profiles_reads_as_the_built_in_default() {
        let old = serde_json::json!({"actor": serde_json::to_value(actor("w1:p1")).unwrap()});
        let conductor: Conductor = serde_json::from_value(old).unwrap();
        assert_eq!(conductor.profile, None);
        assert!(
            serde_json::to_value(&conductor)
                .unwrap()
                .get("profile")
                .is_none()
        );

        let work = Conductor {
            profile: Some(crate::launch::LaunchProfile {
                id: hide_agent_adapter::ProfileId("p-work".into()),
                name: "work".into(),
                account: hide_agent_adapter::AccountKey::Folder("/accounts/work".into()),
            }),
            ..conductor.clone()
        };
        let saved = serde_json::to_value(&work).unwrap();
        assert_eq!(serde_json::from_value::<Conductor>(saved).unwrap(), work);
        assert!(!conductor.profile_unknown, "a mission kept before profiles");
        let unknown = Conductor {
            profile_unknown: true,
            ..conductor
        };
        let saved = serde_json::to_value(&unknown).unwrap();
        assert_eq!(saved["profile_unknown"], true);
        assert_eq!(serde_json::from_value::<Conductor>(saved).unwrap(), unknown);
    }
}
