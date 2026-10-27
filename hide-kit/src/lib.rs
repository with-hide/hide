//! Hide's install kit: the one list of what Hide puts on a machine, and how
//! each part is installed, judged and removed.
//!
//! The same code runs for this machine, inside `hided`, and for a device, inside
//! `hided node serve`; only the [`KitTarget`] differs, so a device gets
//! exactly what this Mac gets (PRD device-parity D-09, D-10). Adding a part is
//! adding a [`ComponentId`] and its three answers here; nothing else in the
//! product learns how to install it.
//!
//! Three rules hold for every part:
//!
//! - what another tool wrote is never touched: a hook entry without Hide's
//!   marker, a `hide` that is not Hide's link, a plugin config folder (D-15);
//! - a part that failed does not stop the others, and says why (D-18);
//! - a part the kit installed and the operator then took away stays away
//!   until the operator asks for it again with Reinstall (D-20, D-26). The
//!   kit's own record of what it installed is what tells "taken away" from
//!   "never installed"; `~/.hide/agent-hooks/installed-once` from before the
//!   kit is not such a record (B20).

mod agent_kit;
mod agent_update;
pub mod agents;
mod cli;
mod codex_trust;
mod coordination_retirement;
mod device;
pub mod device_herdr;
mod herdr_integration;
mod hooks;
mod labels;
pub mod layout;
pub mod legacy;
mod local;
pub mod process;
mod record;
mod refusal;
mod retired_agents;
pub mod retirement_inspection;
pub mod runtime;
mod state_route;
mod why;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hide_platform::programs::Program;
use serde::{Deserialize, Serialize};

pub use agent_update::AgentUpdate;
pub use agents::{
    AgentAdapter, AgentReport, Availability, Feature, HookSupport, PieceReport, SkillDir,
};
pub use coordination_retirement::preflight as retirement_preflight;
pub use device::{CURRENT, device_target};
pub use labels::{HCOORD_PLUGIN_ID, LABELS_PLUGIN_ID, Retirement, labels_home, plugin_state_dir};
pub use legacy::{BuildPlace, build_of, is_build_name};
pub use local::{STANDALONE_REASON, bundled_kit_dir, holds_package_runtime, local_target};
pub use record::kit_state_dir;
pub use refusal::{KitRefusal, RefusalCause};
pub(crate) use why::Reason;
pub use why::{PartCause, PartWhy};

/// One part of the kit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentId {
    /// The `hide` command, installed in the account's command folder.
    Cli,
    /// Hide's entries in `~/.claude/settings.json`.
    ClaudeCodeHook,
    /// Hide's entries in `~/.codex/hooks.json`.
    CodexHook,
    /// One-release retirement of the old coordination installation.
    CoordinationRetirement,
}

impl ComponentId {
    pub const ALL: [ComponentId; 4] = [
        Self::Cli,
        Self::ClaudeCodeHook,
        Self::CodexHook,
        Self::CoordinationRetirement,
    ];

    /// The stable name the wire and the record carry.
    pub fn code(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::ClaudeCodeHook => "claude_code_hook",
            Self::CodexHook => "codex_hook",
            Self::CoordinationRetirement => "coordination_retirement",
        }
    }

    /// The name the operator reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cli => "hide command",
            Self::ClaudeCodeHook => "Claude Code hook",
            Self::CodexHook => "Codex hook",
            Self::CoordinationRetirement => "Coordination retirement",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.code() == code)
    }
}

/// What one part is on the machine now.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentState {
    /// This build's part is in place.
    Installed,
    /// A part is there, but not this build's: an older version, another
    /// path, or a copy installed by hand. The next launch or connection
    /// replaces it.
    Outdated,
    /// Never installed here.
    NotInstalled,
    /// The kit installed it and it has since gone; only Reinstall brings it
    /// back.
    Removed,
    /// It could not be installed or judged; `reason` says why.
    Failed,
    /// The machine has nothing for this part to attach to, such as an agent
    /// runtime that is not set up there.
    Absent,
    /// The agent whose switch governs the part is off, whether or not Hide
    /// ever installed it; no pass puts it back and nothing asks them to (D-24,
    /// B36). Every reader treats it as a choice, never as damage: the machine
    /// row offers no Reinstall and a device's hook reads "switched off".
    Off,
}

