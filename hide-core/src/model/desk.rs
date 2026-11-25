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
    /// What Desk's agent opened with `hide file open`, oldest first, waiting
    /// for a window showing Desk to open it in the tab panel and send
    /// `desk_open_taken`; at most 16, the oldest dropped, kept in memory
    /// only.
    pub opens: Vec<DeskOpenSnapshot>,
}

/// A file Desk's agent opened for the operator: its machine, its checkout's
/// folder as the catalog names it, and its path, checked inside that
/// checkout on its own host.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskOpenSnapshot {
    pub id: u64,
    pub device_id: String,
    pub root: String,
    pub path: String,
}

/// The widths of Desk's two panels as the operator last dragged them (PRD
/// desk-2-ui D-06, D-07, B22, B23), in CSS pixels; `None` until dragged, so
/// the shell draws its own first width. Which session is open beside Desk,
/// whether the tab panel shows and which tab is in front are the window's,
/// never the core's (B26).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeskLane {
    /// The tab panel's width: its left edge.
    pub view_width: Option<u32>,
    /// The session panel's width: the edge between it and the tab panel.
    pub session_width: Option<u32>,
}

/// The widest a stored panel width may be; a wider one is refused
/// `desk_lane_width_invalid`, as is zero.
pub const DESK_LANE_WIDTH_MAX: u32 = 8192;

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
    /// The other sessions the newest lines of the conversation name (a
    /// reply's chips, a letter's line), as cards of their own read from
    /// their sidebar rows, so a chip's hover card and its thread show what
    /// the row shows (D-86); none is drawn in the chat or listed in Threads.
    pub named: Vec<DeskJobSnapshot>,
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

/// The mission a job conducts, as its card's `미션 k/n` chip draws it; the
/// chip's colour is the screen's, from `status` and `turn`, by the rule
/// every mission chip follows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeskJobMission {
    pub id: String,
    pub title: String,
    pub status: crate::missions::MissionStatus,
    pub done: usize,
    pub total: usize,
    /// What of the open mission waits on the operator, the chip's `!`.
    pub turn: Option<DeskMissionTurn>,
    /// The delay holding the open mission, the chip's grey pause.
    pub delay: Option<String>,
    /// When that delay ends by itself, said by the chip's name with its reason.
    pub delay_until_unix_ms: Option<u64>,
}

/// What of an open mission waits on the operator, as every mission chip
/// marks it (`web/src/missions.ts` `missionTurn`): its decision, else a node
/// raised to the operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskMissionTurn {
    Decision,
    Attention,
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
    /// The operator moved its tab out of the reserved Desk group (B37).
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
    /// Hide's hook for the agent is not in place where it would run, so it
    /// would start unconnected; that machine's row in Settings says why.
    AgentHookNotReady,
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
            code if crate::runtime::start_hook::HookRefusal::is_code(code) => {
                Self::AgentHookNotReady
            }
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
            | "beside_with_parent"
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
    /// The pane of the session it was spawned from, always one the menu
    /// lists too, so the menu folds it under that parent (D-84).
    pub parent: Option<String>,
    /// The project its pane sits in, as the sidebar names it.
    pub project: String,
    /// The branch of the checkout its pane sits in; none for a folder or a
    /// pane the navigator has not placed.
    pub branch: Option<String>,
    /// Its latest request, the time the sidebar row shows.
    pub last_prompt_at_unix_ms: Option<u64>,
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

