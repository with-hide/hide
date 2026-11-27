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
        /// The path a paired machine's connection came up on, sent with a
        /// ready outcome of that machine's device only;
        /// none for an SSH or WSL helper device, whose link is its own, and
        /// for every failure.
        path: Option<DevicePath>,
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
    /// An automation was made, with the example its form started from and
    /// whether it runs once or repeats (mission-104).
    AutomationCreated {
        start: AutomationStart,
        schedule: AutomationSchedule,
    },
    /// A run of an automation ended, once per run.
    AutomationRunEnded {
        outcome: AutomationOutcome,
        trigger: AutomationTrigger,
    },
    /// A main feature was used for the first time in a UTC day
    /// ([`ActiveDays`]).
    FeatureUsed {
        feature: Feature,
    },
    /// A bot was connected: the operator made it and its AI joined with the
    /// code, or it asked and the operator approved it; with the mode it
    /// works in. A join run again after a lost answer adds none.
    BotConnected {
        way: BotWay,
        mode: crate::access::Mode,
    },
}

/// How a bot came in: made in Settings › Bots and joined with its code, or
/// asked with `hide access request` and approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotWay {
    Made,
    Asked,
}

/// The example an automation's form started from; `Own` for one written
/// from an empty form or made with `hide automation add`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationStart {
    Report,
    Briefing,
    Ci,
    Own,
}

/// Whether an automation runs at one time or repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedule {
    Once,
    Repeat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationOutcome {
    Succeeded,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationTrigger {
    Scheduled,
    Missed,
    Manual,
}

/// A main feature whose daily use is counted by [`Event::FeatureUsed`].
/// A feature the What's new window names is measured by one of these or by
/// an event of its own ([`MEASURES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// A pane conversation opened in place.
    PaneConversation,
    /// Focus mode turned on.
    Focus,
    /// A skill or MCP server added, changed, switched or removed, from
    /// Settings or `hide skill` / `hide mcp`.
    SkillsMcp,
    /// A pairing with another computer that runs Hide finished, in either
    /// direction (PRD device-connect-iroh): the daemon tells the core.
    HidesPaired,
    /// A gitlab.com project's merge requests read on this machine.
    Gitlab,
    /// An agent's own question, plan or tool permission answered with one
    /// of its choices from a phone.
    PhoneAnswer,
    /// A reply sent from a phone with files attached to it.
    PhoneAttachments,
}

impl Feature {
    const ALL: [Self; 7] = [
        Self::PaneConversation,
        Self::Focus,
        Self::SkillsMcp,
        Self::HidesPaired,
        Self::Gitlab,
        Self::PhoneAnswer,
        Self::PhoneAttachments,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::PaneConversation => "pane_conversation",
            Self::Focus => "focus",
            Self::SkillsMcp => "skills_mcp",
            Self::HidesPaired => "hides_paired",
            Self::Gitlab => "gitlab",
            Self::PhoneAnswer => "phone_answer",
            Self::PhoneAttachments => "phone_attachments",
        }
    }
}

/// What a What's new highlight may name as its `measure` in
/// `web/src/releases.json`: an event's name, or `hide_feature_used:` and a
/// feature's name. A test holds every highlight from 1.2.11 on to this list.
pub fn measures() -> Vec<String> {
    let mut measures: Vec<String> = [
        "hide_agent_started",
        "hide_device_connected",
        "hide_mission_started",
        "hide_mission_ended",
        "hide_practice",
        "hide_phone_paired",
        "hide_active",
        "hide_automation_created",
        "hide_automation_run_ended",
        "hide_bot_connected",
    ]
    .map(str::to_owned)
    .to_vec();
    measures.extend(
        Feature::ALL
            .iter()
            .map(|feature| format!("hide_feature_used:{}", feature.name())),
    );
    measures
}

/// What counts once per UTC day: a surface acted from, or a feature used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Daily {
    Surface(ActiveSurface),
    Feature(Feature),
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
/// sent [`Event::Active`] and each feature [`Event::FeatureUsed`]. Kept in the core state so a restart, a reload or
/// a reconnect the same day sends nothing; never on the snapshot wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveDays {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desktop: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phone: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    focus: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skills_mcp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hides_paired: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gitlab: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phone_answer: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phone_attachments: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pane_conversation: Option<u64>,
}

