//! Which agent runtimes Hide instruments, and the vocabulary it writes.

use std::path::PathBuf;

use hide_agent_adapter::AccountPath;

use crate::account::AccountAt;

use serde::{Deserialize, Serialize};

/// The marker Hide stamps into every hook entry it installs.
///
/// It is carried as the `--source` argument of the helper invocation, so one
/// substring in the command line answers both questions the diagnosis asks:
/// whether Hide owns this entry, and which version of the entry it is (PRD
/// D-31, D-57). Nothing outside this crate may write it.
pub const HOOK_SOURCE_NAME: &str = "hide-subagents";

/// The hook helper's file name on this system, as the bundle ships it
/// (`.exe` on Windows). The kit looks for it in its kit folder and hands the
/// path to [`crate::install`], so the name is stated once here rather than in
/// the app and the build script both.
pub const HELPER_BINARY_NAME: &str = if cfg!(windows) {
    "hide-agent-hooks.exe"
} else {
    "hide-agent-hooks"
};

/// The pane's `hide_hooks` reporting protocol. Raise it only when older
/// reports cannot be interpreted: changing an installed command must not
/// disconnect an already-running pane whose reports are still valid.
/// Version 8's reports still read the same when they began to reach the core through
/// hided instead of through Herdr, so moving the path kept it.
pub const HOOK_VERSION: u32 = 8;

/// The installed Claude Code and Codex command version, independent of the
/// pane reporting protocol. Version 10 adds passive Post observation slots;
/// version 11 adds the permission and interrupt events (PRD engine-bundle-1
/// D-10); version 12's Windows guard tests for the helper without a cmdlet
/// (with-hide/hide-private#318); version 13 adds Claude Code's `StopFailure`
/// and its usage-limit continuation notice (mission-56).
pub const HOOK_ENTRY_VERSION: u32 = 13;

/// The full intent table, the first section of `hide guide` and the body of
/// the `hide` skill (`hide_kit::agents::GUIDE_SKILL`), as a literal so both
/// carry the very same text. A session start carries the short form in
/// [`START_CONTEXT`] and points at the guide for the rest.
#[macro_export]
macro_rules! intent_table {
    () => {
        "Inside Hide, turn what the operator asks for into its command:\n| The operator asks | Run |\n| --- | --- |\n| show the operator a file, a diff or a web page | `hide file open <path>`, `hide diff open <path>`, `hide browser open <url-or-path>`: it opens beside your terminal when the operator watches your pane, else waits in your pane's Views list, and stays in the operator's Needs You as To look at until they look at it |\n| you made something for the operator to look at (an HTML page, an image, a PDF, a diagram, a long report), asked for or not | open it the same way as soon as it is ready, without asking whether to, and say in one line what you opened: from a pane the operator is not watching it only waits in your Views list, so it interrupts nothing |\n| check a page for yourself (your own browser QA) | your own browser tool, such as `aside`, not `hide browser open`: a page you open in Hide waits for the operator |\n| collect what a delegated child opened | `hide view list` shows what the operator has not seen and who opened it; read it with `hide view status <view-id>` or `hide browser snapshot <display>`, and to show it, open it again from your own pane |\n| start an agent beside you | `hide agent spawn --beside …` (your own checkout only; it is the operator's to watch, so it takes no `--parent`) |\n| hand work to an agent and collect its result | `hide agent spawn --parent here …` |\n| complex work: planning or review first, multiple PRs or parallel work, another agent reviewing, or work likely to fill the conversation | first ask in one line: Shall I run this as a mission, or do it directly? Follow the operator's answer; if they already requested a mission or invoked `/hide-mission`, run `hide mission help` and follow it without asking again. With none of these signals, work directly without asking |\n| Hide itself misbehaves: a crash, a command that fails or answers wrongly for no reason of yours, a pane or screen that breaks | `hide issue help`, which the `hide-issue` skill points at: collect the exact error, versions and steps into the bug template, show the operator, and file it in with-hide/hide after their yes |\n| start independent work the operator will handle | `hide agent spawn …` without --parent |\n| repeat a task at set times with no one present (every weekday morning, every few hours), or do one at a later time (check it in N days, check this again next Tuesday) | `hide automation add --name <name> --task <text> --schedule <when> --kind <kind>`, with `--schedule \"once in 7d 09:00\"` for one later time; run `hide automation help` first for the schedule forms, then tell the operator what it reads as and when it runs next. Not a cron job, an OS scheduler or a script of your own. A wait or a check shorter than five minutes, or one that ends within this conversation, you keep doing yourself as now |\nBefore any `hide agent spawn`, read `candidates` in `hide agent catalog`, the operator's list of agents and models to hand work to, and take the row whose purpose fits the work unless the operator named one; it is a reference, not a limit. Without --beside, `hide agent spawn` opens the agent in a tab of its own. Never start an agent with `hide pane open` or by typing its program into a pane: that agent has no parent, letters or watch. Never show the operator a result with the system opener (`open`, `xdg-open`, `start`, `Start-Process`, `Invoke-Item`): it lands in an app outside Hide with no trace of who opened it, and Hide refuses the call. Use `hide browser open` for a page or a URL and `hide file open` for anything else, for a file outside the checkout too."
    };
}

