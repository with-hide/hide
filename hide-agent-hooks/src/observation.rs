//! Passive post-tool observations. Provider output is a claim about a
//! submission, never a receipt from the receiving model. The proof-bound
//! daemon admits the sender and independently resolves any graph endpoint.
//! Command text and result bodies live only in this bounded parse and never
//! cross that admission boundary.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{delivery, spawn_guard};

pub const ITEM_LIMIT: usize = 8 * 1024;
pub const COMMAND_LIMIT: usize = 64 * 1024;
pub const RESULT_LIMIT: usize = 128 * 1024;
/// Includes CLI startup, fresh proof, admission and owned-child cleanup.
pub const SEND_BUDGET: Duration = Duration::from_millis(1_200);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCorrelation {
    ToolCall,
    /// Antigravity has no documented independent tool call id.
    ConversationStep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    SubmittedObserved,
    OutcomeUnknown,
    ResultUnavailable,
    RejectedObserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationReason {
    Complete,
    MissingNativeReference,
    WaitSnapshot,
    CommandAmbiguous,
    RoutingUnverified,
    ResultMissing,
    ResultTruncated,
    ToolError,
    Interrupted,
    AsyncUnconfirmed,
    HerdrRejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationNativeSession {
    pub source: String,
    pub agent: String,
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerReportedTarget {
    pub pane_id: String,
    pub terminal_id: Option<String>,
    pub native_session: Option<ObservationNativeSession>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectObservation {
    pub runtime_id: String,
    pub session_id: String,
    pub call_id: String,
    pub correlation_kind: ObservationCorrelation,
    /// Used only for freshness, never as a sender or recipient proof.
    pub observed_at_unix_ms: u64,
    pub status: ObservationStatus,
    pub reason: ObservationReason,
    pub target: Option<ServerReportedTarget>,
}

/// An invocation's terminal native event, normalized by the provider
/// adapter. It contains borrowed transient content, not stored metadata.
pub struct ToolResult<'a> {
    pub runtime_id: &'a str,
    pub session_id: &'a str,
    pub call_id: &'a str,
    pub correlation_kind: ObservationCorrelation,
    pub command: &'a str,
    pub result: Option<&'a Value>,
    pub failed: bool,
    pub interrupted: bool,
    pub truncated: bool,
    pub asynchronous: bool,
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

/// Reports only a statically recognized Herdr prompt candidate. Ordinary
/// shell calls stop before parsing output or starting a transport child.
pub fn observe(
    call: ToolResult<'_>,
    observed_at_unix_ms: u64,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<DirectObservation> {
    if call.command.len() > COMMAND_LIMIT
        || !identifier(call.session_id)
        || !identifier(call.call_id)
        || !(call.command.contains("herdr") || call.command.contains("HERDR_BIN_PATH"))
    {
        return None;
    }
    let line = spawn_guard::shell_line(call.command.trim());
    let mut candidate = false;
    let mut canonical = line.plain && line.commands.len() == 1 && line.targets.is_empty();
    let mut routed = false;
    let mut wait = false;
    for words in &line.commands {
        let (words, foreign) = spawn_guard::command_words(words, env, true);
        let Some((program, rest)) = words.split_first() else {
            continue;
        };
        let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
        if !matches!(
            name,
            "herdr" | "herdr.exe" | "$HERDR_BIN_PATH" | "${HERDR_BIN_PATH}"
        ) {
            continue;
        }
        // Global routing flags can precede `agent`; they are deliberately
        // never interpreted as the sender pane's local server.
        let Some(agent_at) = rest.iter().position(|word| word == "agent") else {
            continue;
        };
        if rest.get(agent_at + 1).map(String::as_str) != Some("prompt") {
            continue;
        }
        candidate = true;
        routed |= foreign
            || agent_at != 0
            || words.len() != line.commands[0].len()
            || program.contains('$');
        let args = &rest[agent_at + 2..];
        // The pinned CLI's canonical no-wait syntax is target + text.
        wait |= args
            .iter()
            .any(|word| word == "--wait" || word.starts_with("--wait="));
        canonical &= args.len() == 2 && !args[0].starts_with('-');
    }
    if !candidate {
        return None;
    }
    let mut observation = DirectObservation {
        runtime_id: call.runtime_id.to_owned(),
        session_id: call.session_id.to_owned(),
        call_id: call.call_id.to_owned(),
        correlation_kind: call.correlation_kind,
        observed_at_unix_ms,
        status: ObservationStatus::OutcomeUnknown,
        reason: ObservationReason::ResultMissing,
        target: None,
    };
    observation.reason = if routed {
        ObservationReason::RoutingUnverified
    } else if wait {
        ObservationReason::WaitSnapshot
    } else if !canonical {
        ObservationReason::CommandAmbiguous
    } else if call.truncated {
        observation.status = ObservationStatus::ResultUnavailable;
        ObservationReason::ResultTruncated
    } else if call.interrupted {
        ObservationReason::Interrupted
    } else if call.asynchronous {
        ObservationReason::AsyncUnconfirmed
    } else {
        let raw_output = call
            .result
            .filter(|_| call.runtime_id == "grok")
            .and_then(grok_raw_output);
        let parsed = raw_output.as_ref().or(call.result).and_then(|result| {
            herdr_answer(result).or_else(|| {
                (call.runtime_id == "omp")
                    .then(|| omp_timed_answer(result))
                    .flatten()
            })
        });
        match parsed {
            Some(answer) if answer.get("error").is_some() => {
                // Only the exact server validation refusal proves no input.
                if answer["error"]["code"] == "agent_blocked" {
                    observation.status = ObservationStatus::RejectedObserved;
                    ObservationReason::HerdrRejected
                } else {
                    ObservationReason::ToolError
                }
            }
            Some(answer)
                if !call.failed
                    && answer.get("error").is_none()
                    && answer["result"]["type"] == "agent_prompted" =>
            {
                let agent = &answer["result"]["agent"];
                let pane = agent["pane_id"].as_str().filter(|value| identifier(value));
                if let Some(pane_id) = pane {
                    let native_session = agent
                        .get("agent_session")
                        .and_then(|value| {
                            serde_json::from_value::<ObservationNativeSession>(value.clone()).ok()
                        })
                        .filter(|native| {
                            identifier(&native.source)
                                && matches!(native.kind.as_str(), "id" | "path")
                                && identifier(&native.agent)
                                && identifier(&native.value)
                        });
                    let reason = if native_session.is_some() {
                        ObservationReason::Complete
                    } else {
                        ObservationReason::MissingNativeReference
                    };
                    observation.status = ObservationStatus::SubmittedObserved;
                    observation.target = Some(ServerReportedTarget {
                        pane_id: pane_id.to_owned(),
                        terminal_id: agent["terminal_id"]
                            .as_str()
                            .filter(|value| identifier(value))
                            .map(str::to_owned),
                        native_session,
                    });
                    reason
                } else {
                    observation.status = ObservationStatus::ResultUnavailable;
                    ObservationReason::ResultMissing
                }
            }
            _ if call.failed => ObservationReason::ToolError,
            _ => {
                observation.status = ObservationStatus::ResultUnavailable;
                ObservationReason::ResultMissing
            }
        }
    };
    Some(observation)
}

/// A provider's structured response or one complete text output. No scan
/// for a JSON substring: prefixes, suffixes and competing outputs remain
/// uncertain, including truncated model-facing Codex output.
fn herdr_answer(value: &Value) -> Option<Value> {
    if value["id"] == "cli:agent:prompt" {
        return (value.get("result").is_some() ^ value.get("error").is_some())
            .then(|| value.clone());
    }
    if let Some(text) = value.as_str() {
        if text.len() > RESULT_LIMIT {
            return None;
        }
        let answer = serde_json::from_str::<Value>(text.trim()).ok()?;
        return herdr_answer(&answer);
    }
    // Known provider envelopes; never recursively search arbitrary metadata.
    let outputs: Vec<_> = ["stdout", "stderr", "output", "structuredContent"]
        .iter()
        .filter_map(|field| value.get(*field))
        .filter(|output| !output.is_null() && output.as_str() != Some(""))
        .collect();
    if !outputs.is_empty() {
        return (outputs.len() == 1)
            .then(|| herdr_answer(outputs[0]))
            .flatten();
    }
    let parts = value
        .get("content")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())?;
    let mut text = String::new();
    for part in parts {
        if part["type"] != "text" {
            return None;
        }
        let next = part["text"].as_str()?;
        if text.len().saturating_add(next.len()) > RESULT_LIMIT {
            return None;
        }
        text.push_str(next);
    }
    let answer = serde_json::from_str::<Value>(text.trim()).ok()?;
    herdr_answer(&answer)
}

/// Grok's Bash result retains raw bytes separately from its decorated
/// model-facing output. Require the reported full byte count and valid UTF-8;
/// never strip an `exit:` prefix or read the output-file fallback.
fn grok_raw_output(response: &Value) -> Option<Value> {
    if response["type"] != "Bash" {
        return None;
    }
    let output = response["output"].as_array()?;
    if output.len() > RESULT_LIMIT || response["total_bytes"].as_u64()? != output.len() as u64 {
        return None;
    }
    let bytes = output
        .iter()
        .map(|value| u8::try_from(value.as_u64()?).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok().map(Value::String)
}

/// omp 18.7.0's completed Bash result has no separate raw stdout field.
/// Its tool appends two newlines and `formatWallTimeNotice` (toFixed(2))
/// to stdout. Accept only that pinned envelope around one complete JSON
/// answer; clipping notices, other text parts and arbitrary suffixes fail.
fn omp_timed_answer(response: &Value) -> Option<Value> {
    if response.as_object()?.len() != 1 {
        return None;
    }
    let parts = response["content"].as_array()?;
    if parts.len() != 1 || parts[0]["type"] != "text" {
        return None;
    }
    let text = parts[0]["text"].as_str()?;
    if text.len() > RESULT_LIMIT {
        return None;
    }
    let (output, notice) = text.rsplit_once("\n\nWall time: ")?;
    let seconds = notice.strip_suffix(" seconds")?;
    let (whole, fraction) = seconds.split_once('.')?;
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
        || fraction.len() != 2
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let answer = serde_json::from_str::<Value>(output.trim()).ok()?;
    (answer["id"] == "cli:agent:prompt")
        .then(|| herdr_answer(&answer))
        .flatten()
}

/// Native command-hook payloads. Post has the actual executed input, so no
/// Pre spool, target query, detached helper or transcript scan is needed.
pub fn native(
    runtime: &str,
    payload: &[u8],
    truncated: bool,
    failure: bool,
    now: u64,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<DirectObservation> {
    if truncated {
        return None;
    }
    let input: Value = serde_json::from_slice(payload).ok()?;
    let field = |snake: &str, camel: &str| input.get(snake).or_else(|| input.get(camel));
    // Conflicting aliases are not resolved by guessing which session owns it.
    for (snake, camel) in [
        ("session_id", "sessionId"),
        ("tool_use_id", "toolUseId"),
        ("tool_name", "toolName"),
        ("tool_input", "toolInput"),
        ("tool_response", "toolResult"),
        ("tool_input_truncated", "toolInputTruncated"),
        ("tool_result_truncated", "toolResultTruncated"),
    ] {
        if input
            .get(snake)
            .zip(input.get(camel))
            .is_some_and(|(a, b)| a != b)
        {
            return None;
        }
    }
    let agy = runtime == "antigravity";
    let tool = if agy {
        input["toolCall"]["name"].as_str()
    } else {
        field("tool_name", "toolName").and_then(Value::as_str)
    }?;
    if !matches!(
        tool,
        "Bash" | "PowerShell" | "run_terminal_command" | "run_command"
    ) {
        return None;
    }
    let tool_input = if agy {
        &input["toolCall"]["args"]
    } else {
        field("tool_input", "toolInput")?
    };
    let command = tool_input[if agy { "CommandLine" } else { "command" }].as_str()?;
    let session = if agy {
        input["conversationId"].as_str()
    } else {
        field("session_id", "sessionId").and_then(Value::as_str)
    }?;
    let step;
    let call_id = if agy {
        step = format!("step:{}:{tool}", input["stepIdx"].as_u64()?);
        step.as_str()
    } else {
        field("tool_use_id", "toolUseId").and_then(Value::as_str)?
    };
    let response = field("tool_response", "toolResult");
    let grok = runtime == "grok";
    observe(
        ToolResult {
            runtime_id: runtime,
            session_id: session,
            call_id,
            command,
            correlation_kind: if agy {
                ObservationCorrelation::ConversationStep
            } else {
                ObservationCorrelation::ToolCall
            },
            // Antigravity documents no result. An unexpected field cannot turn
            // its conversation-step correlation into an independent tool receipt.
            result: if agy { None } else { response },
            failed: failure
                || input.get("error").is_some_and(|v| !v.is_null())
                || response.is_some_and(|v| {
                    v["isError"] == true
                        || ["exitCode", "exit_code", "code"]
                            .iter()
                            .any(|key| v[*key].as_i64().is_some_and(|code| code != 0))
                }),
            interrupted: input["is_interrupt"] == true
                || (grok
                    && response.is_some_and(|v| {
                        v["timed_out"] == true
                            || v.get("signal").is_some_and(|signal| !signal.is_null())
                    })),
            truncated: field("tool_input_truncated", "toolInputTruncated")
                .is_some_and(|v| v == true)
                || field("tool_result_truncated", "toolResultTruncated").is_some_and(|v| v == true)
                || (grok && response.is_some_and(|v| v["truncated"] == true)),
            asynchronous: tool_input["run_in_background"] == true
                || (grok && input["isBackgrounded"] == true)
                || response.is_some_and(|v| {
                    v["status"] == "async_launched"
                        || v.get("shell_id").is_some()
                        || v.get("session_id").is_some()
                }),
        },
        now,
        env,
    )
}

/// No retries of the original operation. This owned CLI child only submits
/// metadata and inherits the same proof/route as existing hook requests.
pub fn send(home: &Path, observation: &DirectObservation, deadline: Instant) -> bool {
    let Ok(bytes) = serde_json::to_vec(observation) else {
        return false;
    };
    if bytes.len() > ITEM_LIMIT {
        delivery::diagnose(home, "capacity");
        return false;
    }
    match delivery::run_cli_with_input(&["observation", "--hook"], &bytes, deadline) {
        Ok(_) => true,
        Err(failure) => {
            delivery::diagnose_failure(home, &failure);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn answer() -> Value {
        json!({"id":"cli:agent:prompt","result":{"type":"agent_prompted","agent":{
            "pane_id":"w1:p2","terminal_id":"term-2","agent_session":{
                "source":"herdr:codex","agent":"codex","kind":"id","value":"recipient-native"
            }
        }}})
    }

    fn call<'a>(command: &'a str, result: &'a Value) -> ToolResult<'a> {
        ToolResult {
            runtime_id: "claude-code",
            session_id: "sender-native",
            call_id: "tool-1",
            correlation_kind: ObservationCorrelation::ToolCall,
            command,
            result: Some(result),
            failed: false,
            interrupted: false,
            truncated: false,
            asynchronous: false,
        }
    }

    #[test]
    fn canonical_complete_output_keeps_only_metadata() {
        let result = json!({"stdout":answer().to_string(),"stderr":""});
        let fact = observe(
            call("herdr agent prompt worker 'private prompt body'", &result),
            42,
            &|_| None,
        )
        .unwrap();
        assert_eq!(
            (fact.status, fact.reason),
            (
                ObservationStatus::SubmittedObserved,
                ObservationReason::Complete
            )
        );
        let target = fact.target.as_ref().unwrap();
        assert_eq!(
            target.native_session.as_ref().unwrap().value,
            "recipient-native"
        );
        assert_eq!(target.terminal_id.as_deref(), Some("term-2"));
        let wire = serde_json::to_string(&fact).unwrap();
        assert!(!wire.contains("private prompt body"));
        assert!(!wire.contains("stdout"));
        assert_eq!(fact.observed_at_unix_ms, 42);
    }

    #[test]
    fn uncertain_execution_never_creates_a_recipient() {
        let result = answer();
        for (command, reason) in [
            (
                "herdr agent prompt worker hi --wait",
                ObservationReason::WaitSnapshot,
            ),
            (
                "herdr agent prompt worker hi > result.json",
                ObservationReason::CommandAmbiguous,
            ),
            (
                "herdr agent prompt worker hi && echo ok",
                ObservationReason::CommandAmbiguous,
            ),
            (
                "herdr agent prompt worker hi &",
                ObservationReason::CommandAmbiguous,
            ),
            (
                "herdr agent prompt worker $BODY",
                ObservationReason::CommandAmbiguous,
            ),
            (
                "HERDR_SOCKET_PATH=/foreign herdr agent prompt worker hi",
                ObservationReason::RoutingUnverified,
            ),
            (
                "herdr --session foreign agent prompt worker hi",
                ObservationReason::RoutingUnverified,
            ),
        ] {
            let fact = observe(call(command, &result), 1, &|_| None).unwrap();
            assert_eq!(fact.reason, reason, "{command}");
            assert!(fact.target.is_none(), "{command}");
        }
        assert!(observe(call("echo ordinary", &result), 1, &|_| None).is_none());
        assert!(
            observe(
                call("echo 'herdr agent prompt worker hi'", &result),
                1,
                &|_| None
            )
            .is_none()
        );
    }

    #[test]
    fn incomplete_failed_and_background_results_stay_uncertain() {
        let result = answer();
        let mut missing = result.clone();
        missing["result"]["agent"]
            .as_object_mut()
            .unwrap()
            .remove("agent_session");
        let fact = observe(call("herdr agent prompt worker hi", &missing), 1, &|_| None).unwrap();
        assert_eq!(fact.reason, ObservationReason::MissingNativeReference);
        for (field, reason) in [
            (0, ObservationReason::ResultTruncated),
            (1, ObservationReason::ToolError),
            (2, ObservationReason::Interrupted),
            (3, ObservationReason::AsyncUnconfirmed),
        ] {
            let mut input = call("herdr agent prompt worker hi", &result);
            match field {
                0 => input.truncated = true,
                1 => input.failed = true,
                2 => input.interrupted = true,
                _ => input.asynchronous = true,
            }
            let fact = observe(input, 1, &|_| None).unwrap();
            assert_eq!(fact.reason, reason);
            assert!(fact.target.is_none());
        }
        for result in [
            json!("prefix {}"),
            json!("{\"ok\":true}"),
            json!({"stdout":answer().to_string(),"stderr":"unrelated warning"}),
            json!({"content":[{"type":"image"},{"type":"text","text":answer().to_string()}]}),
        ] {
            let fact =
                observe(call("herdr agent prompt worker hi", &result), 1, &|_| None).unwrap();
            assert_eq!(fact.status, ObservationStatus::ResultUnavailable);
            assert!(fact.target.is_none());
        }
    }

    #[test]
    fn only_the_exact_server_refusal_proves_rejection() {
        for (code, status) in [
            ("agent_blocked", ObservationStatus::RejectedObserved),
            ("timeout", ObservationStatus::OutcomeUnknown),
            ("agent_prompt_stalled", ObservationStatus::OutcomeUnknown),
        ] {
            let result = json!({"stderr":json!({"id":"cli:agent:prompt","error":{"code":code}}).to_string()});
            let mut input = call("herdr agent prompt worker hi", &result);
            input.failed = true;
            let fact = observe(input, 1, &|_| None).unwrap();
            assert_eq!(fact.status, status);
            assert!(fact.target.is_none());
        }
    }

    fn grok_payload() -> Value {
        // Grok 1.0.52's actual Post envelope has raw output bytes and a
        // separate decorated output_for_prompt; only raw bytes are evidence.
        let output = format!("{}\n", answer());
        let response = json!({"type":"Bash", "output":output.as_bytes(),
            "output_for_prompt":format!("exit: 0\n{output}"), "exit_code":0,
            "total_bytes":output.len(), "truncated":false, "timed_out":false,
            "signal":null});
        json!({"sessionId":"native", "session_id":"native",
            "toolUseId":"call", "tool_use_id":"call",
            "toolName":"run_terminal_command", "tool_name":"run_terminal_command",
            "toolInput":{"command":"herdr agent prompt worker hi"},
            "tool_input":{"command":"herdr agent prompt worker hi"},
            "toolResult":response, "tool_response":response,
            "toolInputTruncated":false, "toolResultTruncated":false,
            "isBackgrounded":false})
    }

    #[test]
    fn grok_native_byte_output_preserves_complete_server_metadata() {
        let payload = grok_payload();
        let fact = native(
            "grok",
            &serde_json::to_vec(&payload).unwrap(),
            false,
            false,
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(fact.reason, ObservationReason::Complete);
        assert_eq!(
            fact.target.unwrap().native_session.unwrap().value,
            "recipient-native"
        );
    }

    #[test]
    fn grok_raw_output_requires_complete_foreground_bytes() {
        for (field, value, reason) in [
            ("truncated", json!(true), ObservationReason::ResultTruncated),
            ("timed_out", json!(true), ObservationReason::Interrupted),
            ("signal", json!(15), ObservationReason::Interrupted),
            ("exit_code", json!(1), ObservationReason::ToolError),
            ("total_bytes", json!(1), ObservationReason::ResultMissing),
            ("output", json!([256]), ObservationReason::ResultMissing),
            ("output", json!([255]), ObservationReason::ResultMissing),
            ("output", json!(null), ObservationReason::ResultMissing),
        ] {
            let mut payload = grok_payload();
            for alias in ["toolResult", "tool_response"] {
                payload[alias][field] = value.clone();
                if field == "output" {
                    payload[alias]["total_bytes"] = json!(1);
                }
            }
            let fact = native(
                "grok",
                &serde_json::to_vec(&payload).unwrap(),
                false,
                false,
                1,
                &|_| None,
            )
            .unwrap();
            assert_eq!(fact.reason, reason, "{field}: {value}");
            assert!(fact.target.is_none(), "{field}: {value}");
        }
        let mut payload = grok_payload();
        payload["isBackgrounded"] = json!(true);
        let fact = native(
            "grok",
            &serde_json::to_vec(&payload).unwrap(),
            false,
            false,
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(fact.reason, ObservationReason::AsyncUnconfirmed);
        assert!(fact.target.is_none());
        let mut payload = grok_payload();
        payload["toolResult"]["truncated"] = json!(true);
        assert!(
            native(
                "grok",
                &serde_json::to_vec(&payload).unwrap(),
                false,
                false,
                1,
                &|_| None
            )
            .is_none()
        );
    }

    #[test]
    fn omp_complete_native_timing_envelope_preserves_server_metadata() {
        // omp 18.7.0's actual completed Bash callback contains one text
        // part: stdout's newline, two notice separators and this exact footer.
        let result = json!({"content":[{"type":"text",
            "text":format!("{}\n\n\nWall time: 0.34 seconds", answer())}]});
        let mut input = call("herdr agent prompt worker hi", &result);
        input.runtime_id = "omp";
        let fact = observe(input, 1, &|_| None).unwrap();
        assert_eq!(fact.reason, ObservationReason::Complete);
        assert_eq!(
            fact.target.unwrap().native_session.unwrap().value,
            "recipient-native"
        );
    }

    #[test]
    fn omp_timing_envelope_never_recovers_uncertain_or_competing_output() {
        let text = format!("{}\n\n\nWall time: 0.34 seconds", answer());
        let complete = json!({"content":[{"type":"text", "text":text}]});
        for runtime in ["claude-code", "codex", "grok", "opencode", "pi"] {
            let mut input = call("herdr agent prompt worker hi", &complete);
            input.runtime_id = runtime;
            let fact = observe(input, 1, &|_| None).unwrap();
            assert!(fact.target.is_none(), "{runtime}");
        }
        for result in [
            json!({"content":[{"type":"text", "text":format!("{text}\nwarning") }]}),
            json!({"content":[{"type":"text", "text":format!("prefix\n{text}") }]}),
            json!({"content":[{"type":"text", "text":format!("{}\n{text}", answer()) }]}),
            json!({"content":[{"type":"text", "text":text}, {"type":"text", "text":"warning"}]}),
            json!({"content":[{"type":"text", "text":text}], "stderr":"warning"}),
            json!({"content":[{"type":"text", "text":"{\"id\":\"cli:agent:prompt\"\n\nWall time: 0.34 seconds"}]}),
            json!({"content":[{"type":"text", "text":format!("{}\n[Some lines truncated to fit]...\n\nWall time: 0.34 seconds", answer()) }]}),
            json!({"content":[{"type":"text", "text":text.replace("0.34", "-0.34") }]}),
            json!({"content":[{"type":"text", "text":text.replace("0.34", "0.3") }]}),
            json!({"content":[{"type":"text", "text":text.replace("0.34", "00.34") }]}),
            json!({"content":[{"type":"text", "text":text.replace("0.34", "NaN") }]}),
        ] {
            let mut input = call("herdr agent prompt worker hi", &result);
            input.runtime_id = "omp";
            let fact = observe(input, 1, &|_| None).unwrap();
            assert_eq!(fact.reason, ObservationReason::ResultMissing, "{result}");
            assert!(fact.target.is_none());
        }
        for flag in 0..4 {
            let mut input = call("herdr agent prompt worker hi", &complete);
            input.runtime_id = "omp";
            match flag {
                0 => input.truncated = true,
                1 => input.failed = true,
                2 => input.interrupted = true,
                _ => input.asynchronous = true,
            }
            assert!(observe(input, 1, &|_| None).unwrap().target.is_none());
        }
    }

    #[test]
    fn opencode_native_content_output_preserves_complete_server_metadata() {
        // OpenCode 2.0.29's captured completed shell callback is normalized
        // to text content, without modifying the server's complete output.
        let result = json!({"content":[{"type":"text", "text":format!("{}\n", answer())}]});
        let mut input = call("herdr agent prompt worker hi", &result);
        input.runtime_id = "opencode";
        input.session_id = "ses_native";
        let fact = observe(input, 1, &|_| None).unwrap();
        assert_eq!(fact.reason, ObservationReason::Complete);
        assert_eq!(fact.session_id, "ses_native");
        assert_eq!(fact.target.unwrap().pane_id, "w1:p2");
    }

    #[test]
    fn provider_post_envelopes_keep_native_identity_and_weak_steps_weak() {
        for runtime in ["claude-code", "codex", "grok"] {
            let payload = if runtime == "grok" {
                json!({"sessionId":"native","toolUseId":"call","toolName":"run_terminal_command",
                "toolInput":{"command":"herdr agent prompt worker hi"},"toolResult":{"output":answer().to_string()}})
            } else {
                json!({"session_id":"native","tool_use_id":"call","tool_name":"Bash",
                    "tool_input":{"command":"herdr agent prompt worker hi"},"tool_response":{"stdout":answer().to_string()}})
            };
            let fact = native(
                runtime,
                &serde_json::to_vec(&payload).unwrap(),
                false,
                false,
                1,
                &|_| None,
            )
            .unwrap();
            assert_eq!(fact.session_id, "native");
            assert_eq!(fact.status, ObservationStatus::SubmittedObserved);
        }
        let agy = json!({"conversationId":"native-agy","stepIdx":2,"toolCall":{"name":"run_command", "args":{"CommandLine":"herdr agent prompt worker hi"}},"tool_response":answer()});
        let fact = native(
            "antigravity",
            &serde_json::to_vec(&agy).unwrap(),
            false,
            false,
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(
            fact.correlation_kind,
            ObservationCorrelation::ConversationStep
        );
        assert_eq!(fact.status, ObservationStatus::ResultUnavailable);
        assert!(fact.target.is_none());
        let conflict = json!({"session_id":"one","sessionId":"two"});
        assert!(
            native(
                "grok",
                &serde_json::to_vec(&conflict).unwrap(),
                false,
                false,
                1,
                &|_| None
            )
            .is_none()
        );
    }
}
