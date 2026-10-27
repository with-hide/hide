//! Static agent declarations shared by hooks, sessions, installation and UI.
//!
//! A dialect id selects code in its existing owner. This crate performs no
//! I/O, starts no work and stores no runtime state. Lookup borrows the input
//! and static rows: no allocation, lock, normalization buffer or cache.

pub mod composer;
mod declarations;
mod journey;
pub mod usage;
pub use declarations::ADAPTERS;
pub use journey::{JourneyAdapter, JourneyCells, JourneyLetter, JourneyWake, journey_contract};
mod web;
pub use web::{HookKind, WebAdapter, web_contract};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Grok,
    OpenCode,
    Pi,
    Omp,
    Antigravity,
    Cursor,
}

impl AgentId {
    pub const fn adapter(self) -> &'static AgentAdapter {
        match self {
            Self::Cursor => &declarations::RETIRED_CURSOR,
            _ => &ADAPTERS[self as usize],
        }
    }
}

/// The input and output protocol of an agent's own hook: how its payload
/// names the shell call and session, and how a refusal is written. Claude
/// Code's and Codex's six-event hook, a script file of Hide's in the agent's
/// own folder (OpenCode's plugin, Pi's and omp's extension), which calls the
/// same helper, and Grok's own hook file. Cursor's dialect remains for legacy cleanup.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HookDialect {
    ClaudeCode,
    Codex,
    OpenCode,
    Grok,
    Cursor,
    Pi,
    Omp,
    Antigravity,
}

impl HookDialect {
    pub const ALL: [Self; 7] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::OpenCode,
        Self::Grok,
        Self::Pi,
        Self::Omp,
        Self::Antigravity,
    ];

    pub const fn adapter(self) -> &'static AgentAdapter {
        match self {
            Self::ClaudeCode => AgentId::ClaudeCode.adapter(),
            Self::Codex => AgentId::Codex.adapter(),
            Self::OpenCode => AgentId::OpenCode.adapter(),
            Self::Grok => AgentId::Grok.adapter(),
            Self::Cursor => AgentId::Cursor.adapter(),
            Self::Pi => AgentId::Pi.adapter(),
            Self::Omp => AgentId::Omp.adapter(),
            Self::Antigravity => AgentId::Antigravity.adapter(),
        }
    }
}

/// Hide's own hook file for an agent of the basic tier
/// (`hide_agent_hooks::guidance`): the events that agent documents, under
/// one marker, beside every other tool's hooks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuidanceDialect {
    Cursor,
    Grok,
    /// agy's `hide` block in `~/.gemini/config/hooks.json`, beside Herdr's
    /// and the operator's blocks: the collaboration tier through a command
    /// hook, since agy's `injectSteps` put text into a prompt and its
    /// `PreToolUse` refuses a call.
    Antigravity,
}

impl GuidanceDialect {
    /// Whether the file's session-start entry adds Hide's guidance to the
    /// agent's context. Grok discards what a session-start hook prints.
    pub const fn prints_guidance(self) -> bool {
        matches!(self, Self::Cursor | Self::Antigravity)
    }
}

/// A script file Hide owns in the agent's own plugin or extension folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginDialect {
    OpenCode,
    /// Pi's and omp's extension: one source, two files.
    Pi,
    Omp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookInstall {
    Runtime(HookDialect),
    Guidance(GuidanceDialect),
    Plugin(PluginDialect),
    None,
}

/// Where context actually enters the model, independently of installing a hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextRoute {
    /// Native SessionStart guidance/index and UserPromptSubmit Memory capsules.
    NativeHook(HookDialect),
    /// A guidance hook whose output the host consumes as model context.
    GuidanceHook(GuidanceDialect),
    /// The host's prompt event carries guidance/index on the first prompt,
    /// and matching Memory cards on subsequent prompts.
    PluginPrompt(PluginDialect),
}

/// A missing context/title capability is a decision with a verifiable reference,
/// never an omitted field or an unexplained `None`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityAbsence {
    pub reason: &'static str,
    /// Repository-relative owning guide or recorded contract fixture.
    pub evidence: &'static str,
    pub would_change: &'static str,
}

pub type Capability<S> = usage::Support<S, CapabilityAbsence>;