/// What a session start gives every agent inside Hide besides the core's
/// answer: where to read more when it needs it, and the few requests the
/// operator makes most, each with its command. It is short on purpose:
/// Claude Code saves a session-start context of about 10,000 characters or
/// more to a file and gives the model only its first 2,000, so every rule
/// the agent needs only sometimes lives in `hide guide <topic>`, and the
/// operator's global instruction and the Project Memory index come before
/// this ([`session_start_context`]).
pub const START_CONTEXT: &str = "Inside Hide, read more when you need it:\n\
- `hide guide <topic>` prints one part of the guide to working in Hide, such as `agents`, `letters`, `panes`, `operators-screen`, `automations` or `devices`; `hide guide` lists every topic.\n\
- `hide workspace info` shows this checkout and the commands this pane may run.\n\
- `hide session list` lists this Project's sessions, and `hide session record <id>` reads one.\n\
- `hide memory show <id>` reads a Project Memory card in full, and `hide memory search <words>` finds others.\n\
| The operator asks | Run |\n\
| --- | --- |\n\
| show a file, a diff or a page, or you made one for them to look at | `hide file open <path>`, `hide diff open <path>` or `hide browser open <url-or-path>`, as soon as it is ready and without asking, then say in one line what you opened; never the system opener (`open`, `xdg-open`, `start`) |\n\
| an agent beside you | `hide agent spawn --beside …`; read `hide guide agents` before your first spawn |\n\
| hand work to an agent and collect its result | `hide agent spawn --parent here …` |\n\
| tell another agent something | `hide request send <agent> …`, never by typing into its pane |\n\
| complex work: planning or review first, several PRs or parallel work, another agent reviewing | first ask in one line: Shall I run this as a mission, or do it directly? A mission is `hide mission help` |\n\
| Hide itself misbehaves | `hide issue help` |";

/// The characters a session start's whole context keeps at most, the one
/// string the hook prints with everything in it: under the 10,000 or so
/// past which Claude Code shows the model only the first 2,000 (the 1.2.13
/// release candidate's start was saved to a file at 9.9 K).
pub const START_CONTEXT_LIMIT_CHARS: usize = 9_000;

/// What stands in for the Project's open sessions when the start has no room
/// for them.
pub const SESSIONS_LEFT_OUT: &str =
    "The Project's open sessions are left out to keep this short: `hide session list` lists them.";

/// What stands in for the Workspace note when the start has no room for it,
/// a long checkout path being the usual cause.
pub const WORKSPACE_LEFT_OUT: &str = "Hide Workspace: `hide workspace info` names this session's checkout and the commands this pane may run.";

/// What stands in for [`START_CONTEXT`] when the operator's global
/// instruction and the Memory index leave no room for it either.
pub const START_LEFT_OUT: &str = "Inside Hide, `hide guide` prints how to work here.";

/// The few lines a session in a Hide pane needs about its Workspace: which
/// checkout the daemon bound it to, that its commands act only there, and
/// where the commands this pane may run are listed. A bare `hide` reaches
/// that checkout from the pane (it bootstraps its own one-shot credential),
/// so the note gives no command prefix, and the command list and the rules
/// for letters and Views live in `hide workspace info` and `hide guide`.
pub fn workspace_note(checkout_path: &str) -> String {
    format!(
        "Hide Workspace: this session's checkout is `{checkout_path}`, and Workspace commands act only on it. `hide workspace info` lists the commands this pane may run."
    )
}

