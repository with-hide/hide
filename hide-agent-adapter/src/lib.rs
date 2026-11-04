//! Static agent declarations shared by hooks, sessions, installation and UI.
//!
//! A dialect id selects code in its existing owner; who each agent is comes
//! from its definition file (`definition`). This crate reads no file and no
//! network at run time, starts no work and keeps no state of a caller's.
//! An adapter lookup borrows the input and static rows: no allocation, lock,
//! normalization buffer or cache. The one exception is
//! `definition::builtins`, which compiles the built-in files' screen rules
//! once, on first use, behind a `LazyLock`, and keeps them for the process.

pub mod composer;
mod declarations;
pub mod definition;
mod home;
pub use home::{HomeScope, HomeUnresolved, antigravity_app_data};
mod journey;
pub mod usage;
pub use declarations::ADAPTERS;
pub use journey::{JourneyAdapter, JourneyCells, JourneyLetter, JourneyWake, journey_contract};
mod web;
pub use web::{HookKind, WebAdapter, web_contract};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

    /// Whether a typed first prompt has to wait until the agent reported its
    /// session to Herdr. omp 18.7.0 takes text while its extensions' session
    /// start still runs but drops the Enter that follows, so a brief typed
    /// before the report stays unsubmitted in its composer (measured
    /// 2026-10-31 on macOS: every lost brief was typed before the report,
    /// every taken one after; docs/delivery.md, Input an agent did not take).
    /// omp reports its session at start, so the wait costs no turn.
    pub const fn brief_after_session(self) -> bool {
        matches!(self, Self::Omp)
    }

    /// Whether the agent takes a multi-line paste as one paste, never
    /// sending it partway, on a system (`windows` or not): what the agent
    /// journey's `composer` cell measured. On Windows Grok reads console keys
    /// and is never told of a paste, so a pause turns a line break into Enter
    /// (docs/INSTALL.md, What each system supports); Cursor is retired.
    pub const fn takes_one_paste(self, windows: bool) -> bool {
        match self {
            Self::Grok => !windows,
            Self::Cursor => false,
            _ => true,
        }
    }

    /// Whether the agent's own prompt hook carries the prompt it took, so its
    /// digest is the agent's record of typed input: the hook files of Claude
    /// Code, Codex and Grok, which hand `UserPromptSubmit`'s prompt to Hide's
    /// helper. The others record typed input as the person's message in
    /// their session file (Pi and omp measured 2026-10-31; docs/delivery.md,
    /// Input an agent did not take).
    /// Whether a turn the agent starts after its brief was typed is its
    /// record of taking the brief. omp 18.7.0 writes the person's message to
    /// its session only when the turn ends (measured 2026-10-31 on macOS:
    /// 25.6 to 28.3 s after the typing with the model held 20 s, within
    /// 1.4 s of its rest), so a long first turn would otherwise leave the
    /// brief unrecorded for the whole turn; a freshly spawned omp has no
    /// other work to start one for (docs/delivery.md, Input an agent did
    /// not take).
    pub const fn turn_takes_brief(self) -> bool {
        matches!(self, Self::Omp)
    }

    pub const fn prompt_hook_records(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex | Self::Grok)
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

/// How Hide reads the agent's session as a thread of ACP updates
/// (`hide-session`, `thread`; PRD session-thread-acp D-06): the record format
/// its converter reads, and what that agent's records cannot show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadReading {
    pub format: SessionFormat,
    /// Each part the agent's same-scenario sample cannot show, with why.
    /// A part the converter does draw from that sample must not be listed.
    pub absent: &'static [(ThreadPart, CapabilityAbsence)],
}

/// The parts of a thread every agent's same-scenario sample is checked for
/// (`hide-session/tests/it/thread.rs`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ThreadPart {
    /// An image the operator attached or the agent read.
    Image,
    /// A created or changed file, as a diff.
    FileChange,
    /// A command with its output and how it ended.
    CommandOutput,
}

