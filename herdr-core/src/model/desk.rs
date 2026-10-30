//! The Desk snapshot sections (PRD Desk 1차): `desk`, the room's agent,
//! conversation, approval cards and settings; `desk_live`, the reply being
//! written; `desk_jobs`, the sessions Desk started; and `desk_mentions`, the
//! `@` menu. The core decides every state here; the shell words them and
//! dispatches the actions.
//!
//! Each moves on its own, so a streamed chunk or a card's change never
//! resends the conversation.

use hide_desk::AdapterId;
use serde::{Deserialize, Serialize};

/// The section: absent until Desk is installed in the daemon.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskSnapshot {
    pub selection: DeskSelection,
    /// Every agent Desk can talk to, in the order the selector lists them.
    pub agents: Vec<DeskAgentSnapshot>,
    pub host: DeskHostSnapshot,
    pub conversation: DeskConversationSnapshot,
    /// Approval cards waiting for the operator.
    pub approvals_waiting: usize,
    pub settings: DeskSettings,
    pub lane: DeskLane,
}

/// Desk's lane as the core keeps it across a restart (D-64, B31, B32): its
/// pin, and the tabs the operator kept in its 보기 column. Which columns
/// show, the preview tab and the thread in the tool column are the window's.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskLane {
    pub pinned: bool,
    /// Oldest first, at most `DESK_KEPT_TABS_MAX`.
    pub kept: Vec<DeskKeptTab>,
}

/// Tabs the 보기 column keeps; keeping one more is refused `desk_tabs_full`.
pub const DESK_KEPT_TABS_MAX: usize = 16;

/// A tab kept in Desk's 보기 column: a job's output as its card listed it
/// when it was kept, or a mission's map (B34).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeskKeptTab {
    Output { job: String, output: DeskJobOutput },
    Mission { mission: String, title: String },
}

impl DeskKeptTab {
    /// The tab's identity: one tab per output of a job, one per mission.
    pub fn key(&self) -> String {
        match self {
            Self::Output { job, output } => format!("output:{job}:{}", output.id),
            Self::Mission { mission, .. } => format!("mission:{mission}"),
        }
    }
}

/// The `desk_jobs` section: the sessions Desk started and the ones waiting
/// in its queue, newest first (B20-B24, B36-B39).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskJobsSnapshot {
    pub jobs: Vec<DeskJobSnapshot>,
    /// The workers of the missions Desk's jobs conduct, as cards of their
    /// own, so the Threads list opens a Conductor to them and a mission map's
    /// node opens a worker's thread (B28, B34); none is Desk's job, and none
    /// is drawn in the chat.
    pub workers: Vec<DeskJobSnapshot>,
}