/// Passive tool-result observation is an independent, required decision.
/// The hook owner implements the selected dialect; unsupported agents must
/// explain why no result can be observed rather than silently omitting it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationSupport {
    Native(HookDialect),
    Plugin(PluginDialect),
    Unsupported { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionFormat {
    Claude,
    Codex,
    Grok,
    Pi,
    Omp,
    Cursor,
    OpenCode,
    Antigravity,
}

impl SessionFormat {
    pub const fn adapter(self) -> &'static AgentAdapter {
        match self {
            Self::Claude => AgentId::ClaudeCode.adapter(),
            Self::Codex => AgentId::Codex.adapter(),
            Self::Grok => AgentId::Grok.adapter(),
            Self::Pi => AgentId::Pi.adapter(),
            Self::Omp => AgentId::Omp.adapter(),
            Self::Cursor => AgentId::Cursor.adapter(),
            Self::OpenCode => AgentId::OpenCode.adapter(),
            Self::Antigravity => AgentId::Antigravity.adapter(),
        }
    }

    pub const fn reports_turns(self) -> bool {
        matches!(
            self,
            Self::Claude | Self::Codex | Self::Grok | Self::Omp | Self::OpenCode
        )
    }
    /// Whether Herdr keeps reporting the agent `working` while it waits for
    /// the operator's plan approval or answer, and the session records say
    /// what it waits for as the session is now (Grok's `plan_mode.json`, a
    /// question call with no result yet): such a wait outranks the status.
    /// Codex's plan menu is read from how the last turn ended, which says
    /// nothing once Herdr reports the next turn running, so it does not.
    pub const fn waits_while_working(self) -> bool {
        matches!(self, Self::Grok)
    }
    /// Whether the session records prove which background tasks are still
    /// running and when a new process began. Claude Code's do; Codex writes no
    /// record when a session's command ends, and no other format keeps a
    /// process boundary, so their waits are Herdr's alone (PRD
    /// agent-blocked-state B18, D-26).
    pub const fn reports_wake_devices(self) -> bool {
        matches!(self, Self::Claude)
    }
    pub const fn has_session_file(self) -> bool {
        matches!(
            self,
            Self::Claude
                | Self::Codex
                | Self::Grok
                | Self::Pi
                | Self::Omp
                | Self::Cursor
                | Self::Antigravity
        )
    }

    pub const fn is_jsonl(self) -> bool {
        matches!(
            self,
            Self::Claude | Self::Codex | Self::Grok | Self::Pi | Self::Omp | Self::Antigravity
        )
    }

    /// Native file metadata and the default CLI resolver must agree before
    /// a transcript can authorize a read or a lifecycle effect.
    pub const fn requires_native_file_proof(self) -> bool {
        matches!(
            self,
            Self::Grok | Self::Pi | Self::Omp | Self::Cursor | Self::Antigravity
        )
    }

    /// The native reader proves the session's owner and checkout before a
    /// lifecycle effect (wake, fork, resume) uses its id, and Herdr's
    /// reported id alone grants none of them: every native-file reader, and
    /// OpenCode's database reader.
    pub const fn requires_native_proof(self) -> bool {
        self.requires_native_file_proof() || matches!(self, Self::OpenCode)
    }

    /// The reported reference a native proof takes: OpenCode names a session
    /// by its id in OpenCode's database, and Herdr's agy integration reports
    /// the conversation id, whose transcript the reader finds by it; a file
    /// reader names the file's path.
    pub const fn proof_reference_kind(self) -> &'static str {
        match self {
            Self::OpenCode | Self::Antigravity => "id",
            Self::Claude | Self::Codex | Self::Grok | Self::Pi | Self::Omp | Self::Cursor => "path",
        }
    }

    /// The native reader answers a session's modification time and size.
    pub const fn reports_activity(self) -> bool {
        self.has_session_file() || matches!(self, Self::OpenCode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchDialect {
    Claude,
    Codex,
    Grok,
    OpenCode,
    Pi,
    Omp,
    Cursor,
    Antigravity,
}

impl LaunchDialect {
    pub const fn adapter(self) -> &'static AgentAdapter {
        match self {
            Self::Claude => AgentId::ClaudeCode.adapter(),
            Self::Codex => AgentId::Codex.adapter(),
            Self::Grok => AgentId::Grok.adapter(),
            Self::OpenCode => AgentId::OpenCode.adapter(),
            Self::Pi => AgentId::Pi.adapter(),
            Self::Omp => AgentId::Omp.adapter(),
            Self::Cursor => AgentId::Cursor.adapter(),
            Self::Antigravity => AgentId::Antigravity.adapter(),
        }
    }

    /// How the CLI takes a model and a reasoning effort at launch; `{}` is
    /// replaced by the value. This is the one rule every start and restart
    /// follows (the start surfaces, `hide agent spawn`, and
    /// a wake, fork or reopen of any of them), so a model or effort an agent
    /// does not declare is refused wherever it is asked for. Verified against
    /// Claude Code 2.1 and Codex 0.160, agy 1.4.0's model (`--model`, listed
    /// by `agy models`, which Hide AI's agy backend reads and passes too), and
    /// the `--model` that the help of Grok 1.0.52, Pi 1.0.4, omp 18.7.0 and
    /// Cursor CLI 2026.10.14 lists (read 2026-10-18; their efforts are not
    /// declared yet). OpenCode 2.0.29's lists no model option, so it takes
    /// neither.
    pub const fn options(self) -> LaunchOptions {
        match self {
            Self::Antigravity => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
            },
            Self::Claude => LaunchOptions {
                model: &["--model", "{}"],
                effort: &["--effort", "{}"],
                efforts: &["low", "medium", "high", "xhigh", "max"],
            },
            // `-m` is the short form; every agent spells a model `--model`.
            Self::Codex => LaunchOptions {
                model: &["--model", "{}"],
                effort: &["-c", "model_reasoning_effort={}"],
                efforts: &["minimal", "low", "medium", "high", "xhigh"],
            },
            Self::Grok | Self::Pi | Self::Omp | Self::Cursor => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
            },
            Self::OpenCode => LaunchOptions {
                model: &[],
                effort: &[],
                efforts: &[],
            },
        }
    }

    /// The first interactive prompt is an argument, held by the native CLI
    /// through startup questions. OpenCode's positional argument is a project;
    /// agy takes the prompt of an interactive session only through `-i`
    /// (`--prompt-interactive`), a bare argument being refused.
    pub const fn prompt_flag(self) -> &'static str {
        match self {
            Self::OpenCode => "--prompt",
            Self::Antigravity => "-i",
            _ => "--",
        }
    }

    /// Verified launch dialects that accept repeated extra permission roots.
    /// A checkout cwd is independent of this optional CLI argument.
    pub const fn accepts_extra_directories(self) -> bool {
        matches!(self, Self::Claude | Self::Codex | Self::Omp | Self::Cursor)
    }

    pub const fn closes_pane_when_sleeping(self) -> bool {
        !matches!(self, Self::Claude | Self::Codex | Self::Antigravity)
    }

    /// The keys, as Herdr names them, that make the idle agent quit on its
    /// own, as the operator quits it: a sleep in the agent's own pane presses
    /// them first, so the agent puts back the terminal modes it set (bracketed
    /// paste, extended keys) before the shell takes the terminal again
    /// (herdrdev/herdr#5371: a Windows prompt reads a bracketed paste left on
    /// as keys), and ends it only when it does not quit. Each is a press of
    /// its own, read within the agent's double-press window: Claude Code
    /// 2.1.302 quits on a second Ctrl+C within 800 ms of the first ("Press
    /// Ctrl-C again to exit"), Codex CLI on a second Ctrl+C ("again to quit")
    /// and agy 1.4.2 on "ctrl+c or ctrl+d twice"; Ctrl+D is never pressed,
    /// because a shell that already holds the terminal ends on it. A first
    /// Ctrl+C clears a draft instead, and that agent is then ended. Empty for
    /// an agent that closes its pane when sleeping.
    pub const fn exit_keys(self) -> &'static [&'static str] {
        match self {
            Self::Claude | Self::Codex | Self::Antigravity => &["ctrl+c", "ctrl+c"],
            Self::Grok | Self::OpenCode | Self::Pi | Self::Omp | Self::Cursor => &[],
        }
    }

    /// Whether the agent asks before it works in a folder it has not been
    /// told to trust, so Hide records that trust for a worktree it made
    /// before it starts the agent there: the question would otherwise hold a
    /// delegated child that nobody is watching (PRD antigravity-first-class
    /// D-08; Claude Code 2.1.302 and Codex 0.163.0 ask the same, measured
    /// 2026-10-17, and Grok 1.0.67, measured 2026-10-23; docs/agent-hooks.md,
    /// Folder trust for the worktrees Hide makes).
    pub const fn trusts_worktrees(self) -> bool {
        matches!(
            self,
            Self::Claude | Self::Codex | Self::Antigravity | Self::Grok
        )
    }

    /// Whether an agent that has reported no conversation yet can still
    /// sleep, waking as a new one in the same pane: agy reports its
    /// conversation only once its first prompt is sent (PRD
    /// antigravity-first-class D-10).
    pub const fn sleeps_without_session(self) -> bool {
        matches!(self, Self::Antigravity)
    }

    /// Whether the agent, resumed with its `resume_flag`, tells Herdr which
    /// session it runs. Cursor's `sessionStart` hook is Herdr's only source
    /// for that, and Cursor CLI 2026.10.01 does not run it on `--resume`
    /// (issue 938), so the pane that resumed it has no session in Herdr
    /// until whoever started the resume says which one it resumed. agy 1.3.6
    /// reports its conversation from `PreInvocation`, which runs only once the
    /// next prompt is sent, so a resumed agy pane waits for the operator too.
    pub const fn resume_reports_session(self) -> bool {
        !matches!(self, Self::Cursor | Self::Antigravity)
    }

    /// The native selector prefix, independent of whether this build has
    /// implemented and declared the reader needed to resume that agent.
    pub const fn resume_flag(self) -> &'static str {
        match self {
            Self::Codex => "resume",
            Self::Pi => "--session",
            Self::OpenCode => "-s",
            Self::Claude | Self::Grok | Self::Omp | Self::Cursor => "--resume",
            Self::Antigravity => "--conversation",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchOptions {
    pub model: &'static [&'static str],
    pub effort: &'static [&'static str],
    pub efforts: &'static [&'static str],
}

