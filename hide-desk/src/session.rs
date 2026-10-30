//! The ACP session on a Desk process: open or resume it in its ask mode,
//! prompt, cancel, choose a model or effort, and read what the agent sends
//! back (ACP v1).

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::adapter::{AdapterId, AskMode};
use crate::rpc::{CallError, Connection};

/// How long a control request may take: a first `session/new` starts the
/// agent's CLI, and a `session/load` replays the conversation.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(90);
/// ACP's error code for a request that needs the agent signed in.
pub const AUTH_REQUIRED: i64 = -32000;

/// One option the session lets the operator choose (`configOptions`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Choice {
    pub config_id: String,
    pub current: Option<String>,
    pub values: Vec<(String, String)>,
}

/// The model and effort choices the session offers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Choices {
    pub model: Option<Choice>,
    pub effort: Option<Choice>,
}

/// An open session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opened {
    pub session_id: String,
    /// The conversation asked for was loaded; `false` is a new session.
    pub resumed: bool,
    /// A resume was asked for and the agent could not load it.
    pub resume_failed: bool,
    pub choices: Choices,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenError {
    /// The agent needs the operator signed in (B6).
    Login,
    /// The ask mode could not be confirmed: the session is not used (D-53).
    AskModeUnconfirmed(String),
    Failed(String),
}

impl OpenError {
    fn from_call(step: &str, error: CallError) -> Self {
        match error {
            CallError::Rejected(rejected) if rejected.code == AUTH_REQUIRED => Self::Login,
            other => Self::Failed(format!("{step}: {other}")),
        }
    }
}

/// Opens a session in `cwd`: loads `resume` when the agent can, else starts
/// a new one, then puts it in the adapter's ask mode. Pi's mode is proven by
/// its extension's marker, which `adapter::pi_extension_loaded` checks once the session exists.
pub fn open(
    connection: &Connection,
    adapter: AdapterId,
    cwd: &Path,
    resume: Option<&str>,
    pi_ready: &dyn Fn() -> Result<(), String>,
) -> Result<Opened, OpenError> {
    let initialized = connection
        .call(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "hide-desk", "version": env!("CARGO_PKG_VERSION")},
            }),
            CONTROL_TIMEOUT,
        )
        .map_err(|error| OpenError::from_call("initialize", error))?;
    let can_load = initialized
        .pointer("/agentCapabilities/loadSession")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut params = json!({"cwd": cwd, "mcpServers": []});
    if let Some(meta) = adapter.session_meta() {
        params["_meta"] = meta;
    }
    let mut resume_failed = resume.is_some() && !can_load;
    let mut opened = None;
    if let Some(session_id) = resume.filter(|_| can_load) {
        let mut load = params.clone();
        load["sessionId"] = json!(session_id);
        match connection.call("session/load", load, CONTROL_TIMEOUT) {
            Ok(answer) => opened = Some((session_id.to_owned(), answer, true)),
            Err(CallError::Rejected(rejected)) if rejected.code == AUTH_REQUIRED => {
                return Err(OpenError::Login);
            }
            Err(CallError::Rejected(_)) => resume_failed = true,
            Err(error) => return Err(OpenError::from_call("session/load", error)),
        }
    }
    let (session_id, answer, resumed) = match opened {
        Some(opened) => opened,
        None => {
            let answer = connection
                .call("session/new", params, CONTROL_TIMEOUT)
                .map_err(|error| OpenError::from_call("session/new", error))?;
            let session_id = answer
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| OpenError::Failed("session/new: no sessionId".into()))?
                .to_owned();
            (session_id, answer, false)
        }
    };
    match adapter.ask_mode() {
        AskMode::SetMode(mode) => {
            connection
                .call(
                    "session/set_mode",
                    json!({"sessionId": session_id, "modeId": mode}),
                    CONTROL_TIMEOUT,
                )
                .map_err(|error| {
                    OpenError::AskModeUnconfirmed(format!("set_mode {mode}: {error}"))
                })?;
        }
        AskMode::PiExtension => pi_ready().map_err(OpenError::AskModeUnconfirmed)?,
    }
    Ok(Opened {
        session_id,
        resumed,
        resume_failed,
        choices: choices(&answer),
    })
}

/// The model and effort choices in a session answer or a
/// `config_option_update`.
pub fn choices(answer: &Value) -> Choices {
    let options = answer
        .get("configOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let pick = |category: &str| {
        options
            .iter()
            .find(|option| option.get("category").and_then(Value::as_str) == Some(category))
            .and_then(|option| {
                Some(Choice {
                    config_id: option.get("id")?.as_str()?.to_owned(),
                    current: option
                        .get("currentValue")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    values: option
                        .get("options")?
                        .as_array()?
                        .iter()
                        .filter_map(|value| {
                            let id = value.get("value")?.as_str()?.to_owned();
                            let name = value
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_owned();
                            Some((id, name))
                        })
                        .take(MAX_CHOICES)
                        .collect(),
                })
            })
    };
    Choices {
        model: pick("model"),
        effort: pick("thought_level"),
    }
}

