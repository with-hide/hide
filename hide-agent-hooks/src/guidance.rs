//! Hide's hook for the agents beyond Claude Code and Codex whose official
//! documentation gives command hooks: Cursor and Grok (issue #517, PRD
//! grok-cursor-hooks), and agy (PRD antigravity-first-class).
//!
//! Claude Code and Codex are instrumented: their hooks count subagents,
//! read Project Memory and pull letters (`install`, `runtime`). Cursor and
//! Grok are the basic tier: neither documents a prompt hook that adds text,
//! so they get no letters, Memory or bell. What they do document is a
//! refusal of a shell call and the start and end of a subagent, so their
//! hooks carry the spawn guard and the subagent count, and Cursor's session
//! start also prints Hide's guidance in the field its documentation names
//! (Grok discards what a session-start hook prints). This is a separate
//! module with its own marker rather than more variants of
//! [`crate::AgentRuntime`], whose six-event file, Memory and diagnosis these
//! agents never feed (engineering rule 5, 13).
//!
//! The rules are the ones `install` states for the other hooks: a file that
//! does not parse is never written, what another tool wrote is never
//! touched, a removal takes only the entries carrying Hide's marker,
//! and installing twice converges. Each agent's file and entry shape is
//! pinned to its documentation's own example by a test.
//!
//! The other seven agents of the retired adapters (Gemini CLI, Qwen Code,
//! Factory Droid, Copilot CLI, Kiro, Augment, Junie) are no longer written. Their layouts
//! stay so the kit's one-time retirement can take Hide's marked entries out of
//! the files an earlier build wrote them into ([`remove`]); [`install`]
//! refuses them. The layouts go once that retirement has shipped.
//!
//! | Agent | File | Shape |
//! | --- | --- | --- |
//! | Gemini CLI (retired) | `~/.gemini/settings.json` | `hooks.SessionStart[{matcher, hooks[{name, type, command, timeout ms}]}]` |
//! | Qwen Code (retired) | `~/.qwen/settings.json` | `hooks.SessionStart[{hooks[{name, type, command, timeout s}]}]` |
//! | Factory Droid (retired) | `~/.factory/hooks.json`, else the `hooks` key of `settings.json` | `SessionStart[{hooks[{type, command, timeout s}]}]` |
//! | Cursor | `~/.cursor/hooks.json` | `{version 1, hooks.<camelCase event>[{command, timeout s, matcher?}]}` |
//! | Grok | `~/.grok/hooks/hide.json`, Hide's own | `hooks.<Event>[{matcher?, hooks[{type, command, timeout s}]}]` |
//! | agy | `~/.gemini/config/hooks.json`, the `hide` block | `hide.<Event>[{type, command, timeout s}]`, `hide.PreToolUse[{matcher, hooks[...]}]` |
//!
//! agy is not of the basic tier: its `SessionStart` and `PreInvocation`
//! answers put text into the conversation and its `PreToolUse` refuses a
//! call, so its hook carries the guidance, Memory and the letters as well
//! (`crate::antigravity`). Its file is the shared one agy reads beside the
//! operator's and Herdr's blocks; Hide's block is the top-level key `hide`
//! and nothing else of the file is Hide's.
//! | Augment (retired) | `~/.augment/settings.json` | `hooks.SessionStart[{hooks[{type, command, timeout ms}]}]` |
//! | Junie (retired) | `~/.junie/config.json` | `hooks.SessionStart[{hooks[{type, command, timeout s, async}]}]` |
//! | Copilot CLI (retired) | `~/.copilot/hooks/hide-guidance.json`, Hide's own | `{version 1, hooks.sessionStart[{type, bash, powershell, timeoutSec}]}` |
//! | Kiro CLI 3.0 (retired) | `~/.kiro/hooks/hide-guidance.json`, Hide's own | `{version "v1", hooks[{name, trigger, action{type, command}, timeout s}]}` |

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::install::{
    HookDocument, HookStatus, InstallFailure, InstallOutcome, Quoting, RemoveOutcome,
    read_document, write_document,
};
use crate::runtime::{HookEvent, PURPOSE_CONTEXT, marker_version_of};

/// The marker Hide stamps into every guidance hook it installs, carried as
/// the `--source` argument of the command like the other hooks' marker.
pub const GUIDANCE_SOURCE_NAME: &str = "hide-guidance";

/// Raise it when the command or entry Hide writes changes shape. Version 2
/// added the guard and subagent events beside Cursor's session start;
/// version 3 stopped writing Cursor's `subagentStart`, `subagentStop` and
/// `stop`, which only fed a count Cursor no longer reports (issue 940).
/// Version 4 prepares Cursor's PowerShell input before starting the helper.
/// Version 5 selects the file tools for the guardrails.
pub const GUIDANCE_VERSION: u32 = 6;

/// The one line every guidance hook adds after the shared instruction, so an
/// agent that Hide cannot place in a Workspace still learns where the usage
/// lives. The text itself is read from the binary, not written here.
pub const GUIDANCE_LINE: &str = "Hide is installed on this machine: run `hide browser help` to learn how to read and drive a browser display from here, and `hide workspace info` to see what this checkout's Workspace offers.";

/// How long the agent waits for the hook, in seconds.
const TIMEOUT_SECONDS: u32 = 8;

/// An agent whose command hook Hide writes the guidance into, or one it
/// wrote it into before and only takes the entry out of ([`Self::is_retired`]).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GuidanceAgent {
    Gemini,
    Qwen,
    Droid,
    Copilot,
    Kiro,
    Cursor,
    Augment,
    Junie,
    Grok,
    Antigravity,
}