/// The one check of a model name, wherever a start or a restart takes one.
/// It reaches the agent's command line as one argument: the pinned Herdr
/// types each argument into the pane's shell in single quotes, so a glob or
/// a parameter in it stays literal (`opus[1m]` and `$HOME` reached the agent
/// unchanged through zsh, measured on Herdr 0.9.5, 2026-10-18), and the name
/// is held to the characters model ids use so it can never read as an option
/// or carry a space or a control character.
pub fn valid_model(model: &str) -> bool {
    // A leading dash would read as an option to the agent's CLI.
    !model.is_empty()
        && !model.starts_with('-')
        && model.len() <= 80
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/[]".contains(&byte))
}

/// The model and reasoning effort an agent was started with; both empty is
/// the CLI's own default. Hide keeps it with the pane it started the agent in
/// and with every record a restart is made from, so a wake, a fork or a
/// reopen starts the agent with the same choice: a CLI resuming a session
/// does not bring them back by itself (Claude Code 2.1.302 resumes and forks
/// with its configured effort, Codex 0.163.0 forks with its configured model
/// and effort; measured 2026-10-18).
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaunchChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl LaunchChoice {
    pub fn is_default(&self) -> bool {
        self.model.is_none() && self.effort.is_none()
    }
}

impl LaunchDialect {
    /// The arguments a start or a restart adds for `choice`, in the form this
    /// dialect declares (`options`); refused with the reason for a model or
    /// an effort it does not take.
    pub fn choice_arguments(self, choice: &LaunchChoice) -> Result<Vec<String>, String> {
        self.options()
            .arguments(choice.model.as_deref(), choice.effort.as_deref())
    }
}