/// A session start's whole context, the one string the hook prints: first
/// the parts whose length is fixed or capped, the operator's global
/// instruction and the Project Memory index (each only when the core
/// answered with it) and [`START_CONTEXT`], then the parts whose length
/// follows their input, the Workspace note of a live pane
/// ([`workspace_note`]) and the Project's open sessions. Every agent with a
/// session-start route gets this text.
///
/// Past [`START_CONTEXT_LIMIT_CHARS`] the parts give way from the back: the
/// open sessions become [`SESSIONS_LEFT_OUT`], then the Workspace note
/// [`WORKSPACE_LEFT_OUT`], then the start text [`START_LEFT_OUT`]. The
/// global instruction and the Memory index are always given whole; their
/// own caps keep the two under the limit (`hide-core`'s
/// `the_worst_session_start_still_fits_what_claude_code_shows` holds that
/// with every part at its cap and the longest checkout path a request
/// takes).
pub fn session_start_context(
    global: Option<&str>,
    memory: Option<&str>,
    sessions: Option<&str>,
    workspace: Option<&str>,
) -> String {
    let sessions = sessions.filter(|sessions| !sessions.is_empty());
    let workspace = workspace.filter(|workspace| !workspace.is_empty());
    let sessions_left_out = sessions.map(|_| SESSIONS_LEFT_OUT);
    let workspace_left_out = workspace.map(|_| WORKSPACE_LEFT_OUT);
    let compose = |start: &str, workspace: Option<&str>, sessions: Option<&str>| {
        joined([global, memory, Some(start), workspace, sessions])
    };
    [
        (START_CONTEXT, workspace, sessions),
        (START_CONTEXT, workspace, sessions_left_out),
        (START_CONTEXT, workspace_left_out, sessions_left_out),
    ]
    .into_iter()
    .map(|(start, workspace, sessions)| compose(start, workspace, sessions))
    .find(|context| within_limit(context))
    .unwrap_or_else(|| compose(START_LEFT_OUT, workspace_left_out, sessions_left_out))
}

/// The open sessions as the core gave them while they fit beside the
/// global instruction, the Memory index and [`START_CONTEXT`] under
/// [`START_CONTEXT_LIMIT_CHARS`], else [`SESSIONS_LEFT_OUT`]; none when the
/// core gave none. OpenCode, Pi and omp, whose script puts the start text
/// and the Workspace note in front of the first prompt's context, keep
/// their sessions by this rule; those hosts show the model the whole
/// context, so the note's length is not counted there.
pub fn open_sessions_within_limit<'a>(
    global: Option<&str>,
    memory: Option<&str>,
    sessions: Option<&'a str>,
) -> Option<&'a str> {
    let sessions = sessions.filter(|sessions| !sessions.is_empty())?;
    let whole = joined([global, memory, Some(START_CONTEXT), Some(sessions)]);
    Some(if within_limit(&whole) {
        sessions
    } else {
        SESSIONS_LEFT_OUT
    })
}

fn within_limit(context: &str) -> bool {
    context.chars().count() <= START_CONTEXT_LIMIT_CHARS
}

/// `parts` one after another, a blank line between two, leaving out a
/// missing or empty one.
fn joined<const N: usize>(parts: [Option<&str>; N]) -> String {
    let mut context = String::new();
    for part in parts.into_iter().flatten().filter(|part| !part.is_empty()) {
        if !context.is_empty() {
            context.push_str(if context.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            });
        }
        context.push_str(part);
    }
    context
}

pub fn hook_stdout(runtime: AgentRuntime, event: HookEvent) -> Option<String> {
    hook_stdout_with_context(runtime, event, None)
}

/// The runtime's envelope for `event` with `context`: at `SessionStart` the
/// whole start context, as [`session_start_context`] composed it, and
/// [`START_CONTEXT`] alone when the core gave nothing; at a prompt, the
/// context itself.
pub fn hook_stdout_with_context(
    runtime: AgentRuntime,
    event: HookEvent,
    memory_context: Option<&str>,
) -> Option<String> {
    let additional_context = match event {
        HookEvent::SessionStart => match memory_context {
            Some(context) if !context.is_empty() => context.to_owned(),
            _ => START_CONTEXT.to_owned(),
        },
        HookEvent::UserPromptSubmit => memory_context.filter(|value| !value.is_empty())?.to_owned(),
        HookEvent::SubagentStart
        | HookEvent::SubagentStop
        | HookEvent::Stop
        | HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure
        | HookEvent::PermissionRequest
        | HookEvent::Notification
        | HookEvent::Interrupt
        | HookEvent::StopFailure => return None,
    };
    let output = match runtime {
        AgentRuntime::ClaudeCode | AgentRuntime::Codex => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": event.name(),
                "additionalContext": additional_context,
            }
        }),
    };
    serde_json::to_string(&output).ok()
}

/// Another agent that runs Claude Code's hooks from `~/.claude/settings.json`
/// beside its own, so Hide's `claude-code` hook starts inside a session that
/// is not a Claude Code session.
///
/// Each is found by a variable its own documentation or source says it sets
/// for the processes it starts; nothing else decides it, since a wrong guess
/// either silences a real Claude Code or lets a stranger take its letters:
///
/// - Cursor sets `CURSOR_VERSION` for every hook it runs; its page on the
///   Claude Code hooks it loads does not say whether those get it too (the
///   same runner and the `CLAUDE_PROJECT_DIR` alias it documents suggest they
///   do).
/// - OpenCode sets `OPENCODE` and `OPENCODE_PID` in every process it starts,
///   which is how a bridge plugin runs the hooks (opencode `src/flag`).
/// - Grok sets `GROK_HOOK_EVENT` and `GROK_SESSION_ID` for every hook it runs
///   (docs.x.ai, Hooks).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignOrigin {
    Cursor,
    OpenCode,
    Grok,
}

