//! The entire external usage vocabulary. No caller can attach content or IDs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Connected,
    AiCalled,
    /// A native agent session a Hide pane carried for the first time.
    AgentStarted(hide_session::Agent),
}

impl Event {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connected => "hide_connected",
            Self::AiCalled => "hide_ai_called",
            Self::AgentStarted(_) => "hide_agent_started",
        }
    }

    /// The `agent` property, spelled here so an adapter's rename cannot
    /// change what the wire says.
    pub fn agent(self) -> Option<&'static str> {
        let Self::AgentStarted(agent) = self else {
            return None;
        };
        Some(match agent {
            hide_session::Agent::Codex => "codex",
            hide_session::Agent::Claude => "claude",
            hide_session::Agent::Grok => "grok",
            hide_session::Agent::Pi => "pi",
            hide_session::Agent::Omp => "omp",
            hide_session::Agent::Cursor => "cursor",
            hide_session::Agent::OpenCode => "opencode",
            hide_session::Agent::Antigravity => "antigravity",
        })
    }
}

/// A bounded, nonblocking enqueue only; the daemon owns transport and shutdown.
pub trait Sink: Send + Sync + std::fmt::Debug {
    fn set_enabled(&self, enabled: bool);
    fn capture(&self, event: Event);
}