impl LaunchOptions {
    /// The launch arguments for a model and an effort, refused with the
    /// reason when either is not something this CLI declares.
    pub fn arguments(
        self,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut arguments = Vec::new();
        if let Some(model) = model {
            if self.model.is_empty() {
                return Err("this agent takes no model at launch".to_owned());
            }
            if !valid_model(model) {
                return Err(format!("model `{model}` is not a model name"));
            }
            arguments.extend(self.model.iter().map(|part| part.replace("{}", model)));
        }
        if let Some(effort) = effort {
            if self.efforts.is_empty() {
                return Err("this agent takes no effort at launch".to_owned());
            }
            if !self.efforts.contains(&effort) {
                return Err(format!(
                    "effort `{effort}` is not one of {}",
                    self.efforts.join(", ")
                ));
            }
            arguments.extend(self.effort.iter().map(|part| part.replace("{}", effort)));
        }
        Ok(arguments)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindDialect {
    ClaudeTranscript,
    CodexTranscript,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillLocation {
    Shared,
    Claude,
    /// The agent's own global skill folder, which it alone reads: agy's model
    /// gets neither `~/.agents/skills` nor `~/.claude/skills`, and of agy's
    /// own folders only `~/.gemini/config/skills` (agy 1.4.0, measured: its
    /// documented `~/.gemini/antigravity-cli/skills` is listed by `/skills`
    /// but not given to the model).
    Antigravity,
}

#[derive(Clone, Copy, Debug)]
pub struct HerdrIntegration {
    /// The agent kind Herdr reports the agent's panes as.
    pub name: &'static str,
    /// The `herdr integration install <target>` name, which is the kind for
    /// every agent but agy (kind `agy`, target `antigravity-cli`).
    pub target: &'static str,
    pub folder: &'static [&'static str],
}

impl HerdrIntegration {
    /// The source the agent's own Herdr integration reports its sessions
    /// under, named by the integration target (`herdr:antigravity_cli` for agy).
    pub fn session_source(&self) -> String {
        format!("herdr:{}", self.target.replace('-', "_"))
    }
}

/// A submission hook may observe a prompt even when its output is discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptHook {
    pub dialect: HookDialect,
    pub context: bool,
}

/// The oldest version of an agent's CLI that Hide's integration (its hook,
/// plugin or extension) was proven on, and where that proof is recorded. It is
/// one version per agent, the oldest proven on every system Hide runs on
/// (macOS, Linux and Windows), never the oldest proven on any one of them, and
/// never a minimum per system. Proven means run: a reading of the agent's
/// source is not a run and sets no minimum. The kit puts the integration in place only for
/// a CLI that answers this version or a newer one (docs/agent-hooks.md, The
/// oldest version of each agent).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MinimumVersion {
    /// `x.y.z`, compared part by part with what the CLI's `--version` answers.
    pub version: &'static str,
    /// Repository-relative guide section or captured fixture holding the proof.
    pub evidence: &'static str,
}