impl GuidanceAgent {
    pub const ALL: [GuidanceAgent; 10] = [
        Self::Gemini,
        Self::Qwen,
        Self::Droid,
        Self::Copilot,
        Self::Kiro,
        Self::Cursor,
        Self::Augment,
        Self::Junie,
        Self::Grok,
        Self::Antigravity,
    ];

    /// The stable identifier: the agent's adapter id, and the `--runtime` of
    /// its hook command.
    pub fn id(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-cli",
            Self::Qwen => "qwen-code",
            Self::Droid => "factory-droid",
            Self::Copilot => "copilot-cli",
            Self::Kiro => "kiro",
            Self::Cursor => hide_agent_adapter::AgentId::Cursor.adapter().id,
            Self::Augment => "augment",
            Self::Junie => "junie",
            Self::Grok => hide_agent_adapter::AgentId::Grok.adapter().id,
            Self::Antigravity => hide_agent_adapter::AgentId::Antigravity.adapter().id,
        }
    }

    /// The agents Hide still writes a hook for, in Settings order.
    pub const LIVE: [GuidanceAgent; 3] = [Self::Grok, Self::Cursor, Self::Antigravity];

    /// The agents whose hook an earlier build wrote and this one only removes.
    pub fn is_retired(self) -> bool {
        !Self::LIVE.contains(&self)
    }

    /// The live agent whose id this is; a retired agent's id answers `None`,
    /// so a hook entry an earlier build left in its file runs nothing.
    pub fn from_id(id: &str) -> Option<Self> {
        use hide_agent_adapter::{GuidanceDialect, HookInstall};
        match hide_agent_adapter::adapter(id)?.hook {
            HookInstall::Guidance(GuidanceDialect::Cursor) => Some(Self::Cursor),
            HookInstall::Guidance(GuidanceDialect::Grok) => Some(Self::Grok),
            HookInstall::Guidance(GuidanceDialect::Antigravity) => Some(Self::Antigravity),
            _ => None,
        }
    }

    /// The hook dialect this agent's payloads and answers are written in.
    /// A retired agent has none: its entries run nothing.
    pub fn dialect(self) -> Option<hide_agent_adapter::HookDialect> {
        match self {
            Self::Cursor => Some(hide_agent_adapter::HookDialect::Cursor),
            Self::Grok => Some(hide_agent_adapter::HookDialect::Grok),
            Self::Antigravity => Some(hide_agent_adapter::HookDialect::Antigravity),
            _ => None,
        }
    }

    /// The events Hide registers for this agent, each under the key the
    /// agent's documentation names. A retired agent only ever had the
    /// session start. Grok's session start fires once per main session and
    /// starts the pane's count over; Cursor's also prints the guidance.
    fn slots(self) -> &'static [Slot] {
        const CURSOR: &[Slot] = &[
            Slot::new(HookEvent::SessionStart, "sessionStart", None),
            // Every tool call: the guardrails read the shell and file calls
            // (Cursor 2026.10.14 sends an edit as `Write`), and the letters
            // due while the agent works go at any call's gap (PRD
            // letters-to-busy-agents B6), so no matcher selects.
            Slot::new(HookEvent::PreToolUse, "preToolUse", None),
        ];
        const GROK: &[Slot] = &[
            Slot::new(HookEvent::SessionStart, "SessionStart", None),
            // Every tool call, which Grok's guide reads an omitted matcher
            // as: the guardrails read the shell and file calls, and the
            // letters due while the agent works go at any call's gap (PRD
            // letters-to-busy-agents B6).
            Slot::new(HookEvent::PreToolUse, "PreToolUse", None),
            Slot::new(HookEvent::SubagentStart, "SubagentStart", None),
            Slot::new(HookEvent::SubagentStop, "SubagentStop", None),
            Slot::new(HookEvent::Stop, "Stop", None),
        ];
        // agy's events: the first prompt of a conversation, every model call
        // (the helper reads it as the prompt hook), the tool call and the
        // turn's end. The matcher is a regular expression on the whole tool
        // name: the shell, the file tools the guardrails judge and the
        // question a Factory worker is refused (agy 1.3.6's CLI toolset).
        const ANTIGRAVITY: &[Slot] = &[
            Slot::new(HookEvent::SessionStart, "SessionStart", None),
            Slot::new(HookEvent::UserPromptSubmit, "PreInvocation", None),
            Slot::new(
                HookEvent::PreToolUse,
                "PreToolUse",
                Some(
                    "run_command|view_file|write_to_file|replace_file_content|multi_replace_file_content|ask_question",
                ),
            ),
            Slot::new(HookEvent::Stop, "Stop", None),
        ];
        const CAMEL_SESSION_START: &[Slot] =
            &[Slot::new(HookEvent::SessionStart, "sessionStart", None)];
        const SESSION_START: &[Slot] = &[Slot::new(HookEvent::SessionStart, "SessionStart", None)];
        match self {
            Self::Cursor => CURSOR,
            Self::Grok => GROK,
            Self::Antigravity => ANTIGRAVITY,
            Self::Copilot => CAMEL_SESSION_START,
            Self::Gemini | Self::Qwen | Self::Droid | Self::Kiro | Self::Augment | Self::Junie => {
                SESSION_START
            }
        }
    }

    /// The event keys an earlier build wrote an entry under for this agent and
    /// this one no longer does. An install takes Hide's entries out of them,
    /// beside the operator's and other tools' entries, which stay.
    fn retired_keys(self) -> &'static [&'static str] {
        match self {
            // The count Cursor's hooks fed is not reported any more
            // (`subagent_counts` is none), so these ran for nothing.
            Self::Cursor => &["subagentStart", "subagentStop", "stop"],
            _ => &[],
        }
    }

    /// Whether `id` names an agent whose hook an earlier build wrote.
    pub fn is_retired_id(id: &str) -> bool {
        Self::ALL
            .into_iter()
            .any(|agent| agent.is_retired() && agent.id() == id)
    }

    /// The folder whose presence means the agent is set up for this account.
    /// agy makes `~/.gemini/config` on its first run, and Herdr's integration
    /// needs it too; `~/.gemini` alone is also the retired Gemini CLI's.
    pub fn home_directory(self, home: &Path) -> PathBuf {
        match self {
            Self::Gemini => home.join(".gemini"),
            Self::Qwen => home.join(".qwen"),
            Self::Droid => home.join(".factory"),
            Self::Copilot => home.join(".copilot"),
            Self::Kiro => home.join(".kiro"),
            Self::Cursor => home.join(".cursor"),
            Self::Augment => home.join(".augment"),
            Self::Junie => home.join(".junie"),
            Self::Grok => home.join(".grok"),
            Self::Antigravity => home.join(".gemini").join("config"),
        }
    }

    /// Whether Hide writes this agent's hook on this system. Copilot CLI has a
    /// `powershell` key of its own. Grok and Cursor run a hook's command as
    /// PowerShell on Windows: Grok's hooks guide says so and its source picks
    /// PowerShell 7, then Windows PowerShell (`xai-grok-config/src/shell.rs`),
    /// and the Cursor CLI's own bundle runs it inside a PowerShell pipeline
    /// (`docs/agent-hooks.md`, Installing). agy's guide, embedded in its
    /// binary, runs a hook's command through `cmd /c` on Windows. The retired
    /// agents' documentation names no Windows shell, so a guard written for
    /// one would be a guess.
    pub fn supported_here(self) -> Result<(), &'static str> {
        if cfg!(windows)
            && !matches!(
                self,
                Self::Copilot | Self::Grok | Self::Cursor | Self::Antigravity
            )
        {
            return Err("its documentation does not say which shell runs a hook on Windows");
        }
        Ok(())
    }

    /// The file Hide writes (and reads back) for this agent.
    pub fn config_path(self, home: &Path) -> PathBuf {
        self.layout(home).path
    }

    fn layout(self, home: &Path) -> Layout {
        let folder = self.home_directory(home);
        match self {
            Self::Gemini => Layout::event_map(folder.join("settings.json"), true),
            Self::Qwen => Layout::event_map(folder.join("settings.json"), true),
            Self::Droid => {
                // Droid reads `hooks.json` when it exists and the `hooks` key
                // of `settings.json` only when it does not, so creating the
                // file beside a settings file that has hooks would hide them.
                let hooks_file = folder.join("hooks.json");
                let settings = folder.join("settings.json");
                if !hooks_file.exists() && settings_has_hooks(&settings) {
                    Layout::event_map(settings, true)
                } else {
                    Layout::event_map(hooks_file, false)
                }
            }
            Self::Copilot => Layout {
                path: folder.join("hooks").join("hide-guidance.json"),
                shape: Shape::Copilot,
                own_file: true,
            },
            Self::Kiro => Layout {
                path: folder.join("hooks").join("hide-guidance.json"),
                shape: Shape::Kiro,
                own_file: true,
            },
            // Cursor's `hooks.json` is shared with the operator's own hooks;
            // Hide creates it when missing and deletes it again only when
            // nothing but the scaffolding is left.
            Self::Cursor => Layout {
                path: folder.join("hooks.json"),
                shape: Shape::Cursor,
                own_file: true,
            },
            Self::Augment => Layout::event_map(folder.join("settings.json"), true),
            Self::Junie => Layout::event_map(folder.join("config.json"), true),
            // Grok merges every `~/.grok/hooks/*.json`, so Hide's is a file of
            // its own beside Herdr's and Orca's.
            Self::Grok => Layout::event_map(folder.join("hooks").join("hide.json"), true),
            // agy reads one global `hooks.json` whose top-level keys are
            // named blocks; Herdr's is `herdr`, Hide's is `hide`. Hide creates
            // the file when agy has none and deletes it again only when
            // nothing is left in it.
            Self::Antigravity => Layout {
                path: folder.join("hooks.json"),
                shape: Shape::NamedBlock { block: "hide" },
                own_file: true,
            },
        }
    }

    /// How the helper's path is quoted in this agent's command on this
    /// system, and read back from it.
    fn quoting(self) -> Quoting {
        if cfg!(windows) && self == Self::Antigravity {
            Quoting::Cmd
        } else {
            Quoting::NATIVE
        }
    }
}