impl ThreadPart {
    pub const ALL: [Self; 3] = [Self::Image, Self::FileChange, Self::CommandOutput];
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
    /// neither. The effort menus are also the ones Hide AI offers
    /// (`AgentAdapter::efforts`): Claude Code's print mode takes the same
    /// `--effort` (claude 2.1.294 `--help`), and Codex app-server's
    /// `turn/start` the same `ReasoningEffort` (codex-cli 0.160.1,
    /// `codex app-server generate-json-schema`).
    ///
    /// `full_access` is the option that starts the agent without asking
    /// before each tool call, which Settings › Agents adds to every start and
    /// restart Hide makes unless the operator chose otherwise for the agent
    /// on that machine. Every dialect declares it, an empty one as
    /// `NotAsked`, so a new dialect cannot leave it out; each flag was read
    /// from the CLI's `--help` on 2026-11-01 (Claude Code 2.1.316, Codex
    /// 0.166.0, Cursor CLI 2026.10.14, agy 1.4.12, OpenCode 2.0.29, Grok
    /// 1.0.70, omp 18.7.0), and the `agent CLIs` lane reads it again from the
    /// pinned version's help on every system. Claude Code 2.1.316 stops a
    /// start with that flag at a one-time "Bypass Permissions mode" warning
    /// whose preselected answer exits, so its start also carries the setting
    /// that skips only that warning, as `--settings` JSON: nothing is written
    /// to its own files, and a brief typed at the start reaches the prompt.
    ///
    /// The operator's shell alias of the program may add the same option
    /// again, since the pinned Herdr types the program's bare name, so each
    /// full-access option is one its CLI accepts twice (measured 2026-11-01 in
    /// an empty HOME with the option given twice): Claude Code 2.1.316, agy
    /// 1.4.12, OpenCode 2.0.29 and omp 18.7.0 start.
    /// Codex 0.166.0 refuses `--dangerously-bypass-approvals-and-sandbox`
    /// twice ("cannot be used multiple times"), and `-a never -s
    /// danger-full-access` twice or beside that flag, so it takes the two
    /// `-c` overrides that flag sets: Codex accepts them repeated and beside
    /// the flag, and its `/status` shows "Full Access" and the prompt input
    /// `codex debug prompt-input` renders is the same as the flag's. Grok
    /// 1.0.70 refuses `--always-approve` twice and `--permission-mode` twice,
    /// and has no option that does the same and may be repeated, so an alias
    /// that adds `--always-approve` stops a Grok start (docs/INSTALL.md).
    pub const fn options(self) -> LaunchOptions {
        match self {
            Self::Antigravity => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::Options(&[&["--dangerously-skip-permissions"]]),
            },
            Self::Claude => LaunchOptions {
                model: &["--model", "{}"],
                effort: &["--effort", "{}"],
                efforts: &["low", "medium", "high", "xhigh", "max"],
                full_access: FullAccess::Options(&[
                    &["--dangerously-skip-permissions"],
                    &[
                        "--settings",
                        r#"{"skipDangerousModePermissionPrompt":true}"#,
                    ],
                ]),
            },
            // `-m` is the short form; every agent spells a model `--model`.
            Self::Codex => LaunchOptions {
                model: &["--model", "{}"],
                effort: &["-c", "model_reasoning_effort={}"],
                efforts: &["minimal", "low", "medium", "high", "xhigh"],
                // `--dangerously-bypass-approvals-and-sandbox` sets these
                // two, but Codex refuses it twice; a repeated `-c` is
                // accepted, beside that flag too (see above). One option,
                // both overrides, so Settings › Agents shows the sandbox too.
                full_access: FullAccess::Options(&[&[
                    "-c",
                    "approval_policy=never",
                    "-c",
                    "sandbox_mode=danger-full-access",
                ]]),
            },
            Self::Grok => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::Options(&[&["--always-approve"]]),
            },
            // Pi 1.0.4 asks before no tool call; its `--approve` trusts
            // project-local files and is not a permission switch.
            Self::Pi => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::NotAsked,
            },
            Self::Omp => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::Options(&[&["--auto-approve"]]),
            },
            Self::Cursor => LaunchOptions {
                model: &["--model", "{}"],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::Options(&[&["--force"]]),
            },
            Self::OpenCode => LaunchOptions {
                model: &[],
                effort: &[],
                efforts: &[],
                full_access: FullAccess::Options(&[&["--auto"]]),
            },
        }
    }

    /// How the first interactive prompt is given at the start, `{}`
    /// replaced by the prompt, which the native CLI holds through its own
    /// start screens. The prompt is always one word, after a word that ends
    /// the options or joined to its option, so a prompt that starts with `-`
    /// is never read as an option. OpenCode's positional argument is a
    /// project, and OpenCode 2.0.29 reads `--prompt -x` as a missing value
    /// (it prints its help and exits 0, measured 2026-11-02), so it takes
    /// `--prompt=<prompt>`; agy takes the prompt of an interactive session
    /// only through `-i` (`--prompt-interactive`), a bare argument being
    /// refused.
    pub const fn prompt_form(self) -> &'static [&'static str] {
        match self {
            Self::OpenCode => &["--prompt={}"],
            Self::Antigravity => &["-i", "{}"],
            _ => &["--", "{}"],
        }
    }

    /// The words a start adds to carry `prompt`, in [`Self::prompt_form`].
    pub fn prompt_arguments(self, prompt: &str) -> Vec<String> {
        self.prompt_form()
            .iter()
            .map(|part| part.replace("{}", prompt))
            .collect()
    }

    /// Whether native arguments already give the start a prompt: they hold
    /// the end of options, or one of the prompt's own options under either
    /// of its names (agy's `-i` is `--prompt-interactive`), alone or joined.
    pub fn carries_prompt(self, args: &[String]) -> bool {
        let options: &[&str] = match self {
            Self::OpenCode => &["--prompt"],
            Self::Antigravity => &["-i", "--prompt-interactive"],
            _ => &[],
        };
        args.iter().any(|arg| {
            arg == "--"
                || options.iter().any(|option| {
                    arg == option
                        || arg
                            .strip_prefix(option)
                            .is_some_and(|rest| rest.starts_with('='))
                })
        })
    }

    /// Verified launch dialects that accept repeated extra permission roots.
    /// A checkout cwd is independent of this optional CLI argument.
    pub const fn accepts_extra_directories(self) -> bool {
        matches!(self, Self::Claude | Self::Codex | Self::Omp | Self::Cursor)
    }

    pub const fn closes_pane_when_sleeping(self) -> bool {
        !matches!(self, Self::Claude | Self::Codex | Self::Antigravity)
    }

    /// The key, as Herdr names it, that makes the idle agent quit on its
    /// own, as the operator quits it: a sleep in the agent's own pane presses
    /// it first, so the agent puts back the terminal modes it set (bracketed
    /// paste, its keyboard protocol, the alternate screen, mouse reports)
    /// before the shell takes the terminal again, and ends it only when it
    /// does not quit. No signal does that: Codex CLI 0.165.0 ended by
    /// SIGHUP, SIGINT or SIGTERM leaves every mode on (measured 2026-10-29),
    /// and a Windows prompt reads a bracketed paste left on as keys
    /// (herdrdev/herdr#5371). A press clears a draft first, and Ctrl+D is
    /// never pressed, because a shell that already holds the terminal ends on
    /// it. None for an agent that closes its pane when sleeping.
    pub const fn quit_key(self) -> Option<&'static str> {
        match self {
            Self::Claude | Self::Codex | Self::Antigravity => Some("ctrl+c"),
            Self::Grok | Self::OpenCode | Self::Pi | Self::Omp | Self::Cursor => None,
        }
    }

    /// What the agent shows while it waits for its quit key a second time,
    /// the only sign that a press did not start its quit. Claude Code
    /// 2.1.312 shows "Press Ctrl-C again to exit" for 800 ms and agy 1.4.9
    /// "press ctrl+c again to exit" (measured 2026-10-29). Codex CLI 0.165.0
    /// quits on the first press into an empty composer and asks for none;
    /// "again to quit" is the hint it showed while it did ask. A second
    /// press is never sent without it: one that reaches an agent already
    /// quitting is left for its shell to read, in the keyboard protocol the
    /// agent turned on (with-hide/hide-private#37).
    pub const fn quit_again(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("Press Ctrl-C again to exit"),
            Self::Codex => Some("again to quit"),
            Self::Antigravity => Some("press ctrl+c again to exit"),
            Self::Grok | Self::OpenCode | Self::Pi | Self::Omp | Self::Cursor => None,
        }
    }

    /// Whether the agent asks before it works in a folder it has not been
    /// told to trust, so Hide records that trust for a worktree it made
    /// before it starts the agent there: the question would otherwise hold a
    /// delegated child that nobody is watching (PRD antigravity-first-class
    /// D-08; Claude Code 2.1.302 and Codex 0.163.0 ask the same, measured
    /// 2026-10-17, Grok 1.0.67, measured 2026-10-23, and Pi 1.0.4 in a folder
    /// with project files it loads, measured 2026-11-02; docs/agent-hooks.md,
    /// Folder trust for the worktrees Hide makes).
    pub const fn trusts_worktrees(self) -> bool {
        matches!(
            self,
            Self::Claude | Self::Codex | Self::Antigravity | Self::Grok | Self::Pi
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
    /// It is the first word of the agent's definition's `[resume] session`,
    /// whose second is the session; retired Cursor keeps its own.
    pub const fn resume_flag(self) -> &'static str {
        use definition::builtin as defs;
        let session = match self {
            Self::Claude => defs::CLAUDE_CODE.resume_session,
            Self::Codex => defs::CODEX.resume_session,
            Self::Grok => defs::GROK.resume_session,
            Self::OpenCode => defs::OPENCODE.resume_session,
            Self::Pi => defs::PI.resume_session,
            Self::Omp => defs::OMP.resume_session,
            Self::Antigravity => defs::ANTIGRAVITY.resume_session,
            Self::Cursor => return "--resume",
        };
        match session {
            Some([flag, _]) => flag,
            _ => panic!("a resume dialect's definition resumes with `<flag> {{session}}`"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchOptions {
    pub model: &'static [&'static str],
    pub effort: &'static [&'static str],
    pub efforts: &'static [&'static str],
    pub full_access: FullAccess,
}

/// How a CLI is started without asking before each tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FullAccess {
    /// The options that turn its questions off, each as its words with the
    /// option's name first: the words its `--help` names for that (Codex's
    /// two `-c` overrides are one option), then any option the CLI needs so
    /// that it does not stop to confirm them.
    Options(&'static [&'static [&'static str]]),
    /// The CLI asks no permission question, so there is nothing to turn off.
    NotAsked,
}

impl FullAccess {
    /// Each option a full-access start adds, as its words; none for `NotAsked`.
    pub const fn options(self) -> &'static [&'static [&'static str]] {
        match self {
            Self::Options(options) => options,
            Self::NotAsked => &[],
        }
    }

    /// The words a full-access start adds, in order.
    pub fn arguments(self) -> Vec<&'static str> {
        self.options()
            .iter()
            .flat_map(|option| option.iter().copied())
            .collect()
    }

    /// Each option's name, which the CLI's `--help` lists.
    pub fn names(self) -> Vec<&'static str> {
        self.options()
            .iter()
            .filter_map(|option| option.first().copied())
            .collect()
    }
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
}