impl ForeignOrigin {
    pub fn detect<V>(variable: impl Fn(&str) -> Option<V>) -> Option<Self> {
        let set = |names: &[&str]| names.iter().any(|name| variable(name).is_some());
        if set(&["CURSOR_VERSION"]) {
            Some(Self::Cursor)
        } else if set(&["OPENCODE", "OPENCODE_PID"]) {
            Some(Self::OpenCode)
        } else if set(&["GROK_HOOK_EVENT", "GROK_SESSION_ID"]) {
            Some(Self::Grok)
        } else {
            None
        }
    }
}

/// Whether Hide's own hook for the agent running this one speaks in its
/// place, so Claude Code's hook says nothing at all: Cursor and Grok have
/// hooks of their own (PRD grok-cursor-hooks D-05), and OpenCode Hide's
/// plugin, so a count or a refusal from both would happen twice when they run
/// Claude Code's hooks too (an OpenCode bridge plugin does).
pub fn silences_claude_hook<V>(variable: impl Fn(&str) -> Option<V>) -> bool {
    ["CURSOR_VERSION", "OPENCODE", "OPENCODE_PID"]
        .iter()
        .any(|name| variable(name).is_some())
        || inside_grok(variable)
}

/// Whether Grok runs this hook. Grok sets `GROK_HOOK_EVENT` only for the
/// hooks it runs, its own and the Claude Code and Cursor hooks it loads beside
/// them; `GROK_SESSION_ID` can also reach a program a Grok session started,
/// such as a Claude Code launched from its shell, whose own hook must still
/// count, so it does not decide this. (That Claude Code's opener guard stays
/// out as in any foreign session, `ForeignOrigin`.)
pub fn inside_grok<V>(variable: impl Fn(&str) -> Option<V>) -> bool {
    variable("GROK_HOOK_EVENT").is_some()
}

/// Whether Claude Code's hook may take and confirm letters here (D-25). A
/// letter is addressed to a pane's own session and is confirmed once that
/// session has seen it, so a hook that runs inside another agent's session
/// would take the letter and confirm it to nobody who reads it.
pub fn takes_letters<V>(variable: impl Fn(&str) -> Option<V>) -> bool {
    ForeignOrigin::detect(variable).is_none()
}

/// `json` with every character outside ASCII written as a `\u` escape,
/// which every JSON reader decodes to the same value.
///
/// On Windows both runtimes run the hook through PowerShell, which can read
/// a program's output in the console code page and write it out again
/// (`docs/agent-hooks.md`, Installing); ASCII is the one
/// encoding that survives that unchanged, and the context carries `…` and
/// Memory text in any language.
pub fn ascii_json(json: &str) -> String {
    let mut ascii = String::with_capacity(json.len());
    for character in json.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                ascii.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    ascii
}

/// Add a live pane's Workspace instructions to the existing SessionStart
/// envelope without changing the Memory receipt embedded in that context.
pub fn append_session_context(output: &str, context: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(output).ok()?;
    let existing = value["hookSpecificOutput"]["additionalContext"].as_str()?;
    let combined = format!("{existing}\n\n{context}");
    value["hookSpecificOutput"]["additionalContext"] = combined.into();
    serde_json::to_string(&value).ok()
}

/// The token that says this pane's session is running Hide's hook at all.
///
/// It is written once at `SessionStart` and carries the hook version, so a
/// pane with no children still proves it is instrumented. Without it, "this
/// agent is working alone" and "Hide cannot see this agent's children" would
/// be the same empty screen (PRD B21, B23, D-61).
pub const INSTRUMENTED_TOKEN: &str = "hide_hooks";

/// The count of subagents this pane's session has running right now.
pub const WORKING_TOKEN: &str = "hide_sub_working";

/// The count of subagents this pane's session has finished.
pub const DONE_TOKEN: &str = "hide_sub_done";

/// The count of subagents this pane's session reports as blocked.
///
/// No shipped adapter observes it, so it is never written today. The reader
/// treats an absent count as unknown and draws the uninstrumented mark rather
/// than a zero (PRD B32, D-53).
pub const BLOCKED_TOKEN: &str = "hide_sub_blocked";

/// `hide-subagents@N`, the exact value the helper writes into its command.
pub fn hook_source_id() -> String {
    format!("{HOOK_SOURCE_NAME}@{HOOK_ENTRY_VERSION}")
}