/// One event Hide registers: the helper's `--event`, the key the agent's
/// file keeps it under, and the matcher its entry selects with.
#[derive(Clone, Copy, Debug)]
struct Slot {
    event: HookEvent,
    key: &'static str,
    matcher: Option<&'static str>,
}

impl Slot {
    const fn new(event: HookEvent, key: &'static str, matcher: Option<&'static str>) -> Self {
        Self {
            event,
            key,
            matcher,
        }
    }
}

fn settings_has_hooks(settings: &Path) -> bool {
    matches!(
        read_document(settings),
        Ok(Some(document)) if document.as_object().is_some_and(|root| root.contains_key("hooks"))
    )
}

/// Where in the file Hide's entry lives.
#[derive(Clone, Debug)]
struct Layout {
    path: PathBuf,
    shape: Shape,
    /// The file may be Hide's alone: Hide creates it, and deletes it when
    /// nothing else is left in it.
    own_file: bool,
}

#[derive(Clone, Debug)]
enum Shape {
    /// `{"hooks": {"<Event>": [group]}}` (`under_hooks_key`), or the events
    /// at the top of the file.
    EventMap { under_hooks_key: bool },
    /// `{"version": 1, "hooks": {"sessionStart": [entry]}}`.
    Copilot,
    /// `{"version": "v1", "hooks": [entry]}`.
    Kiro,
    /// `{"version": 1, "hooks": {"<camelCaseEvent>": [entry]}}`, Cursor's
    /// own file.
    Cursor,
    /// `{"<block>": {"<Event>": [entry]}, ...}`: agy's named blocks, of
    /// which `block` is Hide's.
    NamedBlock { block: &'static str },
}

impl Layout {
    fn event_map(path: PathBuf, under_hooks_key: bool) -> Self {
        Self {
            path,
            shape: Shape::EventMap { under_hooks_key },
            // A file Hide may have created: Grok's own file, Droid's
            // `hooks.json` (an emptied one is deleted, since left as `{}` it
            // would still shadow the hooks the operator keeps in
            // `settings.json`) and a settings file that did not exist. One
            // that holds only what Hide's removal left empty goes, and any
            // other key of the operator's keeps it.
            own_file: true,
        }
    }
}

// --- What the hook prints --------------------------------------------------------

/// The context a guidance hook delivers: the shared instruction, the
/// pointer at `hide browser help`, and the Workspace guidance when the
/// daemon answered for this checkout.
pub fn session_context(workspace: Option<&str>) -> String {
    match workspace.filter(|context| !context.is_empty()) {
        Some(workspace) => format!("{PURPOSE_CONTEXT}\n\n{GUIDANCE_LINE}\n\n{workspace}"),
        None => format!("{PURPOSE_CONTEXT}\n\n{GUIDANCE_LINE}"),
    }
}

/// What the agent's session-start hook writes to stdout for `context`, in
/// the form its documentation says it reads: Gemini CLI and Copilot CLI
/// parse exactly one JSON value, Qwen Code, Factory Droid and Augment read
/// the `hookSpecificOutput` envelope, Cursor reads `additional_context`,
/// Junie reads `additionalContext`, and Kiro reads plain text. Grok discards
/// what a session-start hook prints, so it is told nothing.
pub fn stdout(agent: GuidanceAgent, context: &str) -> Option<String> {
    Some(match agent {
        GuidanceAgent::Gemini => {
            json!({ "hookSpecificOutput": { "additionalContext": context } }).to_string()
        }
        GuidanceAgent::Qwen | GuidanceAgent::Droid | GuidanceAgent::Augment => json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        })
        .to_string(),
        GuidanceAgent::Copilot | GuidanceAgent::Junie => {
            json!({ "additionalContext": context }).to_string()
        }
        GuidanceAgent::Cursor => json!({ "additional_context": context }).to_string(),
        GuidanceAgent::Kiro => context.to_owned(),
        GuidanceAgent::Antigravity => return crate::antigravity::inject(context),
        GuidanceAgent::Grok => return None,
    })
}