impl HerdrIntegration {
    /// The source the agent's own Herdr integration reports its sessions
    /// under, named by the integration target (`herdr:antigravity_cli` for agy).
    pub fn session_source(&self) -> String {
        format!("herdr:{}", self.target.replace('-', "_"))
    }
}

/// The agent as a Hide AI provider (`hide-ai`, registry).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AiProvider {
    /// The provider id Hide AI's settings persist, kept apart from the
    /// adapter id because files an earlier Hide wrote carry it (`claude`).
    pub id: &'static str,
    /// Whether Hide AI may send the agent a request: only once a call to it
    /// is proven read-only (`docs/AI_PROVIDERS.md`, D-15). An agent without
    /// that proof is still listed, and refuses every request with why.
    pub requests: Capability<()>,
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

/// One way the vendor's documentation installs the agent's CLI, how Hide
/// tells a copy installed that way from where its program is, and the one
/// command that updates that copy (docs/agent-hooks.md, Updating an agent
/// that is too old). A copy no route recognises gets the agent's install
/// page instead of a guessed command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpdateRoute {
    pub method: InstallMethod,
    /// The systems the command is documented for.
    pub systems: Systems,
    pub command: &'static str,
    /// The vendor's page the command is from, and, for a package manager's
    /// own upgrade of a package the vendor names, that manager's page too.
    pub source: &'static str,
}