/// Reads the version out of a `hide-subagents@N` source id.
///
/// Returns `None` for anything that is not Hide's marker, which is how a
/// third party's entry is left alone.
pub fn parse_source_version(value: &str) -> Option<u32> {
    parse_marker(HOOK_SOURCE_NAME, value)
}

/// Finds Hide's marker anywhere in a hook entry's command line.
///
/// The command is the only field both runtimes agree on, so ownership is read
/// from it rather than from a key of Hide's own invention that a runtime's
/// settings validator might reject.
pub fn marker_version_in(command: &str) -> Option<u32> {
    marker_version_of(HOOK_SOURCE_NAME, command)
}

/// [`marker_version_in`] for any marker name: the guidance hooks of the
/// agents beyond Claude Code and Codex carry their own (`crate::guidance`).
pub fn marker_version_of(name: &str, command: &str) -> Option<u32> {
    let mut cursor = command;
    while let Some(index) = cursor.find(name) {
        let candidate = &cursor[index..];
        let end = candidate
            .find(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-' && character != '@'
            })
            .unwrap_or(candidate.len());
        if let Some(version) = parse_marker(name, &candidate[..end]) {
            return Some(version);
        }
        cursor = &cursor[index + name.len()..];
    }
    None
}

fn parse_marker(name: &str, value: &str) -> Option<u32> {
    let digits = value.strip_prefix(name)?.strip_prefix('@')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// An agent runtime whose global hook configuration Hide can instrument.
///
/// Adding a runtime is adding a variant here and the three answers below; no
/// other file in the product learns a new configuration format (PRD D-47).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentRuntime {
    ClaudeCode,
    Codex,
}

impl AgentRuntime {
    pub const ALL: [AgentRuntime; 2] = [Self::ClaudeCode, Self::Codex];

    /// The events each runtime declares: Codex reports nonzero outcomes in
    /// PostToolUse and has no PostToolUseFailure, Notification or
    /// StopFailure, and only Codex declares Interrupt, so neither is given an
    /// event it would not send.
    pub fn events(self) -> impl Iterator<Item = HookEvent> {
        HookEvent::ALL.into_iter().filter(move |event| match self {
            Self::ClaudeCode => *event != HookEvent::Interrupt,
            Self::Codex => !matches!(
                event,
                HookEvent::PostToolUseFailure | HookEvent::Notification | HookEvent::StopFailure
            ),
        })
    }

    /// The stable identifier used in state, diagnostics and the wire.
    pub fn id(self) -> &'static str {
        self.dialect().adapter().id
    }

    /// Reads an id back. The wire carries the id, so this is the one place
    /// that turns it into a runtime rather than each caller matching strings.
    pub fn from_id(id: &str) -> Option<Self> {
        match hide_agent_adapter::adapter(id)?.hook {
            hide_agent_adapter::HookInstall::Runtime(dialect) => Self::from_dialect(dialect),
            _ => None,
        }
    }

    /// The name the operator sees.
    pub fn label(self) -> &'static str {
        self.dialect().adapter().label
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::from_id(value)
    }

    pub const fn dialect(self) -> hide_agent_adapter::HookDialect {
        match self {
            Self::ClaudeCode => hide_agent_adapter::HookDialect::ClaudeCode,
            Self::Codex => hide_agent_adapter::HookDialect::Codex,
        }
    }

    /// The settings-file runtime that speaks `dialect`. OpenCode's, Pi's and
    /// omp's dialects are spoken by a script of Hide's in the agent's own
    /// folder (`crate::plugin`), and Grok's, Cursor's and agy's hooks are
    /// written by `crate::guidance`; none has a settings file.
    pub const fn from_dialect(dialect: hide_agent_adapter::HookDialect) -> Option<Self> {
        match dialect {
            hide_agent_adapter::HookDialect::ClaudeCode => Some(Self::ClaudeCode),
            hide_agent_adapter::HookDialect::Codex => Some(Self::Codex),
            hide_agent_adapter::HookDialect::OpenCode
            | hide_agent_adapter::HookDialect::Grok
            | hide_agent_adapter::HookDialect::Cursor
            | hide_agent_adapter::HookDialect::Pi
            | hide_agent_adapter::HookDialect::Omp
            | hide_agent_adapter::HookDialect::Antigravity => None,
        }
    }

    /// The provider id Project Memory keys this runtime's sessions by.
    pub fn memory_id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    /// The directory whose presence means this runtime is set up in the
    /// account: its own folder (`AccountPath::State`). For a home, that is
    /// the default account's for a node that honours no variable, whatever
    /// `CODEX_HOME` the launching shell carries.
    ///
    /// Hide installs nothing for a runtime the operator does not use: writing
    /// `~/.codex/hooks.json` on a machine with no Codex would create a file
    /// for a tool that is not there.
    pub fn home_directory<'a>(self, at: impl Into<AccountAt<'a>>) -> PathBuf {
        at.into()
            .place(self.dialect().adapter(), AccountPath::State)
    }

    /// The file whose `hooks` object Hide appends to.
    pub fn config_path<'a>(self, at: impl Into<AccountAt<'a>>) -> PathBuf {
        at.into()
            .place(self.dialect().adapter(), AccountPath::Hooks)
    }

    /// The file that keeps this runtime's folder trust, found from the same
    /// folder as [`Self::config_path`], so the trust Hide records and its
    /// hooks are always read from one place (`crate::folder_trust`).
    ///
    /// Claude Code's is its global config (`AccountPath::FolderTrust`), or
    /// the older `.config.json` in its folder while that file has content,
    /// which Claude Code then reads instead (its `getGlobalClaudeFile`);
    /// Codex's is the `config.toml` in its folder, which only Codex writes.
    pub fn trust_path<'a>(self, at: impl Into<AccountAt<'a>>) -> PathBuf {
        let at = at.into();
        self.older_config(at)
            .unwrap_or_else(|| at.place(self.dialect().adapter(), AccountPath::FolderTrust))
    }

    /// Claude Code's older global config, `.config.json`, which it reads in
    /// place of `.claude.json` while that file has content: for its folder
    /// trust and its MCP servers alike (`crate::mcp::McpFile::of`).
    pub(crate) fn older_config(self, at: AccountAt<'_>) -> Option<PathBuf> {
        if self != Self::ClaudeCode {
            return None;
        }
        let legacy = self.home_directory(at).join(".config.json");
        std::fs::metadata(&legacy)
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
            .then_some(legacy)
    }
}

