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
    /// The operator acted from `surface` for the first time in a UTC day
    /// ([`ActiveDays`]).
    Active {
        surface: ActiveSurface,
    },
}

/// Where the operator acted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveSurface {
    /// A window of this core: a key typed into a pane, a Desk message.
    Desktop,
    /// A paired phone's action: a reply, a key, an answer, a merge, a start.
    Phone,
}

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// The UTC day, in days since the Unix epoch, on which each surface last
/// sent [`Event::Active`]. Kept in the core state so a restart, a reload or
/// a reconnect the same day sends nothing; never on the snapshot wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveDays {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phone: Option<u64>,
}

impl ActiveDays {
    /// Records that `surface` acted at `unix_ms`; true only for the first
    /// action of its UTC day. Any other day than the one recorded is a new
    /// one, so a clock set back does not silence the surface until it
    /// catches up.
    pub(crate) fn first_of_day(&mut self, surface: ActiveSurface, unix_ms: u64) -> bool {
        let day = unix_ms / DAY_MS;
        let recorded = match surface {
            ActiveSurface::Desktop => &mut self.desktop,
            ActiveSurface::Phone => &mut self.phone,
        };
        if *recorded == Some(day) {
            return false;
        }
        *recorded = Some(day);
        true
    }
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
    Agents,
    Find,
    Project,
}

impl PracticeChapter {
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "mission" => Self::Mission,
            "overview" => Self::Overview,
            "agents" => Self::Agents,
            "find" => Self::Find,
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
    Failed(DeviceFailure),
}

/// Why a device connection failed, as a closed class: never the error's
/// text, which carries host names, addresses and paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFailure {
    /// `ssh` did not connect, as the device row classifies it.
    Ssh(hide_node_link::device::SshProblem),
    /// The machine is off and the attempt could not start it.
    MachineStopped,
    /// The connection opened but the helper could not be reached on it.
    Connect,
    /// Another machine answers the device's address.
    IdentityChanged,
    /// This build cannot serve the device's system.
    Unsupported,
    /// The release kit for the device's system could not be downloaded.
    Download,
    /// The helper could not be installed on the device.
    Install,
    /// The installed helper did not start or answer.
    Helper,
    /// No connection could be set up from the registration (its SSH
    /// config or id).
    NoTransport,
    /// This core could not start the worker that connects.
    WorkerUnavailable,
    /// The helper's link ended before the kit pass finished.
    HelperLost,
    /// The device's kit pass refused to install.
    KitRefused,
    /// The device's kit pass failed.
    KitFailed,
    /// The device's machine id does not name its registration.
    MachineName,
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
            Self::Active { .. } => "hide_active",
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
                let mut properties = Vec::with_capacity(4);
                match outcome {
                    DeviceOutcome::Ready => properties.push(("outcome", "ready")),
                    DeviceOutcome::Failed(failure) => {
                        properties.push(("outcome", "failed"));
                        properties.push((
                            "failed_step",
                            match failure {
                                DeviceFailure::Download => "download",
                                _ => "install",
                            },
                        ));
                        properties.push(("failure", device_failure_name(failure)));
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
                        PracticeChapter::Agents => "agents",
                        PracticeChapter::Find => "find",
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
            Self::Active { surface } => vec![(
                "surface",
                match surface {
                    ActiveSurface::Desktop => "desktop",
                    ActiveSurface::Phone => "phone",
                },
            )],
        }
    }
}

fn device_failure_name(failure: DeviceFailure) -> &'static str {
    use hide_node_link::device::SshProblem;
    match failure {
        DeviceFailure::Ssh(problem) => match problem {
            SshProblem::SignInNeeded => "ssh_sign_in_needed",
            SshProblem::BrowserTimeout => "ssh_browser_timeout",
            SshProblem::SignInRefused => "ssh_sign_in_refused",
            SshProblem::PolicyRefused => "ssh_policy_refused",
            SshProblem::HostKeyChanged => "ssh_host_key_changed",
            SshProblem::NameNotFound => "ssh_name_not_found",
            SshProblem::Unreachable => "ssh_unreachable",
            SshProblem::ProxyFailed => "ssh_proxy_failed",
            SshProblem::Unsupported => "ssh_unsupported",
            SshProblem::SshMissing => "ssh_missing",
            SshProblem::OpenSshOld => "ssh_openssh_old",
            SshProblem::HideUpdated => "ssh_hide_updated",
            SshProblem::Unclassified => "ssh_unclassified",
        },
        DeviceFailure::MachineStopped => "machine_stopped",
        DeviceFailure::Connect => "connect",
        DeviceFailure::IdentityChanged => "identity_changed",
        DeviceFailure::Unsupported => "unsupported",
        DeviceFailure::Download => "download",
        DeviceFailure::Install => "install",
        DeviceFailure::Helper => "helper",
        DeviceFailure::NoTransport => "no_transport",
        DeviceFailure::WorkerUnavailable => "worker_unavailable",
        DeviceFailure::HelperLost => "helper_lost",
        DeviceFailure::KitRefused => "kit_refused",
        DeviceFailure::KitFailed => "kit_failed",
        DeviceFailure::MachineName => "machine_name",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Each surface counts once per UTC day: the day turns at midnight UTC,
    /// not after 24 hours, and the other surface keeps its own day.
    #[test]
    fn each_surface_counts_its_first_action_of_a_utc_day() {
        let midnight = 20_402 * DAY_MS;
        let mut days = ActiveDays::default();
        assert!(days.first_of_day(ActiveSurface::Desktop, midnight - 1));
        assert!(!days.first_of_day(ActiveSurface::Desktop, midnight - DAY_MS + 5));
        assert!(days.first_of_day(ActiveSurface::Desktop, midnight));
        assert!(!days.first_of_day(ActiveSurface::Desktop, midnight + DAY_MS - 1));
        assert!(days.first_of_day(ActiveSurface::Phone, midnight + 1));
        assert!(!days.first_of_day(ActiveSurface::Phone, midnight + 2));
        // A clock set back to an earlier day counts that day again.
        assert!(days.first_of_day(ActiveSurface::Desktop, midnight - 1));
    }

    #[test]
    fn a_device_failure_names_its_class_and_never_its_text() {
        let event = Event::DeviceConnected {
            outcome: DeviceOutcome::Failed(DeviceFailure::Ssh(
                hide_node_link::device::SshProblem::Unreachable,
            )),
            platform: DevicePlatform::Unknown,
        };
        assert_eq!(
            event.properties(),
            [
                ("outcome", "failed"),
                ("failed_step", "install"),
                ("failure", "ssh_unreachable"),
                ("platform", "unknown"),
            ]
        );
    }
}