impl ComponentState {
    /// Whether the machine row offers Reinstall for this part.
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::Outdated | Self::NotInstalled | Self::Removed | Self::Failed
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ComponentReport {
    pub id: ComponentId,
    pub state: ComponentState,
    /// Why the part is not installed, in one sentence for the log and the
    /// `hide` CLI; absent when it is. No screen draws it: `why` is what the
    /// app words.
    #[serde(default)]
    pub reason: Option<String>,
    /// The same why as a cause the app words in the operator's language;
    /// absent from a helper whose build predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<PartWhy>,
    /// The file, link or folder the part lives at on that machine.
    #[serde(default)]
    pub location: Option<String>,
}

/// Every part's state on one machine, in [`ComponentId::ALL`] order.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct KitReport {
    pub components: Vec<ComponentReport>,
    /// Every agent Hide has an adapter for, in the adapters' order (issue
    /// #517); empty in a report from a build that predates them.
    #[serde(default)]
    pub agents: Vec<AgentReport>,
    /// The machine's Codex has the shared daemon setting, so Hide starts it
    /// with `--no-daemon` (`herdr-core`'s `codex_launch`); `Some(false)` is a
    /// Codex read to be older than the setting, `None` no Codex or a failed
    /// read. Hide reads it and never changes it (PRD settings-cleanup D-12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_daemon: Option<bool>,
    /// Whether that Codex starts the shared daemon on its own now, read with
    /// the capability and never changed by a pass: `Some(true)` is the shared
    /// server a pane reads as the reason its session is not connected (PRD
    /// settings-cleanup B27). `None` when there is no answer or the Codex has
    /// no such setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_daemon_on: Option<bool>,
    /// Whether a shared daemon answers now, read with `codex app-server daemon
    /// version` for a Codex that has the setting: with the setting already
    /// off, `Some(true)` is a daemon still serving each Codex started by hand
    /// (PRD codex-daemon-apply D-07, B9). `None` when there is no Codex, no
    /// setting, or no answer Hide can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_daemon_running: Option<bool>,
    /// An answer about the daemon Hide could not read, for the core's log
    /// only; the screen reads it as no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_daemon_unreadable: Option<String>,
    /// Why the machine's Codex gave no answer for [`Self::codex_daemon`], for
    /// the core's log only: every Codex start the core makes is refused until
    /// it reads. `None` when it answered, there is no Codex, or Hide was
    /// quitting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_capability_unread: Option<CodexReadFailure>,
    /// How the operator's request to turn the shared server off ended; present
    /// only in the report of the pass that carried it ([`Scope::codex_daemon_off`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_daemon_off: Option<CodexDaemonOff>,
    /// The machine's record says the operator has not answered the first-run
    /// agent choice: its first pass left the agents that are on by default off
    /// until they do, and an explicit agent choice clears it. It reads the
    /// record, so it stays true across launches until answered.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held_for_onboarding: bool,
    /// What an apply took out of the retired labels plugin; empty when there
    /// was nothing of it (PRD labels-in-hided D-12).
    #[serde(default, skip_serializing_if = "Retirement::is_empty")]
    pub labels_retirement: Retirement,
    /// What an apply took off the machine from older layouts: the
    /// standalone hcoord Herdr plugin and the folders `~/.hide` replaced;
    /// empty when there was nothing (PRD hide-home-layout D-13, D-14).
    #[serde(default, skip_serializing_if = "Retirement::is_empty")]
    pub legacy_retirement: Retirement,
    /// What a device's node did about the Herdr server on its registered
    /// socket after this pass (PRD device-herdr-update); only a device's
    /// apply carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr: Option<Box<device_herdr::HerdrUpdate>>,
    /// The pass stopped before changing anything, and why; the report then
    /// has no part and no agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<Box<KitRefusal>>,
}

impl KitReport {
    pub fn component(&self, id: ComponentId) -> Option<&ComponentReport> {
        self.components.iter().find(|report| report.id == id)
    }

    /// The report of a pass that stopped before changing anything: nothing
    /// was judged, so it lists no part and no agent.
    pub fn refused(refusal: KitRefusal) -> Self {
        Self {
            refused: Some(Box::new(refusal)),
            ..Self::default()
        }
    }
}

/// The login agents the kit uses for `home`: the account's own, and none
/// for a fixture HOME (`layout::fixture_home`), so a test's kit never
/// reaches the account's real launchd domain.
pub fn login_agents(home: &Path) -> hide_platform::user_agents::UserAgents {
    if layout::fixture_home(home) {
        hide_platform::user_agents::UserAgents::none()
    } else {
        hide_platform::user_agents::UserAgents::current()
    }
}

/// Where the kit acts on one machine.
#[derive(Clone, Debug)]
pub struct KitTarget {
    /// The account's home, whose configuration files the hooks go into.
    pub home: PathBuf,
    /// The folder holding `hide` and `hide-agent-hooks`. The hooks and
    /// command link name this folder, so it must outlive the
    /// process: the desktop package's resources on this machine, the helper
    /// root's `current` link on a device (D-11).
    pub kit_dir: PathBuf,
    /// Where the `hide` command is installed (`~/.local/bin` unless the daemon was told
    /// otherwise).
    pub cli_dir: PathBuf,
    /// Folders whose links are Hide's own besides an app bundle's
    /// `Contents/Resources`, such as a device helper root, so a `hide` link
    /// into one of them is replaced rather than left as the operator's.
    pub owned_roots: Vec<PathBuf>,
    /// The machine's Herdr server, which the retired labels plugin is taken
    /// out of.
    pub herdr_socket: PathBuf,
    /// The `herdr` CLI, needed only to take out a GitHub install of the
    /// retired labels plugin; `None` when the machine has none Hide can find.
    pub herdr_bin: Option<PathBuf>,
    /// The machine's `codex`, found on the agents' search
    /// (`agents::search_path`) and run with it; `None` when Hide finds none.
    pub codex: Option<Program>,
    /// The account's login shell, asked for the `PATH` its startup files set
    /// up, so an agent's CLI is found where the operator's terminal finds it
    /// (`agents::Detection`). `None` asks nothing: Windows has no login
    /// shell, and a test searches only its fixture.
    pub login_shell: Option<PathBuf>,
    /// Legacy relocation, inspected only by the one-release retirement.
    pub legacy_coordination_home: Option<PathBuf>,
    /// Account login-agent boundary. Fixtures inject a stand-in command.
    pub user_agents: hide_platform::user_agents::UserAgents,
    /// Registered checkout roots included in retirement preflight.
    pub retirement_projects: Vec<PathBuf>,
    /// Folders of older layouts the pass takes off this machine (D-13).
    pub legacy: Vec<legacy::Legacy>,
    /// Raised by the kit's owner when it is going away: a child the kit is
    /// waiting on is ended and no further part is started.
    pub stop: Arc<AtomicBool>,
}

