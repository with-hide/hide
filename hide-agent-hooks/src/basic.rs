//! What Grok's and Cursor's own hook events say, and what Hide answers them
//! (PRD grok-cursor-hooks).
//!
//! Both agents document a refusal of a shell call and the start and end of a
//! subagent, in their own payload and answer shapes. This module reads those
//! payloads into the facts the spawn guard and the pane count need and
//! writes the answers; the guard itself (`crate::spawn_guard`) and the count
//! (`crate::counters`) are the same ones Claude Code's and Codex's hooks use.
//!
//! | | Grok | Cursor |
//! | --- | --- | --- |
//! | Shell call | `toolName` `run_terminal_command` (`Bash` in a matcher), `toolInput.command`, `cwd` | `tool_name` `Shell`, `tool_input.command`, `tool_input.working_directory`, `cwd` |
//! | Refusal | Claude Code's `hookSpecificOutput.permissionDecision` | `{"permission": "deny", "user_message": ..., "agent_message": ..., "additional_context": ...}` |
//! | Nothing to refuse | no output | `{"permission": "allow"}`, on every path (D-04) |
//! | Turn end | `Stop`: `subagentType` inside a subagent, `backgroundTasks` still running | `stop` |
//!
//! Cursor's `preToolUse` and `subagentStart` are permission hooks: an answer
//! that is not valid JSON of their schema blocks the action even when the
//! hook exits zero, so for them every path that does not refuse prints
//! [`CURSOR_ALLOW`], including a helper that fails or runs out of time.

use serde::Deserialize;
use serde_json::Value;

use crate::runtime::HookEvent;

/// Cursor's answer that lets the action proceed.
pub const CURSOR_ALLOW: &str = r#"{"permission":"allow"}"#;

/// Whether Cursor reads `event`'s answer as a permission, so that anything
/// but a valid answer refuses the action.
pub fn cursor_permission_event(event: HookEvent) -> bool {
    matches!(event, HookEvent::PreToolUse | HookEvent::SubagentStart)
}

/// Cursor's refusal of a tool call: the reason goes back to the agent.
///
/// Cursor CLI reads a refusal on two paths. The local tool path (every shell
/// call) takes the refusal text from `user_message`, falling back to "<tool>
/// blocked by preToolUse hook", and hands `additional_context` to the agent as
/// hook context; the path the agent server asks for takes `agent_message`. The
/// reason is written to all three so it reaches the agent on either path.
pub fn cursor_deny(reason: &str) -> String {
    serde_json::json!({
        "permission": "deny",
        "user_message": reason,
        "agent_message": reason,
        "additional_context": reason,
    })
    .to_string()
}

/// The session a Grok or Cursor tool event names (`sessionId`,
/// `conversation_id`), which a letter pull at that event is made for, or
/// `None` from a truncated or unreadable payload.
pub fn session(
    payload: &[u8],
    truncated: bool,
    dialect: hide_agent_adapter::HookDialect,
) -> Result<Option<String>, crate::delivery::Failure> {
    #[derive(Deserialize)]
    struct Cursor {
        conversation_id: Option<String>,
    }
    if truncated {
        return Ok(None);
    }
    let session = match dialect {
        hide_agent_adapter::HookDialect::Cursor => serde_json::from_slice::<Cursor>(payload)
            .ok()
            .and_then(|input| input.conversation_id),
        _ => return crate::delivery::read_session(payload, truncated),
    };
    Ok(session.filter(|id| crate::delivery::valid_session(id)))
}

/// The answer that lets a tool call run and hands the agent `context` with
/// its result: Grok reads Claude Code's `additionalContext` (its guide says
/// a `PreToolUse` one travels with the batch's tool results), Cursor its own
/// `additional_context` beside the permission.
pub fn context_answer(dialect: hide_agent_adapter::HookDialect, context: &str) -> String {
    match dialect {
        hide_agent_adapter::HookDialect::Cursor => serde_json::json!({
            "permission": "allow",
            "additional_context": context,
        })
        .to_string(),
        _ => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": context,
            }
        })
        .to_string(),
    }
}