// --- Writing the entry -----------------------------------------------------------

fn arguments(agent: GuidanceAgent, event: HookEvent) -> String {
    format!(
        "hook --runtime {} --event {} --source {GUIDANCE_SOURCE_NAME}@{GUIDANCE_VERSION}",
        agent.id(),
        event.name()
    )
}

/// The guarded POSIX command: it runs the helper only while it is there, so
/// a removed app or helper folder is a hook that does nothing and succeeds
/// (`docs/agent-hooks.md`, Installing). `otherwise` is what the command
/// prints when the helper is gone.
fn posix_command(
    helper: &Path,
    agent: GuidanceAgent,
    event: HookEvent,
    otherwise: Option<&str>,
) -> String {
    let quoted = Quoting::Posix.quote(&helper.display().to_string());
    let arguments = arguments(agent, event);
    match otherwise {
        Some(text) => format!(
            "if [ -x {quoted} ]; then exec {quoted} {arguments}; else printf '%s\\n' {}; fi",
            Quoting::Posix.quote(text)
        ),
        None => format!("if [ -x {quoted} ]; then exec {quoted} {arguments}; fi"),
    }
}

/// The same guard for PowerShell: Copilot CLI's `powershell` key, and Grok's
/// command on Windows, which Grok hands to `pwsh` or `powershell.exe` as
/// `-NoProfile -NonInteractive -Command <command>` with the event on stdin.
fn powershell_command(helper: &Path, agent: GuidanceAgent, event: HookEvent) -> String {
    let quoted = Quoting::PowerShell.quote(&helper.display().to_string());
    format!(
        "if (Test-Path -LiteralPath {quoted} -PathType Leaf) {{ & {quoted} {} }}",
        arguments(agent, event)
    )
}

/// Cursor's documentation calls `command` a "script path or command" and does
/// not say whether a shell parses it, so Hide writes the one form that means
/// the same either way: the helper's path and its arguments, with no shell
/// syntax. A missing helper then fails the hook, which Cursor documents as
/// letting the action proceed (any exit but 2), and the status read-back
/// reports the gone helper. A path with a character a shell would read keeps
/// the quoted, guarded form, which for a permission event prints the allow
/// answer itself when the helper is gone.
fn plain_command(helper: &Path, agent: GuidanceAgent, event: HookEvent) -> String {
    let path = helper.display().to_string();
    let plain = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._+@:~-".contains(c));
    if plain {
        format!("{path} {}", arguments(agent, event))
    } else {
        posix_command(
            helper,
            agent,
            event,
            crate::basic::cursor_permission_event(event).then_some(crate::basic::CURSOR_ALLOW),
        )
    }
}

