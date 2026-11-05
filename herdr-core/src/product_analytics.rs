//! The entire external usage vocabulary. No caller can attach content or IDs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Connected,
    /// A native agent session a Hide pane carried for the first time, and
    /// who started it; `None` when the delivery ledger could not be read.
    AgentStarted {
        agent: hide_session::Agent,
        source: Option<AgentSource>,
    },
    DeviceConnected {
        outcome: DeviceOutcome,
        platform: DevicePlatform,
    },
    /// A mission's plan was started (Planning to Running), with its size.
    MissionStarted {
        nodes: MissionSize,
    },
    /// A started mission's first close: a reopened one that closes again,
    /// and a waiting one that later lands, add none.
    MissionEnded {
        outcome: MissionOutcome,
    },
    /// A practice chapter reached its first step, or its end, for the first
    /// time on this installation.
    Practice {
        chapter: PracticeChapter,
        stage: PracticeStage,
    },
    /// A phone paired with this core.
    PhonePaired {
        route: PhoneRoute,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSource {
    /// Typed into a pane, started from Hide's own controls, or forked.
    Operator,
    /// `hide agent spawn`, with a parent or as a handoff.
    Spawn,
    /// A mission node's worker.
    Mission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionSize {
    One,
    TwoToThree,
    FourToSeven,
    EightOrMore,
}

impl MissionSize {
    pub(crate) fn of(nodes: usize) -> Self {
        match nodes {
            0 | 1 => Self::One,
            2..=3 => Self::TwoToThree,
            4..=7 => Self::FourToSeven,
            _ => Self::EightOrMore,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionOutcome {
    Done,
    MergeWait,
    Cancelled,
}

/// The practice window's chapters, spelled here so a chapter the shell
/// adds sends nothing until it is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PracticeChapter {
    Mission,
    Overview,
    Screen,
    Switch,
    Find,
    Follow,
    Delegate,
    Plan,
    Project,
}

impl PracticeChapter {
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "mission" => Self::Mission,
            "overview" => Self::Overview,
            "screen" => Self::Screen,
            "switch" => Self::Switch,
            "find" => Self::Find,
            "follow" => Self::Follow,
            "delegate" => Self::Delegate,
            "plan" => Self::Plan,
            "project" => Self::Project,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PracticeStage {
    Started,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneRoute {
    Relay,
    Tailnet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceOutcome {
    Ready,
    Failed(DeviceFailedStep),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFailedStep {
    Install,
    Download,
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
            Self::AgentStarted { .. } => "hide_agent_started",
            Self::DeviceConnected { .. } => "hide_device_connected",
            Self::MissionStarted { .. } => "hide_mission_started",
            Self::MissionEnded { .. } => "hide_mission_ended",
            Self::Practice { .. } => "hide_practice",
            Self::PhonePaired { .. } => "hide_phone_paired",
        }
    }

    /// The event's own properties, every key and value spelled here so a
    /// rename elsewhere cannot change what the wire says.
    pub fn properties(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Self::Connected => Vec::new(),
            Self::AgentStarted { agent, source } => {
                let mut properties = vec![("agent", agent_name(agent))];
                if let Some(source) = source {
                    properties.push((
                        "source",
                        match source {
                            AgentSource::Operator => "operator",
                            AgentSource::Spawn => "spawn",
                            AgentSource::Mission => "mission",
                        },
                    ));
                }
                properties
            }
            Self::DeviceConnected { outcome, platform } => {
                let mut properties = Vec::with_capacity(3);
                match outcome {
                    DeviceOutcome::Ready => properties.push(("outcome", "ready")),
                    DeviceOutcome::Failed(step) => {
                        properties.push(("outcome", "failed"));
                        properties.push((
                            "failed_step",
                            match step {
                                DeviceFailedStep::Install => "install",
                                DeviceFailedStep::Download => "download",
                            },
                        ));
                    }
                }
                properties.push((
                    "platform",
                    match platform {
                        DevicePlatform::LinuxX86_64 => "linux-x86_64",
                        DevicePlatform::LinuxAarch64 => "linux-aarch64",
                        DevicePlatform::MacosX86_64 => "macos-x86_64",
                        DevicePlatform::MacosAarch64 => "macos-aarch64",
                        DevicePlatform::Unknown => "unknown",
                    },
                ));
                properties
            }
            Self::MissionStarted { nodes } => vec![(
                "nodes",
                match nodes {
                    MissionSize::One => "1",
                    MissionSize::TwoToThree => "2-3",
                    MissionSize::FourToSeven => "4-7",
                    MissionSize::EightOrMore => "8+",
                },
            )],
            Self::MissionEnded { outcome } => vec![(
                "outcome",
                match outcome {
                    MissionOutcome::Done => "done",
                    MissionOutcome::MergeWait => "merge_wait",
                    MissionOutcome::Cancelled => "cancelled",
                },
            )],
            Self::Practice { chapter, stage } => vec![
                (
                    "chapter",
                    match chapter {
                        PracticeChapter::Mission => "mission",
                        PracticeChapter::Overview => "overview",
                        PracticeChapter::Screen => "screen",
                        PracticeChapter::Switch => "switch",
                        PracticeChapter::Find => "find",
                        PracticeChapter::Follow => "follow",
                        PracticeChapter::Delegate => "delegate",
                        PracticeChapter::Plan => "plan",
                        PracticeChapter::Project => "project",
                    },
                ),
                (
                    "stage",
                    match stage {
                        PracticeStage::Started => "started",
                        PracticeStage::Finished => "finished",
                    },
                ),
            ],
            Self::PhonePaired { route } => vec![(
                "route",
                match route {
                    PhoneRoute::Relay => "relay",
                    PhoneRoute::Tailnet => "tailnet",
                },
            )],
        }
    }
}

fn agent_name(agent: hide_session::Agent) -> &'static str {
    match agent {
        hide_session::Agent::Codex => "codex",
        hide_session::Agent::Claude => "claude",
        hide_session::Agent::Grok => "grok",
        hide_session::Agent::Pi => "pi",
        hide_session::Agent::Omp => "omp",
        hide_session::Agent::Cursor => "cursor",
        hide_session::Agent::OpenCode => "opencode",
        hide_session::Agent::Antigravity => "antigravity",
    }
}

/// A bounded, nonblocking enqueue only; the daemon owns transport and shutdown.
pub trait Sink: Send + Sync + std::fmt::Debug {
    fn set_enabled(&self, enabled: bool);
    fn capture(&self, event: Event);
}