/// What an apply is allowed to do. The launch or connection pass is the
/// empty scope: it installs what was never installed and replaces what is
/// outdated, and leaves alone whatever the operator took away or turned off.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scope {
    /// Parts the operator asked back, with Reinstall or by turning one on:
    /// they are installed again even when they were taken away. Parts that
    /// are in place are not touched (B8).
    pub restore: BTreeSet<ComponentId>,
    /// Agents the operator switched on, by adapter id: the choice is
    /// recorded and each of the agent's pieces is installed again, even one
    /// that was taken away (issue #517).
    pub agent_on: BTreeSet<String>,
    /// Agents the operator switched off: their marked entries and stubs come
    /// out and no pass puts them back.
    pub agent_off: BTreeSet<String>,
    /// The operator asked for Codex's shared server to be turned off on this
    /// machine (PRD settings-cleanup D-12, B27). Only this request ever does
    /// it, and it answers in `KitReport::codex_daemon_off`.
    pub codex_daemon_off: bool,
}

impl Scope {
    /// The launch or connection pass.
    pub fn automatic() -> Self {
        Self::default()
    }

    pub fn reinstall(parts: impl IntoIterator<Item = ComponentId>) -> Self {
        Self {
            restore: parts.into_iter().collect(),
            ..Self::default()
        }
    }

    /// The operator's per-agent choices: `on` switched on (or Reinstall of an
    /// agent that is on), `off` switched off.
    pub fn agents<'a>(
        on: impl IntoIterator<Item = &'a str>,
        off: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            agent_on: on.into_iter().map(str::to_owned).collect(),
            agent_off: off.into_iter().map(str::to_owned).collect(),
            ..Self::default()
        }
        .settled()
    }

    /// The answer to the first-run agent choice with `on` switched on: every
    /// agent that is on by default and not chosen is named off, so an answer
    /// with nothing chosen is still an explicit one, which is what clears the
    /// machine's wait for it.
    pub fn first_run<'a>(on: impl IntoIterator<Item = &'a str>) -> Self {
        let on: Vec<&str> = on.into_iter().collect();
        let off = agents::ADAPTERS
            .iter()
            .filter(|adapter| adapter.default_on && !on.contains(&adapter.id))
            .map(|adapter| adapter.id);
        Self::agents(on.iter().copied(), off)
    }

    /// An agent named both ways in one request is switched off, the safer
    /// reading; a later request overrides an earlier one in [`Self::merge`].
    fn settled(mut self) -> Self {
        let both: Vec<String> = self
            .agent_on
            .intersection(&self.agent_off)
            .cloned()
            .collect();
        for id in both {
            self.agent_on.remove(&id);
        }
        self
    }

    /// The operator's request to turn Codex's shared server off, and nothing
    /// else.
    pub fn codex_daemon_off() -> Self {
        Self {
            codex_daemon_off: true,
            ..Self::default()
        }
    }

    pub fn is_automatic(&self) -> bool {
        self.restore.is_empty()
            && self.agent_on.is_empty()
            && self.agent_off.is_empty()
            && !self.codex_daemon_off
    }

    /// Two requests for one machine as one; for an agent named by both, the
    /// later choice wins.
    pub fn merge(mut self, later: Scope) -> Scope {
        self.restore.extend(later.restore);
        self.codex_daemon_off |= later.codex_daemon_off;
        for id in later.agent_on {
            self.agent_off.remove(&id);
            self.agent_on.insert(id);
        }
        for id in later.agent_off {
            self.agent_on.remove(&id);
            self.agent_off.insert(id);
        }
        self
    }

    fn restores(&self, id: ComponentId) -> bool {
        self.restore.contains(&id)
    }
}

/// What one part looks like before the kit decides anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Observed {
    Current,
    /// There, but not this build's; the reason says what is there.
    Stale(Reason),
    /// Hide's, edited by the operator after Hide wrote it: theirs until
    /// Reinstall, read as out of date with the reason saying so.
    Edited(Reason),
    Missing,
    /// Neither installed nor installable as things stand.
    Blocked(Reason),
    /// Nothing on the machine for the part to attach to.
    Absent(Reason),
    /// A hook that cannot be written on this system or for this version.
    Unsupported(Reason),
}

fn observe(id: ComponentId, target: &KitTarget) -> Observed {
    match id {
        ComponentId::Cli => cli::observe(target),
        ComponentId::ClaudeCodeHook => {
            hooks::observe(target, hide_agent_hooks::AgentRuntime::ClaudeCode)
        }
        ComponentId::CodexHook => hooks::observe(target, hide_agent_hooks::AgentRuntime::Codex),
        ComponentId::CoordinationRetirement => coordination_retirement::observe(target),
    }
}

fn install(id: ComponentId, target: &KitTarget) -> Result<(), String> {
    match id {
        ComponentId::Cli => cli::install(target),
        ComponentId::ClaudeCodeHook => {
            hooks::install(target, hide_agent_hooks::AgentRuntime::ClaudeCode)
        }
        ComponentId::CodexHook => hooks::install(target, hide_agent_hooks::AgentRuntime::Codex),
        ComponentId::CoordinationRetirement => coordination_retirement::install(target).map(|_| ()),
    }
}