/// Cursor's command on Windows. The Cursor CLI writes the event to a file and
/// runs `Get-Content <file> -Raw | & { $input | <command> }` under PowerShell 7
/// or Windows PowerShell (read from its 2026.10.01 Windows package), so the
/// command is one pipeline element. Prepare the native pipeline's text
/// before starting the helper: PowerShell otherwise starts the process
/// before initializing Out-String, spending the helper's payload deadline
/// on a cold formatter rather than reading the event (issue 987).
/// A missing helper still answers allow for a permission event, as the
/// POSIX guard does.
fn cursor_powershell_command(helper: &Path, event: HookEvent) -> String {
    let quoted = Quoting::PowerShell.quote(&helper.display().to_string());
    let run = format!(
        "if (Test-Path -LiteralPath {quoted} -PathType Leaf) {{ $payload = $input | Out-String -Stream; $payload | & {quoted} {} }}",
        arguments(GuidanceAgent::Cursor, event)
    );
    if crate::basic::cursor_permission_event(event) {
        format!(
            "& {{ {run} else {{ {} }} }}",
            Quoting::PowerShell.quote(crate::basic::CURSOR_ALLOW)
        )
    } else {
        format!("& {{ {run} }}")
    }
}

/// agy's tool command on macOS and Linux. agy refuses a tool call whose hook
/// fails, so the ask answer is printed by the command itself
/// when the helper is gone and when it ends without answering (a binary that
/// cannot run, a crash): the helper prints its one line last and exits 0 on
/// every path it reaches, so a failed run has printed nothing.
fn antigravity_tool_command(helper: &Path) -> String {
    let quoted = Quoting::Posix.quote(&helper.display().to_string());
    let arguments = arguments(GuidanceAgent::Antigravity, HookEvent::PreToolUse);
    let ask = Quoting::Posix.quote(crate::antigravity::ASK);
    format!(
        "if [ -x {quoted} ]; then {quoted} {arguments} || printf '%s\\n' {ask}; else printf '%s\\n' {ask}; fi"
    )
}

/// agy's command on Windows, which agy hands to `cmd /c`: it runs the helper
/// while it is there and, for a tool call, answers ask itself when it is
/// gone or fails, as [`antigravity_tool_command`] does.
fn cmd_command(helper: &Path, agent: GuidanceAgent, event: HookEvent) -> String {
    let quoted = Quoting::Cmd.quote(&helper.display().to_string());
    let run = format!("{quoted} {}", arguments(agent, event));
    if event == HookEvent::PreToolUse {
        let ask = crate::antigravity::ASK;
        format!("if exist {quoted} ({run} || echo {ask}) else (echo {ask})")
    } else {
        format!("if exist {quoted} ({run})")
    }
}

/// The entry Hide writes for `slot`.
fn entry(agent: GuidanceAgent, helper: &Path, slot: &Slot) -> Value {
    let event = slot.event;
    let command = if cfg!(windows) && agent == GuidanceAgent::Antigravity {
        cmd_command(helper, agent, event)
    } else if agent == GuidanceAgent::Antigravity && event == HookEvent::PreToolUse {
        antigravity_tool_command(helper)
    } else if cfg!(windows) && agent == GuidanceAgent::Cursor {
        cursor_powershell_command(helper, event)
    } else if cfg!(windows) {
        powershell_command(helper, agent, event)
    } else if agent == GuidanceAgent::Cursor {
        plain_command(helper, agent, event)
    } else {
        posix_command(helper, agent, event, None)
    };
    let mut value = match agent {
        GuidanceAgent::Gemini => json!({
            "matcher": "*",
            "hooks": [{
                "name": GUIDANCE_SOURCE_NAME,
                "type": "command",
                "command": command,
                // Gemini CLI counts milliseconds.
                "timeout": TIMEOUT_SECONDS * 1000,
            }],
        }),
        GuidanceAgent::Qwen => json!({
            "hooks": [{
                "name": GUIDANCE_SOURCE_NAME,
                "type": "command",
                "command": command,
                "timeout": TIMEOUT_SECONDS,
            }],
        }),
        // Grok's groups take the matcher; its hooks count seconds, and its
        // `Stop` defaults to 600 of them, so the timeout is always written.
        GuidanceAgent::Droid | GuidanceAgent::Grok => json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": TIMEOUT_SECONDS,
            }],
        }),
        GuidanceAgent::Copilot => json!({
            "type": "command",
            "bash": posix_command(helper, agent, event, None),
            "powershell": powershell_command(helper, agent, event),
            "timeoutSec": TIMEOUT_SECONDS,
        }),
        GuidanceAgent::Cursor => json!({
            "command": command,
            "timeout": TIMEOUT_SECONDS,
        }),
        GuidanceAgent::Augment => json!({
            "hooks": [{
                "type": "command",
                "command": command,
                // Augment counts milliseconds.
                "timeout": TIMEOUT_SECONDS * 1000,
            }],
        }),
        // Junie ignores context from a synchronous SessionStart hook; an
        // asynchronous one is prepended to the next prompt, which is the
        // nearest delivery its documentation offers. Its timeout is seconds.
        GuidanceAgent::Junie => json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": TIMEOUT_SECONDS,
                "async": true,
            }],
        }),
        GuidanceAgent::Kiro => json!({
            "name": GUIDANCE_SOURCE_NAME,
            "trigger": "SessionStart",
            "action": { "type": "command", "command": command },
            "timeout": TIMEOUT_SECONDS,
        }),
        // agy's tool events take a group with the matcher, as in its own
        // examples; the other events take the handler itself. Its timeout
        // counts seconds, and a tool hook it kills at the timeout refuses the
        // call, so the helper's own budgets end far inside it.
        GuidanceAgent::Antigravity if slot.matcher.is_some() => json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": TIMEOUT_SECONDS,
            }],
        }),
        GuidanceAgent::Antigravity => json!({
            "type": "command",
            "command": command,
            "timeout": TIMEOUT_SECONDS,
        }),
    };
    if let (Some(matcher), Some(object)) = (slot.matcher, value.as_object_mut()) {
        object.insert("matcher".to_owned(), Value::from(matcher));
    }
    value
}

