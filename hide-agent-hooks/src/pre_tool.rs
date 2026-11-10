//! What every `PreToolUse` guard shares: the shell call a runtime's payload
//! carries, the refusal envelope Claude Code and Codex read, and the one
//! question to the daemon, whether the caller's pane sits in a checkout Hide
//! has registered. The guards themselves are `crate::guardrails`,
//! `crate::plan_record`, `crate::global_guard` and `crate::opener_guard`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use hide_agent_adapter::HookDialect;
use hide_shell::Syntax;
use serde::Deserialize;

use crate::hook_request::HookCommand;

/// The shell tool's call, as a runtime's `PreToolUse` payload carries it.
#[derive(Debug, Eq, PartialEq)]
pub struct Call {
    pub command: String,
    pub cwd: Option<PathBuf>,
    /// The syntax of the shell that runs `command`, which every guard reads
    /// it in (`hide_session::call_shell::syntax`).
    pub syntax: Syntax,
}

/// Claude Code's, Codex's and Cursor's payload: snake_case keys.
#[derive(Deserialize)]
struct Payload {
    tool_name: Option<String>,
    tool_input: Option<ToolInput>,
    cwd: Option<String>,
}

/// Grok's payload: camelCase keys (Grok's hooks guide, Writing Hook Scripts).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokPayload {
    tool_name: Option<String>,
    tool_input: Option<ToolInput>,
    cwd: Option<String>,
}

/// agy's payload: camelCase keys, the call under `toolCall` with its
/// arguments in agy's own capitalized names (agy 1.3.6, measured).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AntigravityPayload {
    tool_call: Option<AntigravityCall>,
}

#[derive(Deserialize)]
struct AntigravityCall {
    name: Option<String>,
    args: Option<AntigravityArgs>,
}