/// Takes a part out because the agent whose hook it is was switched off.
fn turn_off(id: ComponentId, target: &KitTarget) -> Result<(), String> {
    match id {
        // An agent's switch turns its hook part off (`agent_kit::part_gate`).
        ComponentId::ClaudeCodeHook => {
            hooks::turn_off(target, hide_agent_hooks::AgentRuntime::ClaudeCode)
        }
        ComponentId::CodexHook => hooks::turn_off(target, hide_agent_hooks::AgentRuntime::Codex),
        ComponentId::Cli | ComponentId::CoordinationRetirement => {
            Err(format!("{} cannot be turned off", id.label()))
        }
    }
}

fn location(id: ComponentId, target: &KitTarget) -> String {
    match id {
        ComponentId::Cli => cli::link_path(target).display().to_string(),
        ComponentId::ClaudeCodeHook => hide_agent_hooks::AgentRuntime::ClaudeCode
            .config_path(&target.home)
            .display()
            .to_string(),
        ComponentId::CodexHook => hide_agent_hooks::AgentRuntime::Codex
            .config_path(&target.home)
            .display()
            .to_string(),
        ComponentId::CoordinationRetirement => coordination_retirement::location(target)
            .display()
            .to_string(),
    }
}

fn report(
    id: ComponentId,
    target: &KitTarget,
    observed: Observed,
    recorded: bool,
    switched_off: bool,
    failure: Option<String>,
) -> ComponentReport {
    let (state, reason) = match (failure, observed) {
        (Some(failure), _) => (ComponentState::Failed, Some(Reason::not_changed(failure))),
        (None, Observed::Current) => (ComponentState::Installed, None),
        // An agent that is switched off gets nothing from a pass, so a hook
        // that is out of date or cannot be judged is not a repair to offer.
        (None, Observed::Stale(_) | Observed::Edited(_) | Observed::Blocked(_)) if switched_off => {
            (ComponentState::Off, None)
        }
        (None, Observed::Stale(reason) | Observed::Edited(reason)) => {
            (ComponentState::Outdated, Some(reason))
        }
        // Gone after Hide applied it: the operator turned it off or undid it
        // by hand, and either way it is theirs now (B36).
        (None, Observed::Missing) if switched_off => (ComponentState::Off, None),
        (None, Observed::Missing) if recorded => {
            (ComponentState::Removed, Some(Reason::taken_out()))
        }
        (None, Observed::Missing) => (ComponentState::NotInstalled, None),
        (None, Observed::Blocked(reason)) => (ComponentState::Failed, Some(reason)),
        (None, Observed::Absent(reason) | Observed::Unsupported(reason)) => {
            (ComponentState::Absent, Some(reason))
        }
    };
    let (reason, why) = Reason::split(reason);
    ComponentReport {
        id,
        state,
        reason,
        why,
        location: Some(location(id, target)),
    }
}

/// How long a kit waits for another kit changing the same account.
const LOCK_DEADLINE: Duration = Duration::from_secs(90);

/// One kit at a time changes an account's files. Two registrations of one
/// machine connect together at launch, and a Mac that is also another Mac's
/// device runs its own kit beside the device's; their copies, hook files and
/// retirement would otherwise interleave. The lock is an advisory lock on a
/// file in the kit's private folder, so it ends with the process that held it.
pub(crate) struct AccountLock {
    _lock: hide_platform::fs::lock::Lock,
}

pub(crate) fn lock_account(target: &KitTarget) -> Result<AccountLock, KitRefusal> {
    use hide_platform::fs::lock::{Mode, Waited, lock_file};
    let path = record::private_state_dir(&target.home, true)?.join(".lock");
    let file = hide_platform::fs::private::open_or_create_file(&path).map_err(|error| {
        KitRefusal::unsafe_path(
            &path,
            format!("{} could not be opened: {error}", path.display()),
        )
    })?;
    match lock_file(file, Mode::Exclusive, LOCK_DEADLINE, &|| {
        target.stop.load(Ordering::Relaxed)
    }) {
        Ok(Waited::Locked(lock)) => Ok(AccountLock { _lock: lock }),
        // The owner is going away and reads no answer.
        Ok(Waited::Cancelled) => Err(KitRefusal::new(
            RefusalCause::Busy,
            None,
            "Hide is quitting",
        )),
        Ok(Waited::TimedOut) => Err(KitRefusal::new(
            RefusalCause::Busy,
            None,
            format!(
                "another Hide was still changing this account's kit after {} seconds",
                LOCK_DEADLINE.as_secs()
            ),
        )),
        Err(error) => Err(KitRefusal::unsafe_path(
            &path,
            format!("{} could not be locked: {error}", path.display()),
        )),
    }
}