// --- Reading and changing the document -----------------------------------------

fn failure_shape(path: &Path, detail: &str) -> InstallFailure {
    InstallFailure::UnexpectedShape {
        path: path.display().to_string(),
        detail: detail.to_owned(),
    }
}

/// The array the entries under `key` live in, created when `create`.
fn entries<'a>(
    document: &'a mut Value,
    layout: &Layout,
    key: &str,
    create: bool,
) -> Result<Option<&'a mut Vec<Value>>, InstallFailure> {
    let path = &layout.path;
    let root = document
        .as_object_mut()
        .ok_or_else(|| failure_shape(path, "the document is not a JSON object"))?;
    let container: &mut Map<String, Value> = match &layout.shape {
        Shape::NamedBlock { block } => {
            let slot = if create {
                root.entry(*block)
                    .or_insert_with(|| Value::Object(Map::new()))
            } else {
                match root.get_mut(*block) {
                    Some(slot) => slot,
                    None => return Ok(None),
                }
            };
            slot.as_object_mut()
                .ok_or_else(|| failure_shape(path, &format!("\"{block}\" is not an object")))?
        }
        Shape::Kiro => {
            let slot = if create {
                root.entry("hooks")
                    .or_insert_with(|| Value::Array(Vec::new()))
            } else {
                match root.get_mut("hooks") {
                    Some(slot) => slot,
                    None => return Ok(None),
                }
            };
            return slot
                .as_array_mut()
                .map(Some)
                .ok_or_else(|| failure_shape(path, "\"hooks\" is not an array"));
        }
        Shape::EventMap {
            under_hooks_key: false,
        } => root,
        Shape::EventMap {
            under_hooks_key: true,
        }
        | Shape::Copilot
        | Shape::Cursor => {
            let slot = if create {
                root.entry("hooks")
                    .or_insert_with(|| Value::Object(Map::new()))
            } else {
                match root.get_mut("hooks") {
                    Some(slot) => slot,
                    None => return Ok(None),
                }
            };
            slot.as_object_mut()
                .ok_or_else(|| failure_shape(path, "\"hooks\" is not an object"))?
        }
    };
    let slot = if create {
        container
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()))
    } else {
        match container.get_mut(key) {
            Some(slot) => slot,
            None => return Ok(None),
        }
    };
    slot.as_array_mut()
        .map(Some)
        .ok_or_else(|| failure_shape(path, &format!("\"{key}\" is not an array")))
}

/// Every string anywhere in an entry. The marker is looked for in all of
/// them, so ownership never depends on which key an agent keeps its command
/// under.
fn strings<'a>(value: &'a Value, found: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => found.push(text),
        Value::Array(items) => items.iter().for_each(|item| strings(item, found)),
        Value::Object(map) => map.values().for_each(|item| strings(item, found)),
        _ => {}
    }
}

fn entry_marker_version(entry: &Value) -> Option<u32> {
    let mut found = Vec::new();
    strings(entry, &mut found);
    found
        .into_iter()
        .filter_map(|text| marker_version_of(GUIDANCE_SOURCE_NAME, text))
        .min()
}

/// The helper path in the first entry Hide owns.
fn entry_helper(agent: GuidanceAgent, entry: &Value) -> Option<String> {
    let mut found = Vec::new();
    strings(entry, &mut found);
    found
        .into_iter()
        .filter(|text| text.contains(GUIDANCE_SOURCE_NAME))
        .find_map(|text| {
            agent.quoting().first_quoted(text).or_else(|| {
                // Cursor's plain form: the helper's path, then its arguments.
                text.split_whitespace()
                    .next()
                    .filter(|first| first.starts_with('/'))
                    .map(str::to_owned)
            })
        })
}

/// Takes Hide's own hook out of `list` and nothing else. A group another
/// tool shares with Hide (`{"matcher": "*", "hooks": [hide, theirs]}`) keeps
/// the other tool's hook and loses only Hide's; a group left with no hook
/// goes, and so does a flat entry that carries the marker. Returns how many
/// of Hide's hooks were taken out.
fn strip_owned(list: &mut Vec<Value>) -> usize {
    let mut removed = 0;
    list.retain_mut(|entry| {
        if let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
            let before = hooks.len();
            hooks.retain(|hook| entry_marker_version(hook).is_none());
            let taken = before - hooks.len();
            removed += taken;
            return taken == 0 || !hooks.is_empty();
        }
        if entry_marker_version(entry).is_some() {
            removed += 1;
            return false;
        }
        true
    });
    removed
}

fn owned(entries: &[Value]) -> impl Iterator<Item = &Value> {
    entries
        .iter()
        .filter(|entry| entry_marker_version(entry).is_some())
}

