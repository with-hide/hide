//! Which agent runtimes Hide instruments, and the vocabulary it writes.

use std::path::{Path, PathBuf};

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
pub const HOOK_VERSION: u32 = 8;

/// The installed Claude Code and Codex command version, independent of the
/// pane reporting protocol. Version 10 adds passive Post observation slots.
pub const HOOK_ENTRY_VERSION: u32 = 10;

/// The one sentence SessionStart adds to either agent runtime inside Hide.
/// Both runtimes accept the same `hookSpecificOutput.additionalContext`
/// envelope, while the runtime argument remains explicit in the installed
/// command so a future protocol difference has one dispatch point.
pub const PURPOSE_CONTEXT: &str = "Herdr provides direct terminal input and control. Call it as `\"${HERDR_BIN_PATH:-herdr}\"` (PowerShell: `& $env:HERDR_BIN_PATH`), the Herdr that serves this pane, because a `herdr` found on PATH may be an older one its server refuses. `\"${HERDR_BIN_PATH:-herdr}\" agent prompt` submits input directly; Hide may show a passive observation, but this creates no Hide letter, watch, or parent relationship. Use Hide letters for stored delivery, replies and retry management. When you create a worktree, set its one-line purpose in 40 characters or fewer by running `\"${HERDR_BIN_PATH:-herdr}\" workspace report-metadata <workspace> --source <you> --token purpose=\"…\"`. For work you will supervise, delegate with `hide agent spawn --parent here …`; this records responsibility and starts an automatic watch. For independent work the operator will handle, hand it off with `hide agent spawn …` without --parent; it becomes an operator-owned root with no automatic watch. Both modes record you as origin and leave the current screen and keyboard focus unchanged. From the machine that runs Hide, start the agent on a connected device by adding `--machine <device id>` (the device_id `hide workspace info` shows, the machine `hide agent list` shows for agents there); `--repo` and `--path` are then that device's paths. An agent on a device cannot use `--machine`: it is refused with `machine_not_permitted`.";

pub fn hook_stdout(runtime: AgentRuntime, event: HookEvent) -> Option<String> {
    hook_stdout_with_context(runtime, event, None)
}