/// What a Grok `Stop` says about the pane's subagents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrokStop {
    /// The stop is a subagent's own (`subagentType`) or the payload could not
    /// be read: the pane's count is left as it is.
    Ignore,
    /// The main session's turn ended with this many background subagents
    /// still running.
    Running { running: u32, hook_active: bool },
}

/// Reads a Grok `Stop` payload. Every `backgroundTasks` entry is in flight
/// (the list is empty when nothing is), and only the `subagent` ones are the
/// pane's subagents; shells and monitors are not.
pub fn grok_stop(payload: &[u8], truncated: bool) -> GrokStop {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Stop {
        subagent_type: Option<Value>,
        #[serde(default)]
        stop_hook_active: bool,
        #[serde(default)]
        background_tasks: Vec<Task>,
    }
    #[derive(Deserialize)]
    struct Task {
        #[serde(rename = "type")]
        kind: Option<String>,
    }
    if truncated {
        return GrokStop::Ignore;
    }
    let Ok(stop) = serde_json::from_slice::<Stop>(payload) else {
        return GrokStop::Ignore;
    };
    // An empty or null type is no subagent's: the main session's stop.
    if stop
        .subagent_type
        .is_some_and(|kind| !kind.is_null() && kind.as_str() != Some(""))
    {
        return GrokStop::Ignore;
    }
    let running = stop
        .background_tasks
        .iter()
        .filter(|task| task.kind.as_deref() == Some("subagent"))
        .count();
    GrokStop::Running {
        running: u32::try_from(running).unwrap_or(u32::MAX),
        hook_active: stop.stop_hook_active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_reads_its_permission_answers_as_valid_json_of_its_schema() {
        let allow: Value = serde_json::from_str(CURSOR_ALLOW).unwrap();
        assert_eq!(allow, serde_json::json!({ "permission": "allow" }));
        let deny: Value = serde_json::from_str(&cursor_deny("use hide agent spawn")).unwrap();
        assert_eq!(
            deny,
            serde_json::json!({
                "permission": "deny",
                "user_message": "use hide agent spawn",
                "agent_message": "use hide agent spawn",
                "additional_context": "use hide agent spawn",
            })
        );
        assert!(cursor_permission_event(HookEvent::PreToolUse));
        assert!(cursor_permission_event(HookEvent::SubagentStart));
        for event in [
            HookEvent::SessionStart,
            HookEvent::SubagentStop,
            HookEvent::Stop,
        ] {
            assert!(!cursor_permission_event(event));
        }
    }

    #[test]
    fn a_grok_stop_counts_only_the_background_subagents_of_the_main_session() {
        let stop =
            br#"{"hook_event_name":"Stop","sessionId":"s","reason":"end_turn","backgroundTasks":[
            {"id":"a","type":"subagent","status":"running","agentType":"explore"},
            {"id":"b","type":"shell","status":"running","command":"npm run dev"},
            {"id":"c","type":"subagent","status":"running"},
            {"id":"d","type":"monitor","status":"running"}],"sessionCrons":[]}"#;
        assert_eq!(
            grok_stop(stop, false),
            GrokStop::Running {
                running: 2,
                hook_active: false
            }
        );
        assert_eq!(
            grok_stop(br#"{"sessionId":"s","backgroundTasks":[]}"#, false),
            GrokStop::Running {
                running: 0,
                hook_active: false
            }
        );
        assert_eq!(
            grok_stop(br#"{"sessionId":"s"}"#, false),
            GrokStop::Running {
                running: 0,
                hook_active: false
            }
        );
        let child = br#"{"sessionId":"child","subagentType":"explore","backgroundTasks":[]}"#;
        assert_eq!(grok_stop(child, false), GrokStop::Ignore);
        assert_eq!(
            grok_stop(
                br#"{"sessionId":"s","subagentType":"","backgroundTasks":[]}"#,
                false
            ),
            GrokStop::Running {
                running: 0,
                hook_active: false
            }
        );
        assert_eq!(grok_stop(b"not json", false), GrokStop::Ignore);
        assert_eq!(grok_stop(stop, true), GrokStop::Ignore);
    }
}