/// Judges one agent without changing anything. Every event Hide registers
/// for the agent must carry Hide's entry for the hook to be Installed; one
/// that lacks it (an earlier version wrote fewer events, or the operator took
/// one out) reads Outdated, and the next pass writes it back.
pub fn status(agent: GuidanceAgent, home: &Path) -> HookStatus {
    if !agent.home_directory(home).is_dir() {
        return HookStatus::RuntimeAbsent;
    }
    let layout = agent.layout(home);
    let mut document = match read_document(&layout.path) {
        Ok(Some(document)) => document,
        Ok(None) => return HookStatus::NotInstalled,
        Err(reason) => return HookStatus::Failed { reason },
    };
    let mut lowest: Option<u32> = None;
    let mut missing_event = false;
    let mut helper = None;
    for slot in agent.slots() {
        let found = match entries(&mut document, &layout, slot.key, false) {
            Ok(Some(found)) => found,
            Ok(None) => {
                missing_event = true;
                continue;
            }
            Err(reason) => return HookStatus::Failed { reason },
        };
        match owned(found).filter_map(entry_marker_version).min() {
            Some(version) => lowest = Some(lowest.map_or(version, |best| best.min(version))),
            None => missing_event = true,
        }
        helper = helper.or_else(|| owned(found).find_map(|entry| entry_helper(agent, entry)));
    }
    let Some(version) = lowest else {
        return HookStatus::NotInstalled;
    };
    if let Some(helper) = helper
        && !Path::new(&helper).exists()
    {
        return HookStatus::Failed {
            reason: InstallFailure::HelperMissing {
                path: layout.path.display().to_string(),
                helper,
            },
        };
    }
    if missing_event || version < GUIDANCE_VERSION {
        HookStatus::Outdated { version }
    } else {
        HookStatus::Installed { version }
    }
}

/// The helper Hide's entry in `agent`'s file names, when there is one; the
/// kit compares it with the helper it would install.
pub fn installed_helper_path(agent: GuidanceAgent, home: &Path) -> Option<String> {
    let layout = agent.layout(home);
    let mut document = read_document(&layout.path).ok()??;
    for slot in agent.slots() {
        if let Some(found) = entries(&mut document, &layout, slot.key, false).ok()?
            && let Some(helper) = owned(found).find_map(|entry| entry_helper(agent, entry))
        {
            return Some(helper);
        }
    }
    None
}

/// Whether every entry of Hide's in `agent`'s file is exactly the one this
/// build writes for `helper`: one per event, byte for byte as a value. An
/// entry that carries Hide's marker over anything else was edited, and the
/// kit reads it Outdated and leaves it until Reinstall (PRD grok-cursor-hooks
/// B8).
pub fn matches_install(agent: GuidanceAgent, home: &Path, helper: &Path) -> bool {
    let layout = agent.layout(home);
    let Ok(Some(mut document)) = read_document(&layout.path) else {
        return false;
    };
    agent.slots().iter().all(|slot| {
        let Ok(Some(found)) = entries(&mut document, &layout, slot.key, false) else {
            return false;
        };
        let wanted = entry(agent, helper, slot);
        // A group another tool put a hook into beside Hide's differs too, and
        // the reinstall that follows moves Hide's hook into a group of its own.
        let mut mine = owned(found);
        mine.next() == Some(&wanted) && mine.next().is_none()
    })
}

fn blank_document(layout: &Layout) -> Value {
    match layout.shape {
        Shape::EventMap { .. } | Shape::NamedBlock { .. } => Value::Object(Map::new()),
        Shape::Copilot | Shape::Cursor => json!({ "version": 1 }),
        Shape::Kiro => json!({ "version": "v1" }),
    }
}

/// Appends Hide's entries, preserving everything already in the file; a
/// second install converges on one entry per event.
pub fn install(
    agent: GuidanceAgent,
    home: &Path,
    helper: &Path,
) -> Result<InstallOutcome, InstallFailure> {
    if agent.is_retired() {
        return Err(failure_shape(
            &agent.config_path(home),
            "Hide no longer writes this agent's hook",
        ));
    }
    agent
        .supported_here()
        .map_err(|why| InstallFailure::UnexpectedShape {
            path: agent.config_path(home).display().to_string(),
            detail: why.to_owned(),
        })?;
    if agent.quoting() == Quoting::Cmd && !Quoting::cmd_safe(&helper.display().to_string()) {
        return Err(InstallFailure::UnexpectedShape {
            path: agent.config_path(home).display().to_string(),
            detail: "the helper's path holds a character cmd would read inside its quotes"
                .to_owned(),
        });
    }
    let outcome = install_any(agent, home, helper)?;
    if agent == GuidanceAgent::Antigravity {
        crate::antigravity_trust::prepare_read_only(home)?;
    }
    Ok(outcome)
}

/// [`install`] without the retirement and system checks; the tests use it to
/// lay down the files an earlier build wrote for a retired agent, on any
/// system, since taking such an entry out never depends on the system's hook
/// shell.
fn install_any(
    agent: GuidanceAgent,
    home: &Path,
    helper: &Path,
) -> Result<InstallOutcome, InstallFailure> {
    let layout = agent.layout(home);
    let mut document =
        read_document(&layout.path)?.unwrap_or_else(|| HookDocument::new(blank_document(&layout)));
    let before = serde_json::to_string(&document).unwrap_or_default();
    // Cursor's documentation requires `version` (a positive integer, 1) in
    // `hooks.json`, so a file that never had one is given it, only when it is
    // absent, rather than being left in a state Cursor may refuse along with
    // the operator's own hooks.
    if matches!(layout.shape, Shape::Cursor)
        && let Some(root) = document.as_object_mut()
    {
        root.entry("version").or_insert_with(|| json!(1));
    }
    let mut preserved = 0;
    for slot in agent.slots() {
        let list = entries(&mut document, &layout, slot.key, true)?
            .ok_or_else(|| failure_shape(&layout.path, "the hook list could not be created"))?;
        strip_owned(list);
        preserved += list.len();
        list.push(entry(agent, helper, slot));
    }
    for key in agent.retired_keys() {
        if let Some(list) = entries(&mut document, &layout, key, false)?
            && strip_owned(list) != 0
        {
            let left = list.len();
            if left == 0 {
                drop_empty(&mut document, &layout, key);
            }
            preserved += left;
        }
    }
    let after = serde_json::to_string(&document).unwrap_or_default();
    if before == after {
        return Ok(InstallOutcome {
            changed: false,
            preserved_entries: preserved,
        });
    }
    // Grok's own file sits in `~/.grok/hooks`, which a Grok that has made
    // `~/.grok` may not have made yet; the agent's own folder is never made.
    if layout.own_file
        && agent.home_directory(home).is_dir()
        && let Some(parent) = layout.path.parent()
    {
        std::fs::create_dir_all(parent).map_err(|error| InstallFailure::NotWritable {
            path: layout.path.display().to_string(),
            detail: error.to_string(),
        })?;
    }
    write_document(&layout.path, &document)?;
    Ok(InstallOutcome {
        changed: true,
        preserved_entries: preserved,
    })
}