/// The value of [`MINIMUM_LIFT_VARIABLE`] that lifts the refusal; any other
/// value, or none, keeps it.
pub const MINIMUM_LIFT_VALUE: &str = "lift-for-agent-floor-measurement";
/// The build-time variable the agent floor measurement sets
/// (`.github/workflows/agent-floor.yml`), named in docs/agent-hooks.md, The
/// oldest version of each agent.
pub const MINIMUM_LIFT_VARIABLE: &str = "HIDE_AGENT_FLOOR_MEASUREMENT";

/// Whether this build puts each agent's integration in place below its
/// [`MinimumVersion`], so a CLI older than the minimum can be measured at
/// all. It is read when the crate is compiled, never from a running
/// program's environment, so no installed Hide can be changed by a variable
/// set on the operator's machine; only a build whose environment holds
/// [`MINIMUM_LIFT_VARIABLE`] at exactly [`MINIMUM_LIFT_VALUE`] sets it, and
/// `desktop/scripts/package.mjs` refuses to package from such an environment.
pub const MINIMUM_REFUSAL_LIFTED: bool = match option_env!("HIDE_AGENT_FLOOR_MEASUREMENT") {
    Some(value) => same(value.as_bytes(), MINIMUM_LIFT_VALUE.as_bytes()),
    None => false,
};

