//! The entire external usage vocabulary. No caller can attach content or IDs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Connected,
    AiCalled,
    /// A native agent session a Hide pane carried for the first time.
    AgentStarted(hide_session::Agent),
    DeviceConnected {
        outcome: DeviceOutcome,
        platform: DevicePlatform,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceOutcome {
    Ready(DeviceHerdr),
    Failed(DeviceFailedStep),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceHerdr {
    Started,
    HandedOff,
    Current,
    Kept,
    /// An explicitly selected saved session whose server Hide leaves unchanged.
    Preserved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFailedStep {
    Install,
    Download,
    HerdrStart,
    HerdrHandoff,
}

/// Unknown means establishment failed before the helper reported its system.
/// Never forward an unrecognized platform string from a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevicePlatform {
    LinuxX86_64,
    LinuxAarch64,
    MacosX86_64,
    MacosAarch64,
    Unknown,
}

impl DevicePlatform {
    pub(crate) fn of(os: &str, arch: &str) -> Self {
        match (os, arch) {
            ("linux", "x86_64") => Self::LinuxX86_64,
            ("linux", "aarch64") => Self::LinuxAarch64,
            ("macos", "x86_64") => Self::MacosX86_64,
            ("macos", "aarch64") => Self::MacosAarch64,
            _ => Self::Unknown,
        }
    }
}

impl Event {
    pub fn name(self) -> &'static str {
        match self {
            Self::Connected => "hide_connected",
            Self::AiCalled => "hide_ai_called",
            Self::AgentStarted(_) => "hide_agent_started",
            Self::DeviceConnected { .. } => "hide_device_connected",
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

    pub fn outcome(self) -> Option<&'static str> {
        match self {
            Self::DeviceConnected {
                outcome: DeviceOutcome::Ready(_),
                ..
            } => Some("ready"),
            Self::DeviceConnected {
                outcome: DeviceOutcome::Failed(_),
                ..
            } => Some("failed"),
            _ => None,
        }
    }

    pub fn failed_step(self) -> Option<&'static str> {
        let Self::DeviceConnected {
            outcome: DeviceOutcome::Failed(step),
            ..
        } = self
        else {
            return None;
        };
        Some(match step {
            DeviceFailedStep::Install => "install",
            DeviceFailedStep::Download => "download",
            DeviceFailedStep::HerdrStart => "herdr_start",
            DeviceFailedStep::HerdrHandoff => "herdr_handoff",
        })
    }

    pub fn herdr(self) -> Option<&'static str> {
        let Self::DeviceConnected {
            outcome: DeviceOutcome::Ready(herdr),
            ..
        } = self
        else {
            return None;
        };
        Some(match herdr {
            DeviceHerdr::Started => "started",
            DeviceHerdr::HandedOff => "handed_off",
            DeviceHerdr::Current => "current",
            DeviceHerdr::Kept => "kept",
            DeviceHerdr::Preserved => "preserved",
        })
    }

    pub fn platform(self) -> Option<&'static str> {
        let Self::DeviceConnected { platform, .. } = self else {
            return None;
        };
        Some(match platform {
            DevicePlatform::LinuxX86_64 => "linux-x86_64",
            DevicePlatform::LinuxAarch64 => "linux-aarch64",
            DevicePlatform::MacosX86_64 => "macos-x86_64",
            DevicePlatform::MacosAarch64 => "macos-aarch64",
            DevicePlatform::Unknown => "unknown",
        })
    }
}

/// A bounded, nonblocking enqueue only; the daemon owns transport and shutdown.
pub trait Sink: Send + Sync + std::fmt::Debug {
    fn set_enabled(&self, enabled: bool);
    fn capture(&self, event: Event);
}