/// Takes out only the entries carrying Hide's marker; a file that was Hide's
/// alone goes with them.
pub fn remove(agent: GuidanceAgent, home: &Path) -> Result<RemoveOutcome, InstallFailure> {
    // Every file Hide could have written to, so an entry written beside a
    // settings file is still found after a `hooks.json` appeared (Droid).
    let mut layouts = vec![agent.layout(home)];
    if agent == GuidanceAgent::Droid {
        let folder = agent.home_directory(home);
        for layout in [
            Layout::event_map(folder.join("hooks.json"), false),
            Layout::event_map(folder.join("settings.json"), true),
        ] {
            if !layouts.iter().any(|known| known.path == layout.path) {
                layouts.push(layout);
            }
        }
    }
    let mut total = RemoveOutcome {
        changed: false,
        removed_entries: 0,
        preserved_entries: 0,
    };
    for layout in &layouts {
        let outcome = remove_in(agent, layout)?;
        total.changed |= outcome.changed;
        total.removed_entries += outcome.removed_entries;
        total.preserved_entries += outcome.preserved_entries;
    }
    // Switching agy off also takes out the folder trust Hide added (D-08)
    // and Hide AI's folder for agy (D-09).
    if agent == GuidanceAgent::Antigravity {
        let forgotten = crate::folder_trust::forget_all(
            crate::folder_trust::Agent::Antigravity,
            home,
            crate::folder_trust::Tools::none(),
        )?;
        let removed = crate::antigravity_trust::remove_read_only(home)?;
        total.changed |= forgotten > 0 || removed;
        total.removed_entries += forgotten;
    }
    Ok(total)
}

fn remove_in(agent: GuidanceAgent, layout: &Layout) -> Result<RemoveOutcome, InstallFailure> {
    let Some(mut document) = read_document(&layout.path)? else {
        return Ok(RemoveOutcome {
            changed: false,
            removed_entries: 0,
            preserved_entries: 0,
        });
    };
    let mut removed = 0;
    let mut preserved = 0;
    let keys = agent
        .slots()
        .iter()
        .map(|slot| slot.key)
        .chain(agent.retired_keys().iter().copied());
    for key in keys {
        let Some(list) = entries(&mut document, layout, key, false)? else {
            continue;
        };
        let taken = strip_owned(list);
        removed += taken;
        preserved += list.len();
        if taken != 0 && list.is_empty() {
            drop_empty(&mut document, layout, key);
        }
    }
    if removed == 0 {
        return Ok(RemoveOutcome {
            changed: false,
            removed_entries: 0,
            preserved_entries: preserved,
        });
    }
    if layout.own_file && only_scaffolding(&document, layout) {
        std::fs::remove_file(&layout.path).map_err(|error| InstallFailure::NotWritable {
            path: layout.path.display().to_string(),
            detail: error.to_string(),
        })?;
    } else {
        write_document(&layout.path, &document)?;
    }
    Ok(RemoveOutcome {
        changed: true,
        removed_entries: removed,
        preserved_entries: preserved,
    })
}

/// Drops the event key Hide emptied, so the operator's file does not keep an
/// empty list Hide made.
fn drop_empty(document: &mut Value, layout: &Layout, key: &str) {
    let Some(root) = document.as_object_mut() else {
        return;
    };
    match &layout.shape {
        Shape::EventMap { under_hooks_key } => {
            if *under_hooks_key {
                if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
                    hooks.shift_remove(key);
                    // An empty `hooks` object is Hide's scaffolding too.
                    if hooks.is_empty() {
                        root.shift_remove("hooks");
                    }
                }
            } else {
                root.shift_remove(key);
            }
        }
        Shape::Copilot | Shape::Cursor => {
            if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
                hooks.shift_remove(key);
            }
        }
        Shape::Kiro => {
            root.shift_remove("hooks");
        }
        Shape::NamedBlock { block } => {
            if let Some(events) = root.get_mut(*block).and_then(Value::as_object_mut) {
                events.shift_remove(key);
                // A block Hide emptied is Hide's scaffolding too.
                if events.is_empty() {
                    root.shift_remove(*block);
                }
            }
        }
    }
}

/// Whether nothing but the scaffolding Hide wrote is left in an own file.
fn only_scaffolding(document: &Value, layout: &Layout) -> bool {
    let Some(root) = document.as_object() else {
        return false;
    };
    let hooks_empty = match &layout.shape {
        Shape::Copilot | Shape::Cursor => root
            .get("hooks")
            .and_then(Value::as_object)
            .is_none_or(Map::is_empty),
        Shape::Kiro => !root.contains_key("hooks"),
        Shape::EventMap { .. } | Shape::NamedBlock { .. } => root.is_empty(),
    };
    hooks_empty && root.keys().all(|key| key == "version" || key == "hooks")
}

#[cfg(test)]
mod tests;