/// Judges every part and every agent without changing anything. A machine
/// whose pass would stop before changing anything reads as that refusal, so
/// a re-read never lists parts and agents a pass could not install, and the
/// operator's Try again is answered by the same check.
pub fn status(target: &KitTarget) -> KitReport {
    let mut record = match retirement_preflight(target).and_then(|()| record::load(&target.home)) {
        Ok(record) => record,
        Err(refusal) => return KitReport::refused(refusal),
    };
    if let Err(refusal) = state_route::claim(target, &record) {
        return KitReport::refused(refusal);
    }
    // A machine the kit never ran on reads as `apply` will leave it until the
    // operator chooses; nothing is written for that.
    agent_kit::hold_for_onboarding(&mut record);
    let held = record.awaiting_choice();
    let recorded = |id| record.contains(id);
    let switched_off = |id| agent_kit::part_is_off(&record, id);
    let components: Vec<ComponentReport> = ComponentId::ALL
        .into_iter()
        .map(|id| {
            let observed = observe(id, target);
            // `status` asks Codex no trust question, so Codex's trust is
            // what the last pass found (`codex_trust`).
            let failure = (id == ComponentId::CodexHook
                && matches!(observed, Observed::Current)
                && !switched_off(id))
            .then(|| codex_trust::remembered(target))
            .flatten();
            report(
                id,
                target,
                observed,
                recorded(id),
                switched_off(id),
                failure,
            )
        })
        .collect();
    let agents = agent_kit::status(target, &record, &agent_kit::part_views(&components));
    let daemon = codex_daemon(target);
    KitReport {
        components,
        agents,
        codex_daemon: daemon.feature,
        codex_daemon_on: daemon.on,
        codex_daemon_running: daemon.running,
        codex_daemon_unreadable: daemon.unreadable,
        codex_capability_unread: daemon.unread,
        codex_daemon_off: None,
        held_for_onboarding: held,
        labels_retirement: Retirement::default(),
        legacy_retirement: Retirement::default(),
        herdr: None,
        refused: None,
    }
}

/// Why the machine's Codex gave no answer to `codex features list`: the
/// cause and Codex's exit code, never its path or what it printed, which can
/// name the account (engineering rule 9).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum CodexReadFailure {
    CouldNotStart,
    TimedOut,
    /// Codex exited unsuccessfully with `code`.
    Exited {
        code: i32,
    },
    /// A signal ended Codex.
    Signaled,
    /// Codex printed a `daemon_auto_start` row Hide cannot read.
    Unreadable,
}