/// One agent of Desk's bar: an agent switched on in Hide's catalog that Desk
/// speaks ACP with (D-79), or the selected one after it was switched off.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeskAgentSnapshot {
    pub id: AdapterId,
    /// The catalog's name for it (`Claude Code`, `omp`).
    pub label: &'static str,
    /// Whether Desk can start it: its adapter, its own CLI, its ask mode.
    pub state: DeskAgentState,
    /// What the state names: the Node version found, the install's last
    /// line, the CLI that is missing.
    pub detail: Option<String>,
    /// Whether the operator is signed in to the agent's own CLI, as Settings
    /// reads it (Hide AI's read of that agent on this machine, D-81): `None`
    /// until the read answered, or where it cannot tell. It never follows
    /// the adapter.
    pub signed_in: Option<bool>,
    /// The command that signs the agent in, shown with a `login_needed`
    /// notice.
    pub login_command: &'static str,
    /// The rows of Settings › Labs › Desk › 권한 the agent runs without asking
    /// (`AdapterId::always_runs`), shown on those rows.
    pub always_runs: &'static [hide_desk::permission::Row],
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
    /// Its adapter or its own CLI is in place; a message starts it.
    Ready,
    /// Its pinned adapter is being installed, which Desk turned on starts
    /// for every agent at once (D-80).
    Preparing,
    InstallFailed,
    /// No Node 22 or later on the account's `PATH` (B6).
    NodeMissing,
    /// The agent's own CLI is not on the account's `PATH`.
    CliMissing,
    /// Desk cannot hold it in an ask mode, so it is not used (D-53).
    NoAskMode,
    /// Switched off in Settings › Agents while it is still the one chosen:
    /// listed so the bar can name it, and not sent to.
    Off,
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
    /// Messages the operator sent while a reply was being written, in the
    /// order they go: none is in `items` until its turn starts.
    pub queued: Vec<DeskQueuedMessage>,
    /// How many messages may wait; a send past it is refused
    /// `desk_queued_full` and its words stay in the composer.
    pub queued_max: usize,
    /// The line is full: the screen says so, and ✕ on one frees a place.
    pub queued_full: bool,
    /// The line waits for the operator: the agent refused the message at
    /// its head (a sign-in, a session that did not open), so nothing goes
    /// until `다시 시작` or a new message.
    pub queued_held: bool,
}

/// A message waiting for the turn before it to end (desk.md: "while a reply
/// is being written, the next message waits"); `desk_queued_remove` takes
/// it out before it goes. It is the whole message: its words, chips and
/// files go together, whether it waits from the start or came back first in
/// line because the agent never took it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskQueuedMessage {
    pub id: u64,
    pub text: String,
    pub at_ms: u64,
    pub mentions: Vec<DeskMention>,
    /// The files sent with it, which reach the agent as paths when it goes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<DeskAttachment>,
}

/// Messages that wait at once; one more is refused `desk_queued_full`.
pub const DESK_QUEUED_MAX: usize = 8;