#[derive(Deserialize)]
struct AntigravityArgs {
    #[serde(rename = "CommandLine")]
    command_line: Option<serde_json::Value>,
    #[serde(rename = "Cwd")]
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct ToolInput {
    command: Option<serde_json::Value>,
    /// Cursor's shell call names the folder it runs in.
    working_directory: Option<String>,
}

/// The shell call in a `PreToolUse` payload, in `dialect`'s shape, with the
/// syntax of the shell that runs it: the shell tool is `Bash` or `PowerShell`
/// in Claude Code, `Bash` in Codex, `run_terminal_command` in Grok
/// (which a hook may also see as its alias `Bash`), `Shell` in Cursor,
/// whose call carries its own `working_directory`, and `run_command` in agy,
/// whose call carries `CommandLine` and `Cwd`. A truncated or unreadable
/// payload, another tool and a command that is not one string yield nothing,
/// so the call runs.
pub fn read_call(payload: &[u8], truncated: bool, dialect: HookDialect) -> Option<Call> {
    if truncated {
        return None;
    }
    let (tool, input, cwd) = match dialect {
        // OpenCode's plugin and Pi's and omp's extension hand their helper the
        // command itself; no hook payload of theirs reaches this reader.
        HookDialect::OpenCode | HookDialect::Pi | HookDialect::Omp => return None,
        HookDialect::Antigravity => {
            let call = serde_json::from_slice::<AntigravityPayload>(payload)
                .ok()?
                .tool_call?;
            if call.name.as_deref() != Some("run_command") {
                return None;
            }
            let args = call.args?;
            let serde_json::Value::String(command) = args.command_line? else {
                return None;
            };
            return Some(Call {
                command,
                cwd: args.cwd.filter(|cwd| !cwd.is_empty()).map(PathBuf::from),
                syntax: hide_session::call_shell::syntax(dialect, "run_command"),
            });
        }
        HookDialect::Grok => {
            let payload: GrokPayload = serde_json::from_slice(payload).ok()?;
            (payload.tool_name, payload.tool_input, payload.cwd)
        }
        HookDialect::ClaudeCode | HookDialect::Codex | HookDialect::Cursor => {
            let payload: Payload = serde_json::from_slice(payload).ok()?;
            (payload.tool_name, payload.tool_input, payload.cwd)
        }
    };
    let shell = match dialect {
        HookDialect::ClaudeCode => &["Bash", "PowerShell"][..],
        HookDialect::Codex | HookDialect::OpenCode | HookDialect::Pi | HookDialect::Omp => {
            &["Bash"][..]
        }
        HookDialect::Grok => &["run_terminal_command", "Bash"][..],
        HookDialect::Cursor => &["Shell"][..],
        HookDialect::Antigravity => &["run_command"][..],
    };
    let tool = tool.filter(|tool| shell.contains(&tool.as_str()))?;
    let input = input?;
    let serde_json::Value::String(command) = input.command? else {
        return None;
    };
    let cwd = match dialect {
        HookDialect::Cursor => input
            .working_directory
            .filter(|cwd| !cwd.is_empty())
            .or(cwd),
        _ => cwd,
    };
    Some(Call {
        command,
        cwd: cwd.filter(|cwd| !cwd.is_empty()).map(PathBuf::from),
        syntax: hide_session::call_shell::syntax(dialect, &tool),
    })
}

/// The `PreToolUse` answer that refuses a call, for Claude Code and Codex: the
/// same envelope, observed on Claude Code 2.1.292 and codex-cli 0.160.0, with
/// the reason shown to the agent.
pub fn deny_output(reason: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}

// --- Asking the daemon -----------------------------------------------------------

/// What the daemon says about the caller's pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Registration {
    /// The pane sits in a checkout Hide has registered.
    Registered,
    /// The daemon answered and does not know this pane's checkout.
    NotRegistered,
    /// The daemon could not be asked: no daemon, no answer in time, or a
    /// reason that is not its "not Hide's" answer.
    Unreachable(&'static str),
}

/// Asks the daemon whether the caller sits in a registered checkout, by the
/// hook's own request (`HookCommand::Registration`), the proof `hide
/// workspace bootstrap` gives: it needs no open window and starts no
/// program. A refusal with a reason is the daemon answering; a missing
/// daemon, a timeout and an unreadable answer are not.
pub fn registration(deadline: Instant) -> Registration {
    let answer = match crate::delivery::answer_of(HookCommand::Registration {}, deadline).0 {
        Ok(answer) => answer,
        Err(failure) => return Registration::Unreachable(failure.code),
    };
    if answer["ok"] == true {
        return Registration::Registered;
    }
    // Only the daemon's own "this caller is not in a Hide checkout" answers
    // mean the call is not Hide's to guide. Every other word, an empty one and
    // a reason a later build adds included, is a daemon that could not be
    // asked, which lets the call run and leaves a diagnostic.
    if matches!(
        answer["reason"].as_str(),
        Some(
            "checkout_not_registered"
                | "caller_unavailable"
                | "pane_not_connected"
                | "pane_unavailable"
                | "pane_changed"
                | "caller_not_in_pane"
        )
    ) {
        Registration::NotRegistered
    } else {
        Registration::Unreachable("daemon")
    }
}

/// The daemon could not be asked, so the call ran. One diagnostic per window
/// per ten minutes, to the log file only (PRD B6; principle 10: nothing here is
/// something the operator or the agent can act on).
pub fn unreachable(home: &Path, runtime: &str, cause: &'static str) {
    if crate::delivery::claim(home, "guard") {
        let line = serde_json::json!({
            "component": "pre_tool",
            "kind": "daemon.unreachable",
            "runtime": runtime,
            "cause": cause,
            "at_unix_ms": crate::hook_log::now_ms(),
        })
        .to_string();
        crate::hook_log::append(home, &line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_of_a_shell_call_is_read_and_nothing_else() {
        let call = |json: &str| read_call(json.as_bytes(), false, HookDialect::ClaudeCode);
        assert_eq!(
            call(r#"{"tool_name":"Bash","cwd":"/r","tool_input":{"command":"ls"}}"#),
            Some(Call {
                command: "ls".into(),
                cwd: Some(PathBuf::from("/r")),
                syntax: Syntax::Posix,
            })
        );
        assert_eq!(
            call(r#"{"tool_name":"Edit","tool_input":{"command":"ls"}}"#),
            None
        );
        assert_eq!(
            call(r#"{"tool_name":"Bash","tool_input":{"command":["ls"]}}"#),
            None
        );
        assert_eq!(call(r#"{"tool_name":"Bash","tool_input":{}}"#), None);
        assert_eq!(call("not json"), None);
        assert_eq!(
            read_call(
                br#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
                true,
                HookDialect::ClaudeCode
            ),
            None
        );
    }

    #[test]
    fn grok_and_cursor_shell_calls_are_read_in_their_own_documented_shapes() {
        // Grok's hooks guide, Writing Hook Scripts: camelCase keys.
        let grok = br#"{"hookEventName":"pre_tool_use","hook_event_name":"PreToolUse","sessionId":"abc-123",
            "cwd":"/work/project","toolName":"run_terminal_command","toolInput":{"command":"npm test"},
            "toolUseId":"t","toolInputTruncated":false}"#;
        assert_eq!(
            read_call(grok, false, HookDialect::Grok),
            Some(Call {
                command: "npm test".into(),
                cwd: Some(PathBuf::from("/work/project")),
                syntax: hide_session::call_shell::syntax(HookDialect::Grok, "run_terminal_command"),
            })
        );
        assert_eq!(
            read_call(
                br#"{"toolName":"Bash","toolInput":{"command":"ls"}}"#,
                false,
                HookDialect::Grok
            )
            .map(|call| call.command),
            Some("ls".to_owned())
        );
        assert_eq!(
            read_call(
                br#"{"toolName":"read_file","toolInput":{"command":"ls"}}"#,
                false,
                HookDialect::Grok
            ),
            None
        );
        // Grok's keys are not Claude Code's.
        assert_eq!(read_call(grok, false, HookDialect::ClaudeCode), None);

        // Cursor's hooks page, preToolUse: the shell call carries its folder.
        let cursor = br#"{"conversation_id":"c","generation_id":"g","hook_event_name":"preToolUse",
            "tool_name":"Shell","tool_input":{"command":"npm install","working_directory":"/project/web"},
            "tool_use_id":"abc123","cwd":"/project"}"#;
        assert_eq!(
            read_call(cursor, false, HookDialect::Cursor),
            Some(Call {
                command: "npm install".into(),
                cwd: Some(PathBuf::from("/project/web")),
                syntax: hide_session::call_shell::syntax(HookDialect::Cursor, "Shell"),
            })
        );
        assert_eq!(
            read_call(
                br#"{"tool_name":"Shell","tool_input":{"command":"ls"},"cwd":"/p"}"#,
                false,
                HookDialect::Cursor
            )
            .and_then(|call| call.cwd),
            Some(PathBuf::from("/p"))
        );
        assert_eq!(
            read_call(
                br#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
                false,
                HookDialect::Cursor
            ),
            None
        );
    }

    #[test]
    fn claude_codes_powershell_tool_is_a_shell_call_read_as_powershell() {
        let call = read_call(
            br#"{"tool_name":"PowerShell","cwd":"C:/r","tool_input":{"command":"ii .\\out.png"}}"#,
            false,
            HookDialect::ClaudeCode,
        )
        .unwrap();
        assert_eq!(call.command, r"ii .\out.png");
        assert_eq!(call.syntax, Syntax::PowerShell);
        // Codex has no tool of that name.
        assert_eq!(
            read_call(
                br#"{"tool_name":"PowerShell","tool_input":{"command":"ls"}}"#,
                false,
                HookDialect::Codex
            ),
            None
        );
    }

    /// A Codex call is read in the shell Codex runs it in on this system
    /// (`hide_session::call_shell`, which holds the whole table).
    #[test]
    fn a_codex_call_carries_the_syntax_of_its_system() {
        let codex = read_call(
            br#"{"tool_name":"Bash","tool_input":{"command":"ii .\\out.png"}}"#,
            false,
            HookDialect::Codex,
        )
        .unwrap();
        assert_eq!(
            codex.syntax,
            if cfg!(windows) {
                Syntax::PowerShell
            } else {
                Syntax::Posix
            }
        );
    }
}