impl ActiveDays {
    /// Records that `what` happened at `unix_ms`; true only for the first
    /// time in its UTC day. Any other day than the one recorded is a new
    /// one, so a clock set back does not silence it until it catches up.
    pub(crate) fn first_of_day(&mut self, what: Daily, unix_ms: u64) -> bool {
        let day = unix_ms / DAY_MS;
        let recorded = match what {
            Daily::Surface(ActiveSurface::Desktop) => &mut self.desktop,
            Daily::Surface(ActiveSurface::Phone) => &mut self.phone,
            Daily::Feature(Feature::Focus) => &mut self.focus,
            Daily::Feature(Feature::SkillsMcp) => &mut self.skills_mcp,
            Daily::Feature(Feature::HidesPaired) => &mut self.hides_paired,
            Daily::Feature(Feature::Gitlab) => &mut self.gitlab,
            Daily::Feature(Feature::PhoneAnswer) => &mut self.phone_answer,
            Daily::Feature(Feature::PhoneAttachments) => &mut self.phone_attachments,
            Daily::Feature(Feature::PaneConversation) => &mut self.pane_conversation,
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
    /// An automation's run.
    Automation,
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
    Automation,
    Focus,
    Bots,
    Connect,
    Gitlab,
    Answer,
    Attachments,
    Conversation,
}

impl PracticeChapter {
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "mission" => Self::Mission,
            "overview" => Self::Overview,
            "agents" => Self::Agents,
            "find" => Self::Find,
            "project" => Self::Project,
            "automation" => Self::Automation,
            "focus" => Self::Focus,
            "bots" => Self::Bots,
            "connect" => Self::Connect,
            "gitlab" => Self::Gitlab,
            "answer" => Self::Answer,
            "attachments" => Self::Attachments,
            "conversation" => Self::Conversation,
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
    /// This build cannot serve the device, by why.
    Unsupported(hide_node_link::device::UnsupportedCause),
    /// The release kit for the device's system could not be downloaded.
    Download,
    /// The helper could not be installed on the device, by where the
    /// install stopped.
    Install(hide_node_link::device::InstallStop),
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

/// The path a paired machine's connection takes, as the `path` property
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevicePath {
    Direct,
    Tailscale,
    Ssh,
    Relay,
    Local,
}

impl DevicePath {
    pub(crate) fn of(path: crate::peer_machines::PeerPath) -> Self {
        use crate::peer_machines::PeerPath;
        match path {
            PeerPath::Direct => Self::Direct,
            PeerPath::Tailscale => Self::Tailscale,
            PeerPath::Ssh => Self::Ssh,
            PeerPath::Relay => Self::Relay,
            PeerPath::Local => Self::Local,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Tailscale => "tailscale",
            Self::Ssh => "ssh",
            Self::Relay => "relay",
            Self::Local => "local",
        }
    }
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
            Self::AutomationCreated { .. } => "hide_automation_created",
            Self::AutomationRunEnded { .. } => "hide_automation_run_ended",
            Self::FeatureUsed { .. } => "hide_feature_used",
            Self::BotConnected { .. } => "hide_bot_connected",
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
                            AgentSource::Automation => "automation",
                        },
                    ));
                }
                properties
            }
            Self::DeviceConnected {
                outcome,
                platform,
                path,
            } => {
                let mut properties = Vec::with_capacity(5);
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
                // A paired machine's ready device says the path it came up
                // on; a helper device and a failure say none.
                if let (DeviceOutcome::Ready, Some(path)) = (outcome, path) {
                    properties.push(("path", path.name()));
                }
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
                        PracticeChapter::Automation => "automation",
                        PracticeChapter::Focus => "focus",
                        PracticeChapter::Bots => "bots",
                        PracticeChapter::Connect => "connect",
                        PracticeChapter::Gitlab => "gitlab",
                        PracticeChapter::Answer => "answer",
                        PracticeChapter::Attachments => "attachments",
                        PracticeChapter::Conversation => "conversation",
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
            Self::AutomationCreated { start, schedule } => vec![
                (
                    "start",
                    match start {
                        AutomationStart::Report => "report",
                        AutomationStart::Briefing => "briefing",
                        AutomationStart::Ci => "ci",
                        AutomationStart::Own => "own",
                    },
                ),
                (
                    "schedule",
                    match schedule {
                        AutomationSchedule::Once => "once",
                        AutomationSchedule::Repeat => "repeat",
                    },
                ),
            ],
            Self::AutomationRunEnded { outcome, trigger } => vec![
                (
                    "outcome",
                    match outcome {
                        AutomationOutcome::Succeeded => "succeeded",
                        AutomationOutcome::Failed => "failed",
                        AutomationOutcome::Stopped => "stopped",
                    },
                ),
                (
                    "trigger",
                    match trigger {
                        AutomationTrigger::Scheduled => "scheduled",
                        AutomationTrigger::Missed => "missed",
                        AutomationTrigger::Manual => "manual",
                    },
                ),
            ],
            Self::FeatureUsed { feature } => vec![("feature", feature.name())],
            Self::BotConnected { way, mode } => vec![
                (
                    "way",
                    match way {
                        BotWay::Made => "made",
                        BotWay::Asked => "asked",
                    },
                ),
                (
                    "mode",
                    match mode {
                        crate::access::Mode::Own => "own",
                        crate::access::Mode::All => "all",
                    },
                ),
            ],
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
            SshProblem::SilentEnd => "ssh_silent_end",
            SshProblem::SilentTimeout => "ssh_silent_timeout",
            SshProblem::ConfigUnreadable => "ssh_config_unreadable",
            SshProblem::CheckFailed => "ssh_check_failed",
            SshProblem::Busy => "ssh_busy",
            SshProblem::Unclassified => "ssh_unclassified",
        },
        DeviceFailure::MachineStopped => "machine_stopped",
        DeviceFailure::Connect => "connect",
        DeviceFailure::IdentityChanged => "identity_changed",
        DeviceFailure::Unsupported(cause) => {
            use hide_node_link::device::UnsupportedCause;
            match cause {
                UnsupportedCause::Platform => "unsupported_platform",
                UnsupportedCause::Build => "unsupported_build",
                UnsupportedCause::Probe => "unsupported_probe",
                UnsupportedCause::UserName => "unsupported_user_name",
                UnsupportedCause::Home => "unsupported_home",
                UnsupportedCause::Checksum => "unsupported_checksum",
                UnsupportedCause::MachineId => "unsupported_machine_id",
                UnsupportedCause::Wsl1 => "unsupported_wsl1",
                UnsupportedCause::WslInstalling => "unsupported_wsl_installing",
                UnsupportedCause::WslSetup => "unsupported_wsl_setup",
                UnsupportedCause::WslMissing => "unsupported_wsl_missing",
                UnsupportedCause::PeerUpdate => "unsupported_peer_update",
                UnsupportedCause::PeerFolder => "unsupported_peer_folder",
                UnsupportedCause::PeerFull => "unsupported_peer_full",
            }
        }
        DeviceFailure::Download => "download",
        DeviceFailure::Install(stop) => {
            use hide_node_link::device::InstallStop;
            match stop {
                InstallStop::Refused => "install_refused",
                InstallStop::Ended => "install_ended",
                InstallStop::Quiet => "install_quiet",
                InstallStop::Local => "install_local",
            }
        }
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
        let desktop = Daily::Surface(ActiveSurface::Desktop);
        let phone = Daily::Surface(ActiveSurface::Phone);
        let focus = Daily::Feature(Feature::Focus);
        let mut days = ActiveDays::default();
        assert!(days.first_of_day(desktop, midnight - 1));
        assert!(!days.first_of_day(desktop, midnight - DAY_MS + 5));
        assert!(days.first_of_day(desktop, midnight));
        assert!(!days.first_of_day(desktop, midnight + DAY_MS - 1));
        assert!(days.first_of_day(phone, midnight + 1));
        assert!(!days.first_of_day(phone, midnight + 2));
        // A feature keeps its own day beside the surfaces.
        assert!(days.first_of_day(focus, midnight + 3));
        assert!(!days.first_of_day(focus, midnight + 4));
        assert!(days.first_of_day(Daily::Feature(Feature::SkillsMcp), midnight + 4));
        // A clock set back to an earlier day counts that day again.
        assert!(days.first_of_day(desktop, midnight - 1));
    }

    fn web_file(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../web/src")
            .join(path)
    }

    /// Every practice chapter the window offers sends `hide_practice`: a
    /// chapter added under `web/src/practice/chapters/` without its name
    /// here would count nothing (CONTRIBUTING.md, A main feature brings its
    /// practice, its scene and its measure).
    #[test]
    fn every_practice_chapter_is_named() {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(web_file("practice/chapters")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".ts") || name.ends_with(".test.ts") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let id = text
                .lines()
                .find_map(|line| line.strip_prefix("  id: \""))
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("{name} names no chapter id"));
            ids.push(id.to_owned());
        }
        assert!(ids.len() >= 8, "{ids:?}");
        for id in ids {
            assert!(
                PracticeChapter::of(&id).is_some(),
                "practice chapter `{id}` has no name in PracticeChapter::of"
            );
        }
    }

    /// Every main feature the What's new window shows from 1.2.11 on names
    /// how its use is counted (`measure`), and the measure is one this build
    /// sends, so a main feature cannot ship without one.
    #[test]
    fn every_highlight_names_a_measure_this_build_sends() {
        let text = std::fs::read_to_string(web_file("releases.json")).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let measures = measures();
        let version = |text: &str| -> Vec<u32> {
            text.split('.').map(|part| part.parse().unwrap()).collect()
        };
        let mut checked = 0;
        for entry in file["versions"].as_array().unwrap() {
            let name = entry["version"].as_str().unwrap();
            if version(name) < vec![1, 2, 11] {
                continue;
            }
            for highlight in entry["highlights"].as_array().unwrap() {
                let measure = highlight["measure"].as_str().unwrap_or_else(|| {
                    panic!(
                        "a {name} highlight ({}) names no `measure`",
                        highlight["drawing"]
                    )
                });
                assert!(
                    measures.iter().any(|known| known == measure),
                    "{name}: `{measure}` is not a measure this build sends: {measures:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    /// The wire spells how a bot came in and its mode, and nothing else of
    /// the bot.
    #[test]
    fn a_connected_bot_names_how_it_came_in_and_its_mode_only() {
        use crate::access::Mode;
        let event = |way, mode| Event::BotConnected { way, mode }.properties();
        assert_eq!(
            event(BotWay::Made, Mode::Own),
            [("way", "made"), ("mode", "own")]
        );
        assert_eq!(
            event(BotWay::Asked, Mode::All),
            [("way", "asked"), ("mode", "all")]
        );
    }

    /// A paired machine's ready device names the path its connection came
    /// up on; a helper device names none, and no failure names a path.
    #[test]
    fn a_ready_paired_machine_says_its_path_and_a_helper_device_says_none() {
        let ready = |path| {
            Event::DeviceConnected {
                outcome: DeviceOutcome::Ready,
                platform: DevicePlatform::MacosAarch64,
                path,
            }
            .properties()
        };
        assert_eq!(
            ready(Some(DevicePath::Ssh)),
            [
                ("outcome", "ready"),
                ("platform", "macos-aarch64"),
                ("path", "ssh")
            ]
        );
        assert_eq!(
            ready(None),
            [("outcome", "ready"), ("platform", "macos-aarch64")]
        );
        for path in [
            DevicePath::Direct,
            DevicePath::Tailscale,
            DevicePath::Relay,
            DevicePath::Local,
        ] {
            assert!(ready(Some(path)).iter().any(|(name, _)| *name == "path"));
        }
        let failed = Event::DeviceConnected {
            outcome: DeviceOutcome::Failed(DeviceFailure::KitFailed),
            platform: DevicePlatform::Unknown,
            path: Some(DevicePath::Ssh),
        };
        assert!(!failed.properties().iter().any(|(name, _)| *name == "path"));
    }

    #[test]
    fn a_device_failure_names_its_class_and_never_its_text() {
        let event = Event::DeviceConnected {
            outcome: DeviceOutcome::Failed(DeviceFailure::Ssh(
                hide_node_link::device::SshProblem::Unreachable,
            )),
            platform: DevicePlatform::Unknown,
            path: None,
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

    /// An install failure names where it stopped, so a device that refused
    /// a step is told from a link that ended or went quiet part way.
    #[test]
    fn an_install_failure_names_where_it_stopped() {
        use hide_node_link::device::InstallStop;
        for (stop, name) in [
            (InstallStop::Refused, "install_refused"),
            (InstallStop::Ended, "install_ended"),
            (InstallStop::Quiet, "install_quiet"),
            (InstallStop::Local, "install_local"),
        ] {
            let event = Event::DeviceConnected {
                outcome: DeviceOutcome::Failed(DeviceFailure::Install(stop)),
                platform: DevicePlatform::Unknown,
                path: None,
            };
            assert_eq!(
                event.properties(),
                [
                    ("outcome", "failed"),
                    ("failed_step", "install"),
                    ("failure", name),
                    ("platform", "unknown"),
                ]
            );
        }
    }

    /// A device this build cannot serve names why, so a WSL distribution
    /// left on WSL 1 is told from one nobody finished setting up, and from
    /// another machine's Hide that needs an update.
    #[test]
    fn an_unsupported_device_names_why() {
        use hide_node_link::device::UnsupportedCause;
        for (cause, name) in [
            (UnsupportedCause::Platform, "unsupported_platform"),
            (UnsupportedCause::Build, "unsupported_build"),
            (UnsupportedCause::Probe, "unsupported_probe"),
            (UnsupportedCause::UserName, "unsupported_user_name"),
            (UnsupportedCause::Home, "unsupported_home"),
            (UnsupportedCause::Checksum, "unsupported_checksum"),
            (UnsupportedCause::MachineId, "unsupported_machine_id"),
            (UnsupportedCause::Wsl1, "unsupported_wsl1"),
            (
                UnsupportedCause::WslInstalling,
                "unsupported_wsl_installing",
            ),
            (UnsupportedCause::WslSetup, "unsupported_wsl_setup"),
            (UnsupportedCause::WslMissing, "unsupported_wsl_missing"),
            (UnsupportedCause::PeerUpdate, "unsupported_peer_update"),
            (UnsupportedCause::PeerFolder, "unsupported_peer_folder"),
            (UnsupportedCause::PeerFull, "unsupported_peer_full"),
        ] {
            let event = Event::DeviceConnected {
                outcome: DeviceOutcome::Failed(DeviceFailure::Unsupported(cause)),
                platform: DevicePlatform::Unknown,
                path: None,
            };
            assert_eq!(
                event.properties(),
                [
                    ("outcome", "failed"),
                    ("failed_step", "install"),
                    ("failure", name),
                    ("platform", "unknown"),
                ]
            );
        }
    }

    /// The endings ssh explained with no line Hide knows are counted apart,
    /// so `ssh_unclassified` holds only a line Hide does not know.
    #[test]
    fn an_unexplained_ssh_ending_names_how_it_ended() {
        use hide_node_link::device::SshProblem;
        for (problem, name) in [
            (SshProblem::SilentEnd, "ssh_silent_end"),
            (SshProblem::SilentTimeout, "ssh_silent_timeout"),
            (SshProblem::ConfigUnreadable, "ssh_config_unreadable"),
            (SshProblem::CheckFailed, "ssh_check_failed"),
            (SshProblem::Busy, "ssh_busy"),
            (SshProblem::Unclassified, "ssh_unclassified"),
        ] {
            let event = Event::DeviceConnected {
                outcome: DeviceOutcome::Failed(DeviceFailure::Ssh(problem)),
                platform: DevicePlatform::Unknown,
                path: None,
            };
            assert!(
                event.properties().contains(&("failure", name)),
                "{problem:?}"
            );
        }
    }
}
