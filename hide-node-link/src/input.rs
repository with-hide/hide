//! Input a node hands to a running agent through the agent's own way in
//! (`protocol::Call::Deliver`; `docs/delivery.md`, The input ladder).

use serde::{Deserialize, Serialize};

/// The agent's own way into one running session, with the address the core
/// knows for it. Its `Debug` leaves out the token, so no log carries it.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "api", rename_all = "snake_case")]
pub enum InputChannel {
    /// `codex queue --thread <thread> --message <text>` with the account's
    /// Codex.
    CodexQueue { thread: String },
    /// A Claude Code session's messaging socket and the token its
    /// SessionStart hook received.
    ClaudeMessaging { socket: String, token: String },
}

impl std::fmt::Debug for InputChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CodexQueue { thread } => formatter
                .debug_struct("CodexQueue")
                .field("thread", thread)
                .finish(),
            Self::ClaudeMessaging { socket, .. } => formatter
                .debug_struct("ClaudeMessaging")
                .field("socket", socket)
                .finish_non_exhaustive(),
        }
    }
}

impl InputChannel {
    /// The rung's name in a log line.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CodexQueue { .. } => "codex_queue",
            Self::ClaudeMessaging { .. } => "claude_messaging",
        }
    }
}

/// What handing input through a channel did.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Delivery {
    /// The input was sent whole: `codex queue` queued it, or the session's
    /// socket took the write. Neither says the agent took it; the agent's
    /// own record does.
    Sent,
    /// Nothing was sent: the way in cannot be used here or now (no such
    /// program, a system it was not proven on, a refusal). The next rung
    /// takes the input. `reason` is for the log only.
    Unavailable { reason: String },
    /// The send began and its answer was lost (a timeout, a write cut
    /// short), so the input may have reached the agent: it is never sent
    /// again blindly. `reason` is for the log only.
    Unknown { reason: String },
}