/// How a request to turn Codex's shared server off ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CodexDaemonOff {
    /// Codex answered that autostart is off, a read afterwards agrees, and
    /// no daemon answers any more. `no_daemon` is what the version command
    /// said when no daemon answered before the stop, so nothing was stopped;
    /// it is for the core's log only, so a transient failure that read as
    /// "no daemon" stays visible.
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no_daemon: Option<String>,
    },
    /// `reason` is a code the screen turns into a line; `detail` is Codex's
    /// own words, for the core's log only. `StopFailed` always leaves the
    /// setting off; `Unreachable` (Hide quit or the device's helper went
    /// away) may leave it either way, before or after Codex turned it off;
    /// every other reason leaves it as it was.
    Failed {
        reason: CodexDaemonOffFailure,
        #[serde(default)]
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexDaemonOffFailure {
    /// No Codex program was found on the machine.
    CodexMissing,
    /// Codex refused the change or answered something Hide cannot read.
    CodexRefused,
    /// Codex did not answer in time and was stopped.
    TimedOut,
    /// The machine could not be asked: Hide was quitting or the device's
    /// helper was away.
    Unreachable,
    /// The setting is off now, but the running daemon did not stop, came
    /// back, or answered in a way Hide cannot read; the same request again
    /// only stops it (PRD codex-daemon-apply B7, B8).
    StopFailed,
}

/// Turns the machine's Codex shared server off through Codex's own command,
/// reads the setting back, and then stops the daemon that is still running,
/// so a Done is what Codex now says: autostart off and no daemon answering
/// (PRD codex-daemon-apply D-11, B3, B5-B8). The stop disconnects every
/// Codex attached to that daemon; it runs only for this request, which the
/// operator confirmed.
fn turn_codex_daemon_off(target: &KitTarget) -> CodexDaemonOff {
    use hide_agent_hooks::codex_daemon::{
        DaemonSetting, Stopped, SwitchFailure, read_setting, stop, turn_off,
    };
    let failed = |reason, detail: String| CodexDaemonOff::Failed { reason, detail };
    let Some(codex) = target.codex.as_ref() else {
        return failed(
            CodexDaemonOffFailure::CodexMissing,
            "no codex program was found".to_owned(),
        );
    };
    if let Err(error) = turn_off(codex, &target.home, &target.stop) {
        return failed(
            match error.failure {
                SwitchFailure::CouldNotStart => CodexDaemonOffFailure::CodexMissing,
                SwitchFailure::Refused => CodexDaemonOffFailure::CodexRefused,
                SwitchFailure::TimedOut => CodexDaemonOffFailure::TimedOut,
                SwitchFailure::Stopped => CodexDaemonOffFailure::Unreachable,
            },
            error.message,
        );
    }
    match read_setting(codex, &target.home, &target.stop) {
        Ok(DaemonSetting::Off) => {}
        Ok(setting) => {
            return failed(
                CodexDaemonOffFailure::CodexRefused,
                format!("codex still reports the shared server as {setting:?}"),
            );
        }
        Err(error) => return failed(CodexDaemonOffFailure::CodexRefused, error.message),
    }
    match stop(codex, &target.home, &target.stop) {
        Ok(Stopped::Stopped) => CodexDaemonOff::Done { no_daemon: None },
        Ok(Stopped::AlreadyStopped { answer }) => CodexDaemonOff::Done {
            no_daemon: Some(answer),
        },
        // Hide quitting mid-stop is not Codex refusing: the machine could
        // not be asked to the end.
        Err(message) if target.stop.load(std::sync::atomic::Ordering::Relaxed) => {
            failed(CodexDaemonOffFailure::Unreachable, message)
        }
        Err(message) => failed(CodexDaemonOffFailure::StopFailed, message),
    }
}

/// Installs what `scope` allows and answers what every part is afterwards.
///
/// Every part is tried whatever happened to the ones before it. A second
/// apply of the same build changes nothing (engineering rule 11).
/// Retirement preflight precedes even creation of the account lock.
pub fn apply(target: &KitTarget, scope: &Scope) -> KitReport {
    // The operator's own request runs first and is answered whatever else the
    // pass finds; the report that follows reads the setting after it.
    let daemon_off = scope
        .codex_daemon_off
        .then(|| turn_codex_daemon_off(target));
    let mut report = apply_scope(target, scope);
    report.codex_daemon_off = daemon_off;
    report
}

fn apply_scope(target: &KitTarget, scope: &Scope) -> KitReport {
    if let Err(refusal) = retirement_preflight(target) {
        return KitReport::refused(refusal);
    }
    let _lock = match lock_account(target) {
        Ok(lock) => lock,
        Err(refusal) => return KitReport::refused(refusal),
    };
    // Without its record the kit cannot tell which parts are Hide's, so the
    // pass stops here, before anything below changes (engineering rule 4).
    let mut record = match record::load(&target.home) {
        Ok(record) => record,
        Err(refusal) => return KitReport::refused(refusal),
    };
    // Keep a departed owner's state locked until the account update finishes.
    // A second live daemon sharing HOME must never redirect its hooks.
    let _previous_owner = match state_route::claim(target, &record) {
        Ok(lock) => lock,
        Err(refusal) => return KitReport::refused(refusal),
    };
    let (mut legacy_retirement, retirement_failure) = match coordination_retirement::install(target)
    {
        Ok(report) => (report, None),
        Err(reason) => (Retirement::default(), Some(reason)),
    };
    let labels_retirement = labels::retire(target);
    let mut changed = agent_kit::hold_for_onboarding(&mut record);
    changed |= record.remember_state_route(layout::state_dir_from_process(&target.home));
    if retirement_failure.is_none() && !record.has_retired(HCOORD_PLUGIN_ID) {
        record.mark_retired(HCOORD_PLUGIN_ID);
        changed = true;
    }
    let mut components = Vec::with_capacity(ComponentId::ALL.len());
    let mut trust_codex = false;
    for id in ComponentId::ALL {
        // The owner is quitting: what is done is recorded, and the rest waits
        // for the next launch or connection.
        if target.stop.load(Ordering::Relaxed) {
            break;
        }
        let observed = observe(id, target);
        if id == ComponentId::CoordinationRetirement {
            if matches!(observed, Observed::Current) && !record.contains(id) {
                record.insert(id);
                changed = true;
            }
            components.push(report(
                id,
                target,
                observed,
                record.contains(id),
                false,
                retirement_failure.clone(),
            ));
            continue;
        }
        let recorded = record.contains(id);
        // An agent's switch governs the hook part that is its hook: off, the
        // part is never installed or replaced; switched off now, it comes
        // out; switched on now, it is Reinstall's (issue #517).
        let gate = agent_kit::part_gate(&record, scope, id);
        let agent_off = gate.is_some_and(|gate| !gate.enabled);
        let turning_off = gate.is_some_and(|gate| gate.turning_off);
        let restoring = scope.restores(id) || gate.is_some_and(|gate| gate.turning_on);
        let install_now = !turning_off
            && !agent_off
            && match &observed {
                Observed::Stale(_) => true,
                Observed::Edited(_) => restoring,
                Observed::Missing => restoring || !recorded,
                Observed::Current
                | Observed::Blocked(_)
                | Observed::Absent(_)
                | Observed::Unsupported(_) => false,
            };
        // A hook part comes out whatever blocks judging it (a missing helper,
        // an older CLI): only Hide's marked entries are taken.
        let hook_part = matches!(id, ComponentId::ClaudeCodeHook | ComponentId::CodexHook);
        let undo_now = turning_off
            && (matches!(
                observed,
                Observed::Current | Observed::Stale(_) | Observed::Edited(_)
            ) || (hook_part && matches!(observed, Observed::Blocked(_))));
        let mut failure = None;
        if install_now {
            match install(id, target) {
                Ok(()) => {}
                Err(reason) => failure = Some(reason),
            }
        } else if undo_now {
            failure = turn_off(id, target).err();
        }
        let after = if install_now || undo_now {
            observe(id, target)
        } else {
            observed
        };
        // The entries are in place for an agent that is on: Codex is asked to
        // trust them below, once Herdr's integration is in place too (PRD
        // codex-hook-trust D-05, codex-herdr-hook-trust D-03).
        if id == ComponentId::CodexHook
            && failure.is_none()
            && !turning_off
            && !agent_off
            && matches!(after, Observed::Current)
        {
            trust_codex = true;
        }
        // A part the operator turned off is recorded too, so the next pass
        // reads it as theirs rather than as never installed.
        let applied = matches!(after, Observed::Current)
            || (turning_off && matches!(after, Observed::Missing));
        if applied && !record.contains(id) {
            record.insert(id);
            changed = true;
        }
        if matches!(after, Observed::Current) && id == ComponentId::Cli {
            changed |= record.remember_cli(cli::wanted(target));
        }
        let recorded_now = record.contains(id);
        let off = agent_off || agent_kit::part_is_off(&record, id);
        components.push(report(id, target, after, recorded_now, off, failure));
    }
    let applied = agent_kit::apply(target, scope, &mut record);
    changed |= applied.changed;
    // Whether this pass wrote the entries or found them, Codex trusts Hide's
    // and, when the kit installed it, Herdr's, in one check: Herdr's
    // integration is only in place now. A failure is the Codex part's, and the
    // other parts are already installed.
    if trust_codex
        && !target.stop.load(Ordering::Relaxed)
        && let Some(codex) = agents::adapter_of_part(ComponentId::CodexHook)
        && let Some(failure) = codex_trust::ensure(
            target,
            // Herdr's entries only while Herdr says its integration is in place.
            if applied.herdr_current(codex.herdr.target) {
                record.herdr_hook_entries(codex.id)
            } else {
                &[]
            },
        )
        && let Some(part) = components
            .iter_mut()
            .find(|part| part.id == ComponentId::CodexHook)
    {
        part.state = ComponentState::Failed;
        part.why = Some(PartWhy {
            cause: PartCause::CodexUntrusted,
            value: None,
        });
        part.reason = Some(failure);
    }
    let agents = agent_kit::reports(
        target,
        scope,
        &record,
        &applied,
        &agent_kit::part_views(&components),
    );
    legacy_retirement.removed.extend(applied.retirement.removed);
    legacy_retirement
        .failures
        .extend(applied.retirement.failures);
    // The part an earlier build recorded for the Codex setting it used to
    // switch is gone; its record entry goes with it.
    changed |= record.forget_piece("codex_per_pane");
    // What the record says after this pass: a machine that was held and not
    // answered still waits, and a save that fails below leaves the file as it
    // was, so the next pass holds again rather than reading a lost hold as an
    // answer.
    let held_for_onboarding = record.awaiting_choice();
    if changed && let Err(reason) = record::save(&target.home, &record) {
        // The parts are in place; only the memory of having installed them
        // is not, so the next pass reinstalls rather than leaving one out.
        // Each part's row stays as it is.
        for component in &mut components {
            if component.state == ComponentState::Installed {
                component.reason = Some(format!("Installed, but {reason}"));
                component.why = Some(PartWhy {
                    cause: PartCause::RecordUnsaved,
                    value: None,
                });
            }
        }
    }
    // Last, so a hook or link the pass just moved off a legacy folder no
    // longer names it.
    let folders = legacy::retire(target);
    legacy_retirement.removed.extend(folders.removed);
    legacy_retirement.failures.extend(folders.failures);
    let daemon = codex_daemon(target);
    KitReport {
        components,
        agents,
        codex_daemon: daemon.feature,
        codex_daemon_on: daemon.on,
        codex_daemon_running: daemon.running,
        codex_daemon_unreadable: daemon.unreadable,
        codex_capability_unread: daemon.unread,
        codex_daemon_off: None,
        held_for_onboarding,
        labels_retirement,
        legacy_retirement,
        herdr: None,
        refused: None,
    }
}

/// What the machine's Codex says about its shared daemon, read without
/// changing anything or starting a daemon.
#[derive(Default)]
struct DaemonRead {
    /// It has the setting (`KitReport::codex_daemon`).
    feature: Option<bool>,
    /// The setting is on (`KitReport::codex_daemon_on`).
    on: Option<bool>,
    /// A daemon answers (`KitReport::codex_daemon_running`).
    running: Option<bool>,
    /// The daemon's answer Hide could not read.
    unreadable: Option<String>,
    /// Why the setting was not read (`KitReport::codex_capability_unread`).
    unread: Option<CodexReadFailure>,
}

/// Why a capability read had no answer, as the core logs it: the cause and
/// the exit code, never the path or what Codex printed. A read Hide stopped
/// because it is quitting says nothing about the machine.
fn codex_read_failure(
    failure: hide_agent_hooks::codex_daemon::ReadFailure,
) -> Option<CodexReadFailure> {
    use hide_agent_hooks::codex_daemon::{ReadFailure, SwitchFailure};
    match failure {
        ReadFailure::Run(SwitchFailure::Stopped) => None,
        ReadFailure::Run(SwitchFailure::TimedOut) => Some(CodexReadFailure::TimedOut),
        // A run that ended before Codex answered: it never says Refused.
        ReadFailure::Run(SwitchFailure::CouldNotStart | SwitchFailure::Refused) => {
            Some(CodexReadFailure::CouldNotStart)
        }
        ReadFailure::Exited { code: Some(code) } => Some(CodexReadFailure::Exited { code }),
        ReadFailure::Exited { code: None } => Some(CodexReadFailure::Signaled),
        ReadFailure::Unreadable => Some(CodexReadFailure::Unreadable),
    }
}

fn codex_daemon(target: &KitTarget) -> DaemonRead {
    use hide_agent_hooks::codex_daemon::{DaemonSetting, daemon_running, read_setting};
    let Some(codex) = target.codex.as_ref() else {
        return DaemonRead::default();
    };
    let (feature, on, unread) = match read_setting(codex, &target.home, &target.stop) {
        Ok(DaemonSetting::Unsupported) => (Some(false), None, None),
        Ok(DaemonSetting::On) => (Some(true), Some(true), None),
        Ok(DaemonSetting::Off) => (Some(true), Some(false), None),
        // Every Codex start the core makes is refused until this reads, so
        // why it did not reaches the core's log.
        Err(error) => (None, None, codex_read_failure(error.failure)),
    };
    // A Codex older than the setting starts no shared daemon to ask about.
    if feature != Some(true) {
        return DaemonRead {
            feature,
            on,
            unread,
            ..DaemonRead::default()
        };
    }
    let (running, unreadable) = match daemon_running(codex, &target.home, &target.stop) {
        Ok(running) => (Some(running), None),
        Err(reason) => (None, Some(reason)),
    };
    DaemonRead {
        feature,
        on,
        running,
        unreadable,
        unread,
    }
}

/// What removing the kit did to one part.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RemoveOutcome {
    Removed,
    /// Nothing of Hide's was there.
    Absent,
    /// Left in place on purpose; the sentence says why.
    Kept {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoveReport {
    pub components: Vec<(ComponentId, RemoveOutcome)>,
    /// What removing took off for the agents: each guidance hook
    /// (`hook:<agent>`) and each skill folder (`skill:<folder>`).
    #[serde(default)]
    pub agents: Vec<(String, RemoveOutcome)>,
}

/// Takes Hide's parts off a machine that is being removed from Hide: the
/// hook entries carrying Hide's marker, the `hide` link when it is Hide's,
/// and the record. Preserved retirement folders stay in place.
pub fn remove(target: &KitTarget) -> RemoveReport {
    let _lock = match lock_account(target) {
        Ok(lock) => lock,
        Err(KitRefusal { detail: reason, .. }) => {
            return RemoveReport {
                components: ComponentId::ALL
                    .into_iter()
                    .map(|id| {
                        let reason = reason.clone();
                        (id, RemoveOutcome::Failed { reason })
                    })
                    .collect(),
                agents: Vec::new(),
            };
        }
    };
    let mut components = Vec::new();
    for id in ComponentId::ALL {
        let outcome = match id {
            ComponentId::Cli => cli::remove(target),
            ComponentId::ClaudeCodeHook => {
                hooks::remove(target, hide_agent_hooks::AgentRuntime::ClaudeCode)
            }
            ComponentId::CodexHook => hooks::remove(target, hide_agent_hooks::AgentRuntime::Codex),
            ComponentId::CoordinationRetirement => RemoveOutcome::Kept {
                reason: "retirement preserves the old ledger folder".to_owned(),
            },
        };
        components.push((id, outcome));
    }
    // Before the record goes: it is what says which integrations were Hide's.
    let herdr = match record::load(&target.home) {
        Ok(record) => agent_kit::remove_herdr(target, &record),
        Err(_) => Vec::new(),
    };
    if let Err(reason) = record::forget(&target.home) {
        components.push((
            ComponentId::Cli,
            RemoveOutcome::Failed {
                reason: format!("the install record stayed: {reason}"),
            },
        ));
    }
    let mut agents = agent_kit::remove(target);
    agents.extend(herdr);
    RemoveReport { components, agents }
}

/// `base` joined with `parts`, each folder checked in turn: the kit keeps
/// code there that launchd and Herdr run as this account, so a folder that
/// is a link, belongs to another account or that others can write to is
/// refused rather than trusted. With `create`, a missing folder is made 0700;
/// without it, a missing folder ends the check, since nothing below it can be
/// there yet.
pub(crate) fn private_dirs(
    base: &Path,
    parts: &[&str],
    create: bool,
) -> Result<PathBuf, KitRefusal> {
    use hide_platform::fs::private;
    let mut folder = base.to_path_buf();
    for part in parts {
        folder.push(part);
        if create {
            match private::create_dir(&folder) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(KitRefusal::unsafe_path(
                        &folder,
                        format!("{} could not be created: {error}", folder.display()),
                    ));
                }
            }
        }
        let metadata = match std::fs::symlink_metadata(&folder) {
            Ok(metadata) => metadata,
            Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(base.join(parts.join("/")));
            }
            Err(error) => {
                return Err(KitRefusal::unsafe_path(
                    &folder,
                    format!("{} could not be read: {error}", folder.display()),
                ));
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(KitRefusal::unsafe_path(
                &folder,
                format!(
                    "{} is a link; Hide requires an owned real folder",
                    folder.display()
                ),
            ));
        }
        if !metadata.is_dir() {
            return Err(KitRefusal::unsafe_path(
                &folder,
                format!("{} is not a folder", folder.display()),
            ));
        }
        let trusted = private::owned_by_current_user(&folder)
            .and_then(|owned| Ok(owned && !private::others_can_modify(&folder)?))
            .map_err(|error| {
                KitRefusal::unsafe_path(
                    &folder,
                    format!("{} could not be checked: {error}", folder.display()),
                )
            })?;
        if !trusted {
            return Err(KitRefusal::unsafe_path(
                &folder,
                format!(
                    "{} can be changed by another account, so Hide keeps nothing it runs there",
                    folder.display()
                ),
            ));
        }
    }
    Ok(folder)
}

/// Replaces `path` with `contents` through a temporary file beside it, so a
/// failure part way leaves the old file whole.
pub(crate) fn write_atomically(
    path: &Path,
    contents: &[u8],
    access: hide_platform::fs::Access,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
    hide_platform::fs::atomic::write_file(path, contents, access)
        .map(drop)
        .map_err(|error| format!("{} could not be written: {error}", path.display()))
}

// The suite drives the kit against a stand-in Herdr on a Unix socket and with
// shell scripts for the programs it installs, so it runs where those do; the
// Windows kit is tested with the device helper that installs it there.
#[cfg(all(test, unix))]
mod tests;