/// Choices kept per option; a longer list is cut.
pub const MAX_CHOICES: usize = 64;

/// Sends one prompt; its answer reaches the sink tagged `tag`.
pub fn prompt(
    connection: &Connection,
    session_id: &str,
    text: &str,
    tag: u64,
) -> Result<(), CallError> {
    connection.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
        tag,
    )
}

/// Ends the turn under way; its prompt answers `cancelled`.
pub fn cancel(connection: &Connection, session_id: &str) -> Result<(), CallError> {
    connection.notify("session/cancel", json!({"sessionId": session_id}))
}

pub fn set_choice(
    connection: &Connection,
    session_id: &str,
    config_id: &str,
    value: &str,
) -> Result<Choices, CallError> {
    connection
        .call(
            "session/set_config_option",
            json!({"sessionId": session_id, "configId": config_id, "value": value}),
            CONTROL_TIMEOUT,
        )
        .map(|answer| choices(&answer))
}

/// What one `session/update` says, as far as Desk uses it.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// Text of the agent's reply.
    Text(String),
    /// A tool call announced or changed; kept by id for the permission
    /// request that may follow.
    Tool {
        id: String,
        call: Value,
    },
    /// The session's mode moved.
    Mode(String),
    /// The model or effort lists or values moved.
    Choices(Choices),
    Other,
}

pub fn read_update(params: &Value) -> Update {
    let Some(update) = params.get("update") else {
        return Update::Other;
    };
    match update.get("sessionUpdate").and_then(Value::as_str) {
        Some("agent_message_chunk") => update
            .pointer("/content/text")
            .and_then(Value::as_str)
            .filter(|_| update.pointer("/content/type").and_then(Value::as_str) == Some("text"))
            .map_or(Update::Other, |text| Update::Text(text.to_owned())),
        Some("tool_call" | "tool_call_update") => update
            .get("toolCallId")
            .and_then(Value::as_str)
            .map_or(Update::Other, |id| Update::Tool {
                id: id.to_owned(),
                call: update.clone(),
            }),
        Some("current_mode_update") => update
            .get("currentModeId")
            .and_then(Value::as_str)
            .map_or(Update::Other, |mode| Update::Mode(mode.to_owned())),
        Some("config_option_update") => Update::Choices(choices(update)),
        _ => Update::Other,
    }
}

/// Merges a tool call update into what was announced for its id, so a
/// permission request reads the latest of each field.
pub fn merge_tool(announced: &mut Value, update: &Value) {
    if let (Value::Object(announced), Value::Object(update)) = (announced, update) {
        for (key, value) in update {
            if !value.is_null() {
                announced.insert(key.clone(), value.clone());
            }
        }
    }
}

/// The stop reason of a prompt's answer.
pub fn stop_reason(answer: &Value) -> Option<&str> {
    answer.get("stopReason").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_and_effort_choices_are_read_by_category() {
        // The shape claude-agent-acp 0.97 answered session/new with.
        let answer = json!({"configOptions": [
            {"id": "mode", "category": "mode", "type": "select", "currentValue": "auto",
             "options": [{"value": "default", "name": "Manual"}]},
            {"id": "model", "category": "model", "type": "select", "currentValue": "opus",
             "options": [{"value": "opus", "name": "Opus 5.5"}, {"value": "sonnet", "name": "Sonnet 5.5"}]},
            {"id": "effort", "category": "thought_level", "type": "select", "currentValue": "high",
             "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]}
        ]});
        let read = choices(&answer);
        let model = read.model.unwrap();
        assert_eq!(model.config_id, "model");
        assert_eq!(model.current.as_deref(), Some("opus"));
        assert_eq!(
            model.values[1],
            ("sonnet".to_owned(), "Sonnet 5.5".to_owned())
        );
        assert_eq!(read.effort.unwrap().config_id, "effort");
    }

    #[test]
    fn reply_text_and_tool_calls_are_told_apart() {
        assert_eq!(
            read_update(&json!({"update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "안녕"}}})),
            Update::Text("안녕".into())
        );
        assert!(matches!(
            read_update(&json!({"update": {"sessionUpdate": "tool_call", "toolCallId": "t1"}})),
            Update::Tool { .. }
        ));
        assert_eq!(
            read_update(&json!({"update": {"sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "hmm"}}})),
            Update::Other
        );
    }
}