pub fn hook_stdout_with_context(
    runtime: AgentRuntime,
    event: HookEvent,
    memory_context: Option<&str>,
) -> Option<String> {
    let additional_context = match event {
        HookEvent::SessionStart => match memory_context {
            Some(memory) if !memory.is_empty() => format!("{PURPOSE_CONTEXT}\n\n{memory}"),
            _ => PURPOSE_CONTEXT.to_owned(),
        },
        HookEvent::UserPromptSubmit => memory_context.filter(|value| !value.is_empty())?.to_owned(),
        HookEvent::SubagentStart
        | HookEvent::SubagentStop
        | HookEvent::Stop
        | HookEvent::PreToolUse
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure => return None,
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
/// count, so it does not decide this. (That Claude Code's spawn guard stays
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

    /// Codex reports nonzero outcomes in PostToolUse. It has no declared
    /// PostToolUseFailure event, so never write one into its configuration.
    pub fn events(self) -> impl Iterator<Item = HookEvent> {
        HookEvent::ALL.into_iter().filter(move |event| {
            self == Self::ClaudeCode || *event != HookEvent::PostToolUseFailure
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

    /// The directory whose presence means this runtime is set up on this Mac.
    ///
    /// Hide installs nothing for a runtime the operator does not use: writing
    /// `~/.codex/hooks.json` on a machine with no Codex would create a file
    /// for a tool that is not there.
    pub fn home_directory(self, home: &Path) -> PathBuf {
        match self {
            Self::ClaudeCode => home.join(".claude"),
            Self::Codex => home.join(".codex"),
        }
    }

    /// The file whose `hooks` object Hide appends to.
    pub fn config_path(self, home: &Path) -> PathBuf {
        match self {
            Self::ClaudeCode => self.home_directory(home).join("settings.json"),
            Self::Codex => self.home_directory(home).join("hooks.json"),
        }
    }

    /// The file that keeps this runtime's folder trust, found from the same
    /// folder as [`Self::config_path`], so the trust Hide records and its
    /// hooks are always read from one place (`crate::folder_trust`).
    ///
    /// Claude Code's is its global config, `~/.claude.json`, or the older
    /// `.config.json` in its folder while that file has content, which Claude
    /// Code then reads instead (its `getGlobalClaudeFile`); Codex's is the
    /// `config.toml` in its folder, which only Codex writes.
    pub fn trust_path(self, home: &Path) -> PathBuf {
        match self {
            Self::ClaudeCode => {
                let legacy = self.home_directory(home).join(".config.json");
                if std::fs::metadata(&legacy).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
                    return legacy;
                }
                home.join(".claude.json")
            }
            Self::Codex => self.home_directory(home).join("config.toml"),
        }
    }
}

/// The hook events Hide registers.
///
/// These are exactly the events both shipped runtimes declare, verified
/// against the installed Claude Code binary and the Codex configuration on
/// 2026-09-09 (PRD D-09), and `PreToolUse`, observed on Claude Code 2.1.292
/// and codex-cli 0.160.0 on 2026-10-07 (PRD herdr-spawn-guard D-06). An event
/// only one runtime has is not registered: a key a runtime does not know is a
/// key its settings validator may reject, and the sweep at `Stop` already
/// covers what `SessionEnd` would.
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
    /// A tool call is about to run. Hide's entry selects the shell tool by its
    /// matcher and refuses only a call that starts an agent through Herdr
    /// directly (`crate::spawn_guard`); every other call passes untouched.
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
}

impl HookEvent {
    pub const ALL: [HookEvent; 8] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::SubagentStart,
        Self::SubagentStop,
        Self::Stop,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PostToolUseFailure,
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
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.name() == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let context = format!("{PURPOSE_CONTEXT} 메모리 \u{1F600}");
        let json = serde_json::json!({ "hookSpecificOutput": { "additionalContext": context } })
            .to_string();
        let ascii = ascii_json(&json);
        assert!(ascii.is_ascii(), "{ascii}");
        assert!(ascii.contains(r#"purpose=\"\u2026\""#));
        assert!(ascii.contains(r"\ud83d\ude00"), "outside the BMP as a pair");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&ascii).unwrap(),
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
    }

    #[test]
    fn both_runtimes_emit_the_purpose_instruction_only_at_session_start() {
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
                PURPOSE_CONTEXT
            );
            assert!(PURPOSE_CONTEXT.contains("40 characters or fewer"));
            assert!(PURPOSE_CONTEXT.contains(
                "`\"${HERDR_BIN_PATH:-herdr}\" workspace report-metadata <workspace> --source <you> --token purpose=\"…\"`"
            ));
            assert!(!PURPOSE_CONTEXT.contains("`herdr "));
            assert!(PURPOSE_CONTEXT.contains("`hide agent spawn --parent here"));
            assert!(PURPOSE_CONTEXT.contains("`hide agent spawn …` without --parent"));
            assert!(PURPOSE_CONTEXT.contains("operator-owned root with no automatic watch"));
            assert!(PURPOSE_CONTEXT.contains("record you as origin"));
            assert!(PURPOSE_CONTEXT.contains("`--machine <device id>`"));
            assert!(PURPOSE_CONTEXT.contains("`hide agent list`"));
            assert!(PURPOSE_CONTEXT.contains("From the machine that runs Hide"));
            assert!(PURPOSE_CONTEXT.contains("`machine_not_permitted`"));
            for event in [
                HookEvent::SubagentStart,
                HookEvent::SubagentStop,
                HookEvent::Stop,
            ] {
                assert_eq!(hook_stdout(runtime, event), None);
            }
        }
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