/// The hook events Hide registers.
///
/// These are exactly the events both shipped runtimes declare, verified
/// against the installed Claude Code binary and the Codex configuration on
/// 2026-09-09 (PRD D-09), and `PreToolUse`, observed on Claude Code 2.1.292
/// and codex-cli 0.160.0 on 2026-10-07 (PRD herdr-spawn-guard D-06). An event
/// only one runtime has is registered for that runtime alone
/// ([`AgentRuntime::events`]): a key a runtime does not know is a key its
/// settings validator may reject, and the sweep at `Stop` already covers
/// what `SessionEnd` would.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookEvent {
    /// A new session took over this pane: the pane's counts start again.
    SessionStart,
    /// One user prompt is about to be submitted. This event performs only a
    /// bounded, read-only project Memory lookup.
    UserPromptSubmit,
    /// One subagent started.
    SubagentStart,
    /// One subagent finished.
    SubagentStop,
    /// The turn ended, so nothing this session spawned is still running. This
    /// is what clears a count left behind by a `SubagentStop` that never
    /// arrived (PRD B31, D-53).
    Stop,
    /// A tool call is about to run. Hide's entry selects the tools by its
    /// matcher and refuses only what a guard names (`crate::pre_tool`); every
    /// other call passes untouched.
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    /// The agent is about to ask the operator for a permission (Claude Code
    /// and Codex): the pane's agent waits for them (PRD engine-bundle-1
    /// D-10). Hide's entry decides nothing, so the agent's own prompt runs.
    PermissionRequest,
    /// Claude Code tells the operator something; a permission or input
    /// prompt is a wait, and its idle prompt is the rest an interrupted turn
    /// ends in, which no `Stop` reports.
    Notification,
    /// The operator interrupted the turn (Codex), which no `Stop` reports.
    Interrupt,
    /// The provider refused the turn and it ended (Claude Code): an API
    /// error or a usage limit, which no `Stop` reports. Claude Code 2.1.322
    /// sends it with the refusal's kind (`error`) and the line it showed
    /// (`last_assistant_message`), and on a subscription's usage limit then
    /// waits on its screen for the reset, which the engine reads as working
    /// (with-hide/hide mission-56).
    StopFailure,
}

impl HookEvent {
    pub const ALL: [HookEvent; 12] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::SubagentStart,
        Self::SubagentStop,
        Self::Stop,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PostToolUseFailure,
        Self::PermissionRequest,
        Self::Notification,
        Self::Interrupt,
        Self::StopFailure,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::Stop => "Stop",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::PermissionRequest => "PermissionRequest",
            Self::Notification => "Notification",
            Self::Interrupt => "Interrupt",
            Self::StopFailure => "StopFailure",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.name() == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_marker_is_found_with_its_version_and_only_for_hides_own_entries() {
        let command = "'/Applications/hide.app/Contents/MacOS/hide-agent-hooks' hook \
                       --event SubagentStart --source hide-subagents@1";
        assert_eq!(marker_version_in(command), Some(1));
        assert_eq!(
            marker_version_in("/opt/other/hook.sh --source other-tool@4"),
            None
        );
        assert_eq!(
            marker_version_in("/opt/other/hide-subagents-lookalike.sh"),
            None
        );
    }

