//! The `desk` and `desk_live` snapshot sections (PRD Desk 1차 B1-B13): the
//! Desk room's agent, conversation, approval cards and settings, and the
//! reply being written. The core decides every state here; the shell words
//! them and dispatches the actions.
//!
//! The reply being written is its own section so a streamed chunk resends
//! that reply alone, never the conversation above it.

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
    },
    /// The operator switched agents; `carried` says how much of the
    /// conversation went with the switch.
    Switch {
        id: u64,
        from: Option<AdapterId>,
        to: AdapterId,
        carried: DeskCarried,
    },
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
            Self::Reply { text, .. } => text.len(),
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
}
