//! Input a node hands to a running agent through the agent's own way in
//! (`protocol::Call::Deliver`; `docs/delivery.md`, The input ladder).

use serde::{Deserialize, Serialize};

/// The agent's own way into one running session, with the address the core
/// knows for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "api", rename_all = "snake_case")]
pub enum InputChannel {
    /// `codex queue --thread <thread> --message=<text>` with the account's
    /// Codex.
    CodexQueue { thread: String },
}

impl InputChannel {
    /// The rung's name in a log line.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CodexQueue { .. } => "codex_queue",
        }
    }
}

/// What handing input through a channel did.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Delivery {
    /// The input was sent whole: `codex queue` queued it. That does not say
    /// the agent took it; the agent's own record does.
    Sent,
    /// Nothing was sent: the way in cannot be used here or now (no such
    /// program, a refusal). The next rung
    /// takes the input. `reason` is for the log only.
    Unavailable { reason: String },
    /// The send began and its answer was lost (a timeout, a write cut
    /// short), so the input may have reached the agent: it is never sent
    /// again blindly. `reason` is for the log only.
    Unknown { reason: String },
}