/// One line of the conversation, kept in Desk's store as it is sent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeskItem {
    User {
        id: u64,
        text: String,
        at_ms: u64,
        mentions: Vec<DeskMention>,
        /// The files sent with the message, drawn on its bubble; the agent
        /// got their paths before the words.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<DeskAttachment>,
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
        /// The missions the reply names as `@mission(<id> <title>)`, each
        /// the app's mission chip.
        #[serde(default)]
        missions: Vec<DeskMissionMention>,
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
        /// The folder the command runs in, or that holds the files an edit
        /// changes; none where Hide cannot name one.
        folder: Option<String>,
        reason: Option<String>,
        /// The row of Settings › Labs › Desk › 권한 the request reads as,
        /// whose name is the card's kind (`hide 명령` for a `hide` line);
        /// none on a card kept from before it was sent.
        #[serde(default)]
        row: Option<DeskRow>,
        state: DeskApprovalState,
        /// Why it asks, irreversible kinds first (D-82): what
        /// `이 대화에서 계속 허용` keeps allowed. Empty on a card kept from
        /// before the table.
        #[serde(default)]
        asks: Vec<DeskAsk>,
        /// What the first irreversible kind is about: the working session's
        /// title, or the path outside every project.
        #[serde(default)]
        subject: Option<String>,
        /// The line the operator sent Desk with `거절…` (D-83).
        #[serde(default)]
        refusal: Option<String>,
    },
    Notice {
        id: u64,
        notice: DeskNotice,
        /// The agent the notice is about.
        agent: Option<AdapterId>,
        /// What the notice names: the install's last line, the sign-in
        /// command for `login_needed`, the call that was blocked, the
        /// agent's own reason for `turn_failed` when it gave one.
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
            Self::User {
                text,
                mentions,
                attachments,
                ..
            } => {
                text.len()
                    + mentions
                        .iter()
                        .map(|mention| mention.id.len() + mention.name.len())
                        .sum::<usize>()
                    + attachments
                        .iter()
                        .map(|file| file.name.len() + file.path.len())
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
                subject,
                refusal,
                ..
            } => {
                command.len()
                    + [folder, reason, subject, refusal]
                        .into_iter()
                        .map(|text| text.as_ref().map_or(0, String::len))
                        .sum::<usize>()
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

/// A file the operator dropped, pasted or picked into Desk's composer
/// (PRD desk-2-ui B41): its name and size as the composer showed them, and
/// the path the window's hided staged it at, which the agent was given.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskAttachment {
    pub name: String,
    pub bytes: u64,
    pub path: String,
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

/// A mission a reply names: its id, and the title the reply wrote, shown
/// while the app holds no mission of that id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeskMissionMention {
    pub id: String,
    pub title: String,
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
    /// A file read or search, asked when the operator set reads to ask.
    Read,
    Other,
}

/// Why a card asks (`hide_desk::permission::Ask`).
pub type DeskAsk = hide_desk::permission::Ask;

/// A row of Settings › Labs › Desk › 권한 (`hide_desk::permission::Row`).
pub type DeskRow = hide_desk::permission::Row;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskApprovalState {
    Waiting,
    /// `한 번 허용`.
    Allowed,
    /// `이 대화에서 계속 허용`: the same kinds run without a card until the
    /// Desk process ends (D-83).
    AllowedConversation,
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
    /// The turn failed; the detail is the agent's own one-line reason, when
    /// it gave one.
    TurnFailed,
    /// A file the message named was gone, so the agent was not asked and the
    /// message went back to the operator's input (PRD desk-2-ui B41).
    AttachmentsGone,
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
    /// Settings › Labs › Desk's switch (D-76): off by default. While off no entry
    /// to Desk is drawn, its process does not run and every `desk_*` event
    /// but this setting is refused; its sessions stay ordinary sessions.
    #[serde(default)]
    pub enabled: bool,
    /// The agent a new conversation starts with.
    pub default_agent: Option<AdapterId>,
    pub location: DeskLocation,
    /// Sessions Desk runs at once; more wait in its queue.
    pub cap: u8,
    /// Settings › Labs › Desk › 권한 (D-82): every row 바로 in a store written
    /// before it existed.
    #[serde(default)]
    pub permissions: hide_desk::permission::Table,
}

impl Default for DeskSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_agent: None,
            location: DeskLocation::Desk,
            cap: DESK_CAP_DEFAULT,
            permissions: hide_desk::permission::Table::default(),
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
    /// What the turn's newest tool call does while the agent has written
    /// nothing since it, named as `DeskSteps` counts it: the line under the
    /// reply says it until the turn ends.
    pub step: Option<DeskStepKind>,
}

/// One of `DeskSteps`' kinds, by the name its count has there.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskStepKind {
    Sessions,
    Files,
    Letters,
    Starts,
    Other,
}

impl From<hide_desk::permission::Step> for DeskStepKind {
    fn from(step: hide_desk::permission::Step) -> Self {
        use hide_desk::permission::Step;
        match step {
            Step::Sessions => Self::Sessions,
            Step::Files => Self::Files,
            Step::Letter => Self::Letters,
            Step::Start => Self::Starts,
            Step::Other => Self::Other,
        }
    }
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
