//! The entire external usage vocabulary. No caller can attach content or IDs.

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Connected,
    AiCalled,
}

impl Event {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connected => "hide_connected",
            Self::AiCalled => "hide_ai_called",
        }
    }
}

/// A bounded, nonblocking enqueue only; the daemon owns transport and shutdown.
pub trait Sink: Send + Sync + std::fmt::Debug {
    fn set_enabled(&self, enabled: bool);
    fn capture(&self, event: Event);
}