/// How a copy of a CLI got onto the machine, as its program's path shows it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallMethod {
    /// npm's global folder, holding this package: nvm's, fnm's and
    /// Homebrew's Node, any npm prefix, Volta, and `%AppData%\npm`.
    Npm(&'static str),
    /// bun's global folder, holding this package.
    Bun(&'static str),
    /// A Homebrew formula or cask of this name: the program resolves into
    /// `Cellar/<name>` or `Caskroom/<name>`.
    Homebrew(&'static str),
    /// A WinGet package of this id: the program resolves into
    /// `WinGet/Packages/<id>_<source>`.
    WinGet(&'static str),
    /// The vendor's own installer: the program is, or resolves into, one of
    /// these paths under the home folder, written with `/`.
    Installer(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Systems {
    Every,
    MacOsAndLinux,
    Windows,
}

impl Systems {
    /// Whether the system this code was built for is one of them.
    pub const fn include_this_one(self) -> bool {
        match self {
            Self::Every => true,
            Self::MacOsAndLinux => !cfg!(windows),
            Self::Windows => cfg!(windows),
        }
    }
}

/// One way Hide hands an agent a brief or a letter (`docs/delivery.md`, The
/// input ladder). An agent's ladder ([`AgentAdapter::input`]) names the rungs
/// it has in the order Hide tries them; the last is always [`Self::Typed`],
/// today's typing, so an agent without a channel of its own keeps today's
/// behavior. The Hide extension kind (a script of Hide's in the agent's own
/// extension folder that takes input, measured for Pi and omp) is not
/// declared until an agent uses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputRung {
    /// The agent's own way in.
    AgentApi(AgentApi),
    /// Typed into the agent's screen through Herdr's `agent.prompt`; the
    /// agent's own record (its prompt hook or session file) confirms it.
    Typed,
}

/// The agent's own ways in that Hide uses, each measured on the real CLI
/// (`docs/delivery.md`, The input ladder).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentApi {
    /// A spawn's brief as the start's prompt argument
    /// ([`LaunchDialect::prompt_form`]), which the CLI holds through its own
    /// start screens and submits after them: Codex 0.166.0 through "Hooks
    /// need review", Claude Code 2.1.316 and agy 1.4.12 through their folder
    /// trust question (measured 2026-11-01), Grok 1.0.70 through its folder
    /// trust question and omp 18.7.0 through its first-run setup and update
    /// notice, and OpenCode 2.0.29, which shows no start screen (measured
    /// 2026-11-02). Pi 1.0.4 holds it too, but reads a prompt that starts
    /// with `@` as a file to attach and exits when there is none, so it does
    /// not declare it. Only a fresh start's first input takes it.
    LaunchPrompt,
}

impl InputRung {
    /// The rung's name in the journey contract and a log line.
    pub const fn name(self) -> &'static str {
        match self {
            Self::AgentApi(AgentApi::LaunchPrompt) => "launch_prompt",
            Self::Typed => "typed",
        }
    }
}

/// The ladder of an agent with no way in of its own.
pub const TYPED_ONLY: &[InputRung] = &[InputRung::Typed];

impl AgentAdapter {
    /// Whether a fresh start of the agent carries its brief as the start's
    /// prompt argument ([`AgentApi::LaunchPrompt`]).
    pub fn launch_prompt(&self) -> bool {
        self.input
            .contains(&InputRung::AgentApi(AgentApi::LaunchPrompt))
    }
}

/// Independent gates; adding prompt intake never grants bell or shell denial.
#[derive(Clone, Copy, Debug)]
pub struct AgentAdapter {
    pub key: AgentId,
    /// The agent's identity is its built-in definition file's
    /// (`agents/<id>.toml`, [`definition::Builtin`]): `id`, `label` (the
    /// file's `name`), `aliases`, `executables` and `folder` (its `home`)
    /// are that file's values, never written here.
    pub id: &'static str,
    pub label: &'static str,
    pub picker_label: &'static str,
    pub sidebar_label: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub executables: &'static [&'static str],
    /// The agent's own folder under the account's home, as its parts.
    pub folder: &'static [&'static str],
    /// Every supported agent declares one; only a retired agent, for which
    /// Hide installs nothing, has none.
    pub minimum: Option<MinimumVersion>,
    pub skill_location: SkillLocation,
    pub hook: HookInstall,
    pub guidance: Capability<ContextRoute>,
    pub herdr: HerdrIntegration,
    /// The variable that moves the agent's own folder away from
    /// [`AgentAdapter::folder`] when the operator runs it (Codex's
    /// `CODEX_HOME`), read only through [`AgentAdapter::home`]; `None` for an
    /// agent whose folder Hide reads at its default place alone.
    pub home_variable: Option<&'static str>,
    /// How Hide AI runs the agent for background features, or why it does
    /// not: `hide-ai`'s registry lists exactly the agents that declare one.
    pub ai: Capability<AiProvider>,
    pub default_on: bool,
    pub prompt_hook: Option<PromptHook>,
    /// The dialect in which the agent's `PreToolUse` hook refuses a call:
    /// the guardrails, the plan record, the global-instruction guard and the
    /// opener guard answer in it; `None` for an agent whose hook refuses
    /// nothing.
    pub guard: Option<HookDialect>,
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
    pub thread: Capability<ThreadReading>,
    /// Where the agent draws its composer and what an empty one shows, so
    /// a letter held on a possible draft can be released when the screen
    /// says the composer holds nothing (`docs/delivery.md`, Reading the
    /// composer).
    pub composer: Capability<composer::Composer>,
    /// How Hide hands the agent a brief or a letter, rung by rung
    /// ([`InputRung`]); every agent declares one, ending with `Typed`.
    pub input: &'static [InputRung],
    pub start: Option<LaunchDialect>,
    /// Safety declaration for keyless journeys only; never changes a real login HOME.
    pub fake_home: journey::FakeHome,
    pub find: Option<FindDialect>,
    pub usage: usage::UsageDeclaration,
    pub doc_url: &'static str,
    pub install_url: &'static str,
    /// Each documented way of installing the CLI that Hide can tell from
    /// the program's path, with its update command; empty for a retired
    /// agent.
    pub updates: &'static [UpdateRoute],
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
    Sleep,
    Fork,
    Start,
    Titles,
}