const fn same(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Independent gates; adding prompt intake never grants bell or shell denial.
#[derive(Clone, Copy, Debug)]
pub struct AgentAdapter {
    pub key: AgentId,
    pub id: &'static str,
    pub label: &'static str,
    pub picker_label: &'static str,
    pub sidebar_label: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub executables: &'static [&'static str],
    /// Every supported agent declares one; only a retired agent, for which
    /// Hide installs nothing, has none.
    pub minimum: Option<MinimumVersion>,
    pub skill_location: SkillLocation,
    pub hook: HookInstall,
    pub observation: ObservationSupport,
    pub guidance: Capability<ContextRoute>,
    pub herdr: HerdrIntegration,
    pub default_on: bool,
    pub prompt_hook: Option<PromptHook>,
    pub spawn_guard: Option<HookDialect>,
    pub subagent_counts: Option<HookDialect>,
    /// Automatic model-context injection, not the Memory store or extraction.
    pub memory: Capability<ContextRoute>,
    pub bell: bool,
    pub bell_names_inbox: bool,
    pub turn_end_letters: Option<HookDialect>,
    /// The dialect whose hook, plugin or extension hands letters over at a
    /// tool event while the agent works (PRD letters-to-busy-agents D-02),
    /// A kind with this path may only be belled at rest.
    pub tool_letters: Option<HookDialect>,
    pub session: Option<SessionFormat>,
    pub sleep: Option<LaunchDialect>,
    pub fork: Option<LaunchDialect>,
    pub resume: Option<LaunchDialect>,
    pub conversation: Option<SessionFormat>,
    pub titles: Capability<SessionFormat>,
    /// Where the agent draws its composer and what an empty one shows, so
    /// a letter held on a possible draft can be released when the screen
    /// says the composer holds nothing (`docs/delivery.md`, Reading the
    /// composer).
    pub composer: Capability<composer::Composer>,
    pub start: Option<LaunchDialect>,
    /// Safety declaration for keyless journeys only; never changes a real login HOME.
    pub fake_home: journey::FakeHome,
    pub find: Option<FindDialect>,
    pub usage: usage::UsageDeclaration,
    pub doc_url: &'static str,
    pub install_url: &'static str,
    pub logo_id: &'static str,
    pub terminal_click: bool,
}

/// Herdr spellings, canonical ids and case variants share this one boundary.
pub fn adapter(value: &str) -> Option<&'static AgentAdapter> {
    let value = value.trim();
    ADAPTERS.iter().find(|row| {
        row.id.eq_ignore_ascii_case(value)
            || row
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(value))
    })
}

/// Known identities use Herdr's spelling; an unknown captured kind stays
/// byte-for-byte intact, without granting it any adapter capability.
pub fn canonical_kind(value: &str) -> &str {
    adapter(value).map_or(value, |row| row.herdr.name)
}

pub fn start_kind(value: &str) -> Option<&'static str> {
    let row = adapter(value)?;
    row.start.map(|_| row.herdr.name)
}

const fn start_count() -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < ADAPTERS.len() {
        if ADAPTERS[index].start.is_some() {
            count += 1;
        }
        index += 1;
    }
    count
}

const fn start_kinds() -> [&'static str; start_count()] {
    let mut result = [""; start_count()];
    let mut source = 0;
    let mut destination = 0;
    while source < ADAPTERS.len() {
        if ADAPTERS[source].start.is_some() {
            result[destination] = ADAPTERS[source].herdr.name;
            destination += 1;
        }
        source += 1;
    }
    result
}

pub const START_KINDS: [&str; start_count()] = start_kinds();

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Skill,
    Guidance,
    Letters,
    Bell,
    Memory,
    SpawnGuard,
    HerdrIntegration,
    Sleep,
    Fork,
    Start,
    Titles,
}

impl Feature {
    pub const ALL: [Self; 11] = [
        Self::Skill,
        Self::Guidance,
        Self::Letters,
        Self::Bell,
        Self::Memory,
        Self::SpawnGuard,
        Self::HerdrIntegration,
        Self::Sleep,
        Self::Fork,
        Self::Start,
        Self::Titles,
    ];
}

impl AgentAdapter {
    pub const fn supports(&self, feature: Feature) -> bool {
        match feature {
            Feature::Skill | Feature::HerdrIntegration => true,
            Feature::Guidance => self.guidance.is_some(),
            Feature::Letters => matches!(self.prompt_hook, Some(PromptHook { context: true, .. })),
            Feature::Bell => self.bell,
            Feature::Memory => self.memory.is_some(),
            Feature::SpawnGuard => self.spawn_guard.is_some(),
            Feature::Sleep => self.sleep.is_some(),
            Feature::Fork => self.fork.is_some(),
            Feature::Start => self.start.is_some(),
            // A label reader alone is not the session features offered by
            // Settings: titles count once the conversation is read.
            Feature::Titles => self.conversation.is_some(),
        }
    }

    pub const fn basic(&self) -> bool {
        !matches!(self.prompt_hook, Some(PromptHook { context: true, .. }))
            || self.spawn_guard.is_none()
    }
}