/// One session Desk started, or one waiting to start.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskJobSnapshot {
    /// The spawn's intent, stable from the queue to the end.
    pub id: String,
    /// The ledger agent once it registered.
    pub agent_id: Option<String>,
    pub pane_id: Option<String>,
    pub title: String,
    pub agent_kind: String,
    pub project: DeskJobProject,
    pub location: DeskJobLocation,
    pub state: DeskJobState,
    /// Its place in the queue, from 1, while `queued`.
    pub queue_position: Option<usize>,
    /// What the session does now: its pane's second line.
    pub now: Option<String>,
    pub plan: Option<DeskJobPlan>,
    /// What it asks the operator, while `asking`.
    pub question: Option<String>,
    /// The last turn's summary line, once `done`.
    pub summary: Option<String>,
    pub pull_requests: Vec<DeskPullRequest>,
    pub outputs: Vec<DeskJobOutput>,
    /// The operator typed into its terminal (B36).
    pub joined: bool,
    pub reason: Option<DeskJobReason>,
    /// Why its start failed, for the failed card's words.
    pub failure: Option<DeskJobFailure>,
    /// What the operator's turn is about while it is `asking`.
    pub ask: Option<DeskAskKind>,
    /// The sidebar row's own mark, drawn by the same `StatusMark`; absent
    /// while it has no row (queued, failed, closed).
    pub mark: Option<crate::agent_state::RowMark>,
    /// The ask its mark shows was read and is still unanswered (hollow `!`).
    pub mark_read: bool,
    /// The device it runs on, when not this machine (B23).
    pub machine: Option<String>,
    pub disconnected: bool,
    pub changed_at_unix_ms: u64,
    /// The operator saw it since it ended.
    pub read: bool,
    pub mission: Option<DeskJobMission>,
    /// The panes of a Conductor's workers, each its thread's key.
    pub workers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskJobProject {
    pub workspace_id: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskJobLocation {
    pub kind: DeskJobLocationKind,
    /// The branch or worktree folder the card's ⌂ names.
    pub label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskJobLocationKind {
    Checkout,
    Worktree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DeskJobPlan {
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskJobOutput {
    pub id: String,
    pub kind: DeskOutputKind,
    pub label: String,
    pub thumb: Option<String>,
    pub seen: bool,
    /// A pull request's one state, the PR mark's (`PrState::of`), so it
    /// reads the same on a card, in a thread and in 결과물.
    pub pr_state: Option<crate::model::PrState>,
    /// Where a View's file is read from, so the lane's 보기 column shows it
    /// with the file viewers without opening a View of its own (B31): the
    /// device and the checkout whose View list holds it, and the file. A
    /// page and a pull request carry their address instead.
    pub device_id: Option<String>,
    pub checkout: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
}

/// A pull request the job's session holds or made, as its PR mark draws it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskPullRequest {
    pub number: u32,
    pub state: crate::model::PrState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskOutputKind {
    Image,
    Page,
    Doc,
    Pr,
}

/// The mission a job conducts, as its card's `미션 k/n` chip draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskJobMission {
    pub id: String,
    pub title: String,
    pub status: crate::missions::MissionStatus,
    pub done: usize,
    pub total: usize,
    pub tone: String,
    pub deciding: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskJobState {
    Queued,
    Starting,
    Working,
    Asking,
    Done,
    Failed,
    Cancelled,
    /// The operator closed its tab (B38).
    Closed,
    /// The operator moved its tab out of the `◆ Desk` group (B37).
    MovedOut,
    /// Hide put it to sleep (`hide agent sleep`); a letter wakes it.
    Sleeping,
    Archived,
}

/// What an asking job waits on the operator for (D-75).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskAskKind {
    Question,
    Approval,
    /// A Conductor's mission waits on a decision.
    Decision,
    PlanReview,
    /// The agent stopped on an error.
    Blocked,
}

impl DeskJobState {
    /// Whether the job holds one of Desk's places (B21, B22, B37, B38).
    pub fn holds_place(self) -> bool {
        matches!(self, Self::Starting | Self::Working | Self::Asking)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskJobReason {
    SpawnFailed,
    Exited,
    DeviceDisconnected,
}

/// Why a job's start failed, grouped by what the operator can do about it;
/// the start's own code goes to the diagnostic log.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskJobFailure {
    /// The device it was asked of was not connected.
    DeviceUnavailable,
    /// The agent's CLI is not installed where it would run.
    AgentNotInstalled,
    /// Another worktree holds its branch or folder.
    BranchInUse,
    /// Its project or worktree could not be prepared.
    ProjectUnavailable,
    /// A job with the same task key already started.
    AlreadyStarted,
    /// Hide was starting another session; `다시 시도` works.
    Busy,
    /// Hide could not open or confirm the session.
    NotStarted,
    /// Desk asked for a start Hide does not take.
    InvalidRequest,
    Other,
}

impl DeskJobFailure {
    /// The group a start's failure code falls in.
    pub fn of(code: &str) -> Self {
        let code = code
            .split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .next()
            .unwrap_or_default();
        match code {
            "machine_unavailable"
            | "machine_unknown"
            | "device_unreachable"
            | "device_check_failed" => Self::DeviceUnavailable,
            "agent_not_installed" => Self::AgentNotInstalled,
            "branch_in_use" | "checkout_path_conflict" => Self::BranchInUse,
            "repository_unavailable" | "worktree_unconfirmed" => Self::ProjectUnavailable,
            "intent_conflict" => Self::AlreadyStarted,
            "spawn_busy" => Self::Busy,
            "spawn_unavailable"
            | "child_unavailable"
            | "agent_not_started"
            | "snapshot_unavailable"
            | "brief_not_submitted"
            | "native_identity_unavailable"
            | "parent_unavailable"
            | "ledger_unavailable"
            | "delivery_unavailable" => Self::NotStarted,
            "invalid_spawn"
            | "invalid_brief"
            | "invalid_agent_name"
            | "invalid_launch_choice"
            | "beside_other_checkout"
            | "beside_other_machine"
            | "parent_authority_required"
            | "machine_not_permitted" => Self::InvalidRequest,
            _ => Self::Other,
        }
    }
}

/// The `desk_mentions` section, the `@` menu: recent sessions, then
/// projects (B18).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DeskMentionsSnapshot {
    pub sessions: Vec<DeskMentionSession>,
    pub projects: Vec<DeskMentionProject>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskMentionSession {
    pub id: String,
    pub name: String,
    pub agent_kind: String,
    pub mark: crate::agent_state::RowMark,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskMentionProject {
    pub id: String,
    pub name: String,
    /// Its sessions running now, which `⇥` lists (B18).
    pub sessions: Vec<String>,
}

/// The agent, model and effort the next message goes to.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskSelection {
    pub agent: Option<AdapterId>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskAgentSnapshot {
    pub id: AdapterId,
    pub label: &'static str,
    pub state: DeskAgentState,
    /// What the state names: the Node version found, the install's last
    /// line, the CLI that is missing.
    pub detail: Option<String>,
    /// The command that signs the agent in, shown with `login_needed`.
    pub login_command: &'static str,
    /// The models and efforts the agent's last session offered; empty until
    /// one has opened.
    pub models: Vec<DeskChoiceSnapshot>,
    pub efforts: Vec<DeskChoiceSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskChoiceSnapshot {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskAgentState {
    /// Not looked at yet.
    Checking,
    /// Installed; a message starts it.
    Ready,
    /// Its pinned package is being installed.
    Preparing,
    /// Not installed; choosing it installs it.
    NotInstalled,
    InstallFailed,
    /// No Node 22 or later on the account's `PATH` (B6).
    NodeMissing,
    /// The agent's own CLI is not on the account's `PATH` (Pi).
    CliMissing,
    /// The agent needs the operator signed in.
    LoginNeeded,
    /// Its ask mode could not be confirmed, so it is not used (D-53).
    NoAskMode,
    /// Desk does not offer it yet (`AdapterId::offered`).
    NotOffered,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskHostSnapshot {
    pub state: DeskHostState,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskHostState {
    /// No Desk process runs; the next message starts one.
    Idle,
    Starting,
    /// A session is open and waits for a message.
    Ready,
    /// A turn is under way.
    Thinking,
    /// The process ended on its own or crossed its cap; `다시 시작` starts
    /// it again.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskConversationSnapshot {
    /// Bumped by `새 대화`, so the shell drops what it scrolled.
    pub generation: u64,
    pub items: Vec<DeskItem>,
    /// Items dropped from the front to keep the conversation in its cap.
    pub dropped: u64,
}

/// One line of the conversation, kept in Desk's store as it is sent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeskItem {
    User {
        id: u64,
        text: String,
        at_ms: u64,
        mentions: Vec<DeskMention>,
    },
    Reply {
        id: u64,
        agent: AdapterId,
        text: String,
        at_ms: u64,
        state: DeskReplyState,
        /// From the message to the reply's first words: `생각 8초`.
        #[serde(default)]
        thought_ms: Option<u64>,
        /// What the agent did in the turn, folded into one line.
        #[serde(default)]
        steps: DeskSteps,
        /// The sessions the reply names as `@session(<id> <name>)`, each a
        /// chip.
        #[serde(default)]
        mentions: Vec<DeskSessionChip>,
    },
    /// The operator switched agents; `carried` says how much of the
    /// conversation went with the switch.
    Switch {
        id: u64,
        from: Option<AdapterId>,
        to: AdapterId,
        carried: DeskCarried,
    },
    /// A letter Desk sent a session that already runs (B16): work it
    /// handed (`kind` `request`) or word it passed on. It changes no state
    /// and holds no place.
    Handed {
        id: u64,
        agent_id: String,
        /// The recipient's pane, where its row and status mark are.
        #[serde(default)]
        pane_id: String,
        name: String,
        project: Option<String>,
        /// The letter's kind: `request`, `block`, `progress` or `report`.
        #[serde(default)]
        letter_kind: String,
        /// The letter's first line.
        #[serde(default)]
        line: String,
        #[serde(default)]
        letter_id: Option<String>,
        /// The recipient's runtime took the letter in (`읽음`).
        #[serde(default)]
        read: bool,
        #[serde(default)]
        agent_kind: Option<String>,
        /// The recipient's row mark now, for its chip.
        #[serde(default, skip_deserializing)]
        mark: Option<crate::agent_state::RowMark>,
    },
    /// Desk started a job; its card is `jobs`' entry `job_id`.
    Job { id: u64, job_id: String },
    /// The model or effort changed.
    Model {
        id: u64,
        model: Option<String>,
        effort: Option<String>,
    },
    Approval {
        id: u64,
        card: DeskCardKind,
        /// The command, or the files an edit changes, as the agent asked.
        command: String,
        folder: Option<String>,
        reason: Option<String>,
        state: DeskApprovalState,
    },
    Notice {
        id: u64,
        notice: DeskNotice,
        /// The agent the notice is about.
        agent: Option<AdapterId>,
        /// What the notice names: the install's last line, the sign-in
        /// command for `login_needed`, the call that was blocked.
        detail: Option<String>,
    },
}

impl DeskItem {
    pub fn id(&self) -> u64 {
        match self {
            Self::User { id, .. }
            | Self::Reply { id, .. }
            | Self::Switch { id, .. }
            | Self::Model { id, .. }
            | Self::Handed { id, .. }
            | Self::Job { id, .. }
            | Self::Approval { id, .. }
            | Self::Notice { id, .. } => *id,
        }
    }

    /// The bytes the item's text holds, counted against the conversation's
    /// cap.
    pub fn text_bytes(&self) -> usize {
        match self {
            Self::User { text, mentions, .. } => {
                text.len()
                    + mentions
                        .iter()
                        .map(|mention| mention.id.len() + mention.name.len())
                        .sum::<usize>()
            }
            Self::Reply { text, mentions, .. } => {
                text.len()
                    + mentions
                        .iter()
                        .map(|chip| chip.pane_id.len() + chip.name.len())
                        .sum::<usize>()
            }
            Self::Approval {
                command,
                folder,
                reason,
                ..
            } => {
                command.len()
                    + folder.as_ref().map_or(0, String::len)
                    + reason.as_ref().map_or(0, String::len)
            }
            Self::Notice { detail, .. } => detail.as_ref().map_or(0, String::len),
            Self::Model { model, effort, .. } => {
                model.as_ref().map_or(0, String::len) + effort.as_ref().map_or(0, String::len)
            }
            Self::Handed {
                name,
                agent_id,
                project,
                line,
                ..
            } => name.len() + agent_id.len() + project.as_ref().map_or(0, String::len) + line.len(),
            Self::Job { job_id, .. } => job_id.len(),
            Self::Switch { .. } => 0,
        }
    }
}

/// A session or project the operator named with `@`; the message's text
/// holds it as `@session(<id> <name>)` or `@project(<path> <name>)`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskMention {
    pub kind: DeskMentionKind,
    pub id: String,
    pub name: String,
}

/// A session a line of the conversation names, drawn as a chip with the
/// row's mark as the sidebar draws it now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskSessionChip {
    pub pane_id: String,
    pub name: String,
    #[serde(default, skip_deserializing)]
    pub agent_kind: Option<String>,
    #[serde(default, skip_deserializing)]
    pub mark: Option<crate::agent_state::RowMark>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskMentionKind {
    Session,
    Project,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskCarried {
    /// The whole conversation went with the switch.
    All,
    /// Its oldest part was left out to keep it in its cap.
    Trimmed,
    /// There was nothing to carry.
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskReplyState {
    Done,
    /// The operator stopped it, or the process ended under it.
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskCardKind {
    Command,
    Edit,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskApprovalState {
    Waiting,
    Allowed,
    Rejected,
    /// A new conversation, a restart or the process's end closed it
    /// unanswered, which the agent read as a refusal (D-52).
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskNotice {
    /// A call Hide could not read was refused (D-60).
    Blocked,
    /// The agent's adapter is being installed.
    Preparing,
    /// The process ended; `다시 시작` starts it again.
    HostStopped,
    /// The session before could not be resumed, so a new one started.
    ResumeFailed,
    LoginNeeded,
    InstallFailed,
    /// The agent could not be put in its ask mode, so it was not used.
    NoAskMode,
    /// The turn failed; the detail is the agent's error.
    TurnFailed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskLocation {
    /// The agent picks per task, as its instructions say.
    #[default]
    Desk,
    /// Every task on a new worktree.
    Worktree,
    /// Every task in the checkout as it is.
    Checkout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskSettings {
    /// The agent a new conversation starts with.
    pub default_agent: Option<AdapterId>,
    pub location: DeskLocation,
    /// Sessions Desk runs at once; more wait in its queue.
    pub cap: u8,
}

impl Default for DeskSettings {
    fn default() -> Self {
        Self {
            default_agent: None,
            location: DeskLocation::Desk,
            cap: DESK_CAP_DEFAULT,
        }
    }
}

pub const DESK_CAP_DEFAULT: u8 = 8;
pub const DESK_CAP_MAX: u8 = 8;

/// The reply being written: the text so far of the turn under way.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DeskLiveSnapshot {
    pub text: Option<String>,
    /// When the turn being answered started, for `생각 중 · 3초`.
    pub started_at_ms: Option<u64>,
}

/// What Desk's agent did in a turn, counted from its tool calls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskSteps {
    /// `hide` commands that read sessions, letters and plans.
    pub sessions: u32,
    pub files: u32,
    pub letters: u32,
    pub starts: u32,
    pub other: u32,
}

impl DeskSteps {
    pub(crate) fn count(&mut self, step: hide_desk::permission::Step) {
        use hide_desk::permission::Step;
        let slot = match step {
            Step::Sessions => &mut self.sessions,
            Step::Files => &mut self.files,
            Step::Letter => &mut self.letters,
            Step::Start => &mut self.starts,
            Step::Other => &mut self.other,
        };
        *slot = slot.saturating_add(1);
    }
}