impl Feature {
    pub const ALL: [Self; 9] = [
        Self::Skill,
        Self::Guidance,
        Self::Letters,
        Self::Bell,
        Self::Memory,
        Self::Sleep,
        Self::Fork,
        Self::Start,
        Self::Titles,
    ];
}

impl AgentAdapter {
    /// The reasoning efforts the agent takes: the one menu a start, a
    /// restart and Hide AI offer ([`LaunchDialect::options`]); empty for an
    /// agent Hide does not start.
    pub const fn efforts(&self) -> &'static [&'static str] {
        match self.start {
            Some(dialect) => dialect.options().efforts,
            None => &[],
        }
    }

    pub const fn supports(&self, feature: Feature) -> bool {
        match feature {
            Feature::Skill => true,
            Feature::Guidance => self.guidance.is_some(),
            Feature::Letters => matches!(self.prompt_hook, Some(PromptHook { context: true, .. })),
            Feature::Bell => self.bell,
            Feature::Memory => self.memory.is_some(),
            Feature::Sleep => self.sleep.is_some(),
            Feature::Fork => self.fork.is_some(),
            Feature::Start => self.start.is_some(),
            // A label reader alone is not the session features offered by
            // Settings: titles count once the conversation is read.
            Feature::Titles => self.conversation.is_some(),
        }
    }

    pub const fn basic(&self) -> bool {
        !matches!(self.prompt_hook, Some(PromptHook { context: true, .. })) || self.guard.is_none()
    }
}