    #[test]
    fn a_later_version_is_read_back_verbatim() {
        assert_eq!(parse_source_version("hide-subagents@12"), Some(12));
        assert_eq!(parse_source_version("hide-subagents@"), None);
        assert_eq!(parse_source_version("hide-subagents"), None);
    }

    #[test]
    fn every_runtime_answers_a_distinct_config_path_under_the_given_home() {
        let home = Path::new("/Users/example");
        assert_eq!(
            AgentRuntime::ClaudeCode.config_path(home),
            Path::new("/Users/example/.claude/settings.json")
        );
        assert_eq!(
            AgentRuntime::Codex.config_path(home),
            Path::new("/Users/example/.codex/hooks.json")
        );
    }

    #[test]
    fn ascii_json_decodes_to_the_same_value_in_ascii_only() {
        let context = format!("{START_CONTEXT} 메모리 \u{1F600}");
        let json = serde_json::json!({ "hookSpecificOutput": { "additionalContext": context } })
            .to_string();
        let ascii = ascii_json(&json);
        assert!(ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains(r"--beside \u2026"));
        assert!(ascii.contains(r"\ud83d\ude00"), "outside the BMP as a pair");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&ascii).unwrap(),
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }

    #[test]
    fn both_runtimes_emit_the_start_context_only_at_session_start() {
        for runtime in AgentRuntime::ALL {
            let output: serde_json::Value = serde_json::from_str(
                &hook_stdout(runtime, HookEvent::SessionStart).expect("SessionStart output"),
            )
            .expect("valid JSON output");
            assert_eq!(
                output["hookSpecificOutput"]["hookEventName"],
                "SessionStart"
            );
            assert_eq!(
                output["hookSpecificOutput"]["additionalContext"],
                START_CONTEXT
            );
            for event in [
                HookEvent::SubagentStart,
                HookEvent::SubagentStop,
                HookEvent::Stop,
            ] {
                assert_eq!(hook_stdout(runtime, event), None);
            }
        }
    }

    /// The start text points at where to read more and names the command
    /// for each request the operator makes most; every other rule is in
    /// `hide guide` (hided's guide tests hold that nothing was lost).
    #[test]
    fn the_start_context_points_at_the_guide_and_names_the_common_commands() {
        assert!(START_CONTEXT.starts_with("Inside Hide, read more when you need it:"));
        // Hide's commands only: no Herdr program, variable or command
        // (PRD engine-bundle-1 D-6, B1).
        assert!(!START_CONTEXT.to_lowercase().contains("herdr"));
        for command in [
            "`hide guide <topic>`",
            "`hide workspace info`",
            "`hide session list`",
            "`hide session record <id>`",
            "`hide memory show <id>`",
            "`hide file open <path>`",
            "`hide diff open <path>`",
            "`hide browser open <url-or-path>`",
            "without asking, then say in one line what you opened",
            "never the system opener",
            "`hide agent spawn --beside …`",
            "read `hide guide agents` before your first spawn",
            "`hide agent spawn --parent here …`",
            "`hide request send <agent> …`, never by typing into its pane",
            "first ask in one line: Shall I run this as a mission, or do it directly?",
            "`hide mission help`",
            "`hide issue help`",
        ] {
            assert!(START_CONTEXT.contains(command), "{command}");
        }
        // Open takes no --beside; only a spawn does (PRD
        // unseen-agent-views B29).
        assert!(!START_CONTEXT.contains("open <path> --beside"));
        assert!(!START_CONTEXT.contains("<url-or-path> --beside"));
        assert!(
            START_CONTEXT.chars().count() < 2_000,
            "{}",
            START_CONTEXT.len()
        );
    }

    /// The operator's instruction, the Memory index and the start text come
    /// first, then the Workspace note and the open sessions, and a part
    /// left out leaves no gap.
    #[test]
    fn a_session_start_reads_the_global_instruction_and_memory_first() {
        let context = session_start_context(
            Some("<global/>\n"),
            Some("<memory/>\n"),
            Some("<sessions/>\n"),
            Some("<workspace/>"),
        );
        assert_eq!(
            context,
            format!("<global/>\n\n<memory/>\n\n{START_CONTEXT}\n\n<workspace/>\n\n<sessions/>\n")
        );
        assert_eq!(session_start_context(None, None, None, None), START_CONTEXT);
        assert_eq!(
            session_start_context(None, Some(""), Some("<sessions/>\n"), Some("")),
            format!("{START_CONTEXT}\n\n<sessions/>\n")
        );
    }

    /// Past the limit the parts give way from the back, each to one line:
    /// the open sessions, then the Workspace note, then the start text;
    /// the global instruction and the Memory index stay whole.
    #[test]
    fn past_its_limit_a_session_start_gives_way_from_the_back() {
        let global = format!("<global>{}</global>\n", "g".repeat(3_000));
        let memory = format!("<memory>{}</memory>\n", "m".repeat(3_000));
        let sessions = format!("<sessions>{}</sessions>\n", "s".repeat(3_000));
        let workspace = workspace_note(&"w".repeat(1_000));
        let start = |memory: &str, workspace: &str, sessions: Option<&str>| {
            session_start_context(Some(&global), Some(memory), sessions, Some(workspace))
        };
        assert_eq!(
            start(&memory, &workspace, Some(&sessions)),
            format!("{global}\n{memory}\n{START_CONTEXT}\n\n{workspace}\n\n{SESSIONS_LEFT_OUT}")
        );
        let long_workspace = workspace_note(&"w".repeat(2_000));
        assert_eq!(
            start(&memory, &long_workspace, Some(&sessions)),
            format!(
                "{global}\n{memory}\n{START_CONTEXT}\n\n{WORKSPACE_LEFT_OUT}\n\n{SESSIONS_LEFT_OUT}"
            )
        );
        let memory = format!("<memory>{}</memory>\n", "m".repeat(5_000));
        assert_eq!(
            start(&memory, &workspace, Some(&sessions)),
            format!(
                "{global}\n{memory}\n{START_LEFT_OUT}\n\n{WORKSPACE_LEFT_OUT}\n\n{SESSIONS_LEFT_OUT}"
            )
        );
        // Nothing stands in for a part there was none of.
        assert_eq!(
            session_start_context(Some(&global), Some(&memory), None, None),
            format!("{global}\n{memory}\n{START_LEFT_OUT}")
        );
        // The first-prompt route of OpenCode, Pi and omp keeps its sessions
        // by the same rule.
        let memory_small = format!("<memory>{}</memory>\n", "m".repeat(1_000));
        assert_eq!(
            open_sessions_within_limit(Some(&global), Some(&memory_small), Some(&sessions)),
            Some(sessions.as_str())
        );
        assert_eq!(
            open_sessions_within_limit(Some(&global), Some(&memory), Some(&sessions)),
            Some(SESSIONS_LEFT_OUT)
        );
        assert_eq!(
            open_sessions_within_limit(Some(&global), Some(&memory), None),
            None
        );
        // Over the limit with the instruction and the index alone, both are
        // still given whole.
        let memory = format!("<memory>{}</memory>\n", "m".repeat(9_000));
        assert!(
            start(&memory, &workspace, Some(&sessions)).starts_with(&format!("{global}\n{memory}"))
        );
    }

    #[test]
    fn a_foreign_session_is_found_by_the_variable_its_agent_documents() {
        let with = |name: &'static str| move |asked: &str| (asked == name).then_some("1");
        assert_eq!(
            ForeignOrigin::detect(with("CURSOR_VERSION")),
            Some(ForeignOrigin::Cursor)
        );
        for name in ["OPENCODE", "OPENCODE_PID"] {
            assert_eq!(
                ForeignOrigin::detect(with(name)),
                Some(ForeignOrigin::OpenCode)
            );
        }
        for name in ["GROK_HOOK_EVENT", "GROK_SESSION_ID"] {
            assert_eq!(ForeignOrigin::detect(with(name)), Some(ForeignOrigin::Grok));
        }
        assert_eq!(ForeignOrigin::detect(with("CLAUDE_PROJECT_DIR")), None);
        assert_eq!(ForeignOrigin::detect(|_: &str| None::<&str>), None);
    }

    #[test]
    fn cursor_grok_and_opencode_silence_claude_codes_hook_and_every_foreign_session_takes_no_letters()
     {
        let only = |name: &'static str| move |asked: &str| (asked == name).then_some("1");
        for name in [
            "CURSOR_VERSION",
            "GROK_HOOK_EVENT",
            "OPENCODE",
            "OPENCODE_PID",
        ] {
            assert!(silences_claude_hook(only(name)), "{name}");
        }
        assert!(!silences_claude_hook(only("GROK_SESSION_ID")));
        assert!(takes_letters(|_: &str| None::<&str>));
        for name in ["CURSOR_VERSION", "OPENCODE", "GROK_HOOK_EVENT"] {
            assert!(!takes_letters(|asked: &str| (asked == name).then_some("1")));
        }
    }
}
