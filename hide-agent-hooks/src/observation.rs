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
/// Distinct recipients one call may name (PRD direct-prompt-observation-widen
/// D-12); a call past it keeps one record without a recipient.
pub const RECIPIENT_LIMIT: usize = 64;
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

/// What one shell call yields: one observation per distinct recipient of
/// its prompts (or one without a recipient), and one per agent it started.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Observed {
    pub prompts: Vec<DirectObservation>,
    pub starts: Vec<DirectObservation>,
    /// More than [`RECIPIENT_LIMIT`] recipients: the call keeps one record
    /// without a recipient, and the helper logs the refused capacity.
    pub over_capacity: bool,
}

/// The Herdr CLI answers each call with one JSON line carrying this id.
const PROMPT_ANSWER: &str = "cli:agent:prompt";
const START_ANSWER: &str = "cli:agent:start";

/// A call's commands as the shell runs them: each `herdr agent prompt`'s
/// target and each agent start's target as written, `None` when the word is
/// not literal (a variable, substitution or glob).
#[derive(Default)]
struct Commands {
    prompts: Vec<Option<String>>,
    starts: Vec<Option<String>>,
    /// A call aimed at another Herdr server, session or machine; its answer
    /// names that server's panes, which this pane's server cannot prove.
    routed: bool,
}

fn literal(word: &str) -> Option<String> {
    (identifier(word) && !word.contains(['$', '`', '*', '?', '[']) && !word.starts_with('-'))
        .then(|| word.to_owned())
}

/// The positional words of a `herdr` subcommand, skipping `options` (which
/// take a value unless spelled `--name=value`) and flag words; `None` for
/// `--help`, which runs nothing.
fn positionals<'a>(words: &'a [String], options: &[&str]) -> Option<Vec<&'a str>> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "-h" | "--help" => return None,
            "--" => {
                found.extend(words[at + 1..].iter().map(String::as_str));
                break;
            }
            option if options.contains(&option) => at += 1,
            flag if flag.starts_with('-') => {}
            positional => found.push(positional),
        }
        at += 1;
    }
    Some(found)
}

/// Every Herdr prompt and agent start in the call, read with the spawn
/// guard's lexer; no other pattern decides what a word is.
fn commands(command: &str, env: &dyn Fn(&str) -> Option<String>) -> Commands {
    let mut found = Commands::default();
    read_line(command, env, &mut found, 0);
    found
}

/// How deep `$(...)` substitutions are read; a shell runs each as a line.
const SUBSTITUTION_DEPTH: u8 = 4;

fn read_line(command: &str, env: &dyn Fn(&str) -> Option<String>, found: &mut Commands, depth: u8) {
    let line = spawn_guard::shell_line(command.trim());
    if depth < SUBSTITUTION_DEPTH {
        for inner in &line.substitutions {
            read_line(inner, env, found, depth + 1);
        }
    }
    let mut away = false;
    for words in &line.commands {
        let launch = spawn_guard::launch_of(words, env, away);
        let (words, foreign) = spawn_guard::command_words(words, env, true);
        let herdr = words.split_first().filter(|(program, _)| {
            let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
            matches!(
                name,
                "herdr" | "herdr.exe" | "$HERDR_BIN_PATH" | "${HERDR_BIN_PATH}"
            )
        });
        let Some((_, rest)) = herdr else {
            away |= spawn_guard::aims_away(words, env);
            continue;
        };
        // Global routing flags precede the subcommand; such a call is never
        // read as aimed at the sender pane's own server.
        let Some(at) = rest
            .iter()
            .position(|word| word == "agent" || word == "pane")
        else {
            continue;
        };
        let routed = foreign || away || at != 0;
        let rest = &rest[at..];
        match (
            rest.first().map(String::as_str),
            rest.get(1).map(String::as_str),
        ) {
            (Some("agent"), Some("prompt")) => {
                let Some(args) = positionals(&rest[2..], &["--until", "--timeout"]) else {
                    continue;
                };
                found.routed |= routed;
                found
                    .prompts
                    .push(args.first().and_then(|word| literal(word)));
            }
            (Some("agent"), Some("start")) if launch.is_some() || routed => {
                found.routed |= routed;
                // Herdr names the agent it starts; the name identifies it
                // where the pane alone would name whatever runs there.
                let name = launch.and_then(|launch| launch.name);
                found.starts.push(name.as_deref().and_then(literal));
            }
            (Some("pane"), Some("run")) if launch.is_some() || routed => {
                found.routed |= routed;
                found
                    .starts
                    .push(rest.get(2).and_then(|pane| literal(pane)));
            }
            _ => {}
        }
    }
}

/// Reports only statically recognized Herdr prompts and agent starts.
/// Ordinary shell calls stop before parsing output or starting a transport
/// child.
pub fn observe(
    call: ToolResult<'_>,
    observed_at_unix_ms: u64,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Observed> {
    if call.command.len() > COMMAND_LIMIT
        || !identifier(call.session_id)
        || !identifier(call.call_id)
        || !(call.command.contains("herdr") || call.command.contains("HERDR_BIN_PATH"))
    {
        return None;
    }
    let commands = commands(call.command, env);
    if commands.prompts.is_empty() && commands.starts.is_empty() {
        return None;
    }
    let base = DirectObservation {
        runtime_id: call.runtime_id.to_owned(),
        session_id: call.session_id.to_owned(),
        call_id: call.call_id.to_owned(),
        correlation_kind: call.correlation_kind,
        observed_at_unix_ms,
        status: ObservationStatus::OutcomeUnknown,
        reason: ObservationReason::ResultMissing,
        target: None,
    };
    let raw_output = call
        .result
        .filter(|_| call.runtime_id == "grok")
        .and_then(grok_raw_output);
    let answers = if commands.routed {
        Vec::new()
    } else {
        raw_output
            .as_ref()
            .or(call.result)
            .map(herdr_answers)
            .unwrap_or_default()
    };
    // Literal targets stand in only for a call whose shell ran to its end and
    // whose command text arrived whole; Herdr's own answer always wins.
    let settled = !call.failed && !call.interrupted && !call.asynchronous && !call.truncated;
    let mut observed = Observed::default();
    if !commands.prompts.is_empty() {
        let (targets, outcome) = if commands.routed {
            (
                Vec::new(),
                (
                    ObservationStatus::OutcomeUnknown,
                    ObservationReason::RoutingUnverified,
                ),
            )
        } else {
            recipients(&answers, PROMPT_ANSWER, &commands.prompts, settled, &call)
        };
        if targets.len() > RECIPIENT_LIMIT {
            observed.over_capacity = true;
            observed.prompts.push(DirectObservation {
                reason: ObservationReason::CommandAmbiguous,
                ..base.clone()
            });
        } else if targets.is_empty() {
            observed.prompts.push(DirectObservation {
                status: outcome.0,
                reason: outcome.1,
                ..base.clone()
            });
        } else {
            observed.prompts = targets
                .into_iter()
                .map(|(status, reason, target)| DirectObservation {
                    status,
                    reason,
                    target: Some(target),
                    ..base.clone()
                })
                .collect();
        }
    }
    if !commands.starts.is_empty() && !commands.routed {
        let (targets, _) = recipients(&answers, START_ANSWER, &commands.starts, settled, &call);
        if targets.len() > RECIPIENT_LIMIT {
            observed.over_capacity = true;
            return Some(observed);
        }
        observed.starts = targets
            .into_iter()
            .map(|(status, reason, target)| DirectObservation {
                status,
                reason,
                target: Some(target),
                ..base.clone()
            })
            .collect();
    }
    Some(observed)
}

type Recipient = (ObservationStatus, ObservationReason, ServerReportedTarget);

/// Each distinct pane Herdr reported for `id`, or, when it reported none,
/// the literal targets of the commands; otherwise what the call says about
/// why it has no recipient.
fn recipients(
    answers: &[Value],
    id: &str,
    literals: &[Option<String>],
    settled: bool,
    call: &ToolResult<'_>,
) -> (Vec<Recipient>, (ObservationStatus, ObservationReason)) {
    let mut found: Vec<Recipient> = Vec::new();
    let add = |found: &mut Vec<Recipient>, entry: Recipient| {
        if !found
            .iter()
            .any(|(_, _, target)| target.pane_id == entry.2.pane_id)
        {
            found.push(entry);
        }
    };
    let (mut blocked, mut waited, mut refused) = (false, false, false);
    for answer in answers.iter().filter(|answer| answer["id"] == id) {
        if let Some(error) = answer.get("error") {
            match error["code"].as_str() {
                // Only the exact server validation refusal proves no input.
                Some("agent_blocked") => blocked = true,
                // --wait gave up after Herdr took the input.
                Some("timeout" | "agent_prompt_stalled") => waited = true,
                _ => refused = true,
            }
            continue;
        }
        let agent = &answer["result"]["agent"];
        let Some(pane_id) = agent["pane_id"].as_str().filter(|value| identifier(value)) else {
            continue;
        };
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
        add(
            &mut found,
            (
                ObservationStatus::SubmittedObserved,
                reason,
                ServerReportedTarget {
                    pane_id: pane_id.to_owned(),
                    terminal_id: agent["terminal_id"]
                        .as_str()
                        .filter(|value| identifier(value))
                        .map(str::to_owned),
                    native_session,
                },
            ),
        );
    }
    if !found.is_empty() {
        return (
            found,
            (
                ObservationStatus::SubmittedObserved,
                ObservationReason::Complete,
            ),
        );
    }
    let heard = blocked || waited || refused;
    if (!heard && settled) || (waited && !blocked && !refused) {
        for target in literals.iter().flatten() {
            add(
                &mut found,
                (
                    ObservationStatus::OutcomeUnknown,
                    ObservationReason::ResultMissing,
                    ServerReportedTarget {
                        pane_id: target.clone(),
                        terminal_id: None,
                        native_session: None,
                    },
                ),
            );
        }
        if !found.is_empty() {
            return (
                found,
                (
                    ObservationStatus::OutcomeUnknown,
                    ObservationReason::ResultMissing,
                ),
            );
        }
    }
    let outcome = if blocked {
        (
            ObservationStatus::RejectedObserved,
            ObservationReason::HerdrRejected,
        )
    } else if refused || waited || call.failed {
        (
            ObservationStatus::OutcomeUnknown,
            ObservationReason::ToolError,
        )
    } else if call.truncated {
        (
            ObservationStatus::ResultUnavailable,
            ObservationReason::ResultTruncated,
        )
    } else if call.interrupted {
        (
            ObservationStatus::OutcomeUnknown,
            ObservationReason::Interrupted,
        )
    } else if call.asynchronous {
        (
            ObservationStatus::OutcomeUnknown,
            ObservationReason::AsyncUnconfirmed,
        )
    } else if literals.iter().any(Option::is_none) {
        (
            ObservationStatus::OutcomeUnknown,
            ObservationReason::CommandAmbiguous,
        )
    } else {
        (
            ObservationStatus::ResultUnavailable,
            ObservationReason::ResultMissing,
        )
    };
    (found, outcome)
}

/// Herdr's complete JSON answer lines in a provider's result: a structured
/// answer, or each line of the known output envelopes that parses whole as
/// one answer. No scan for a JSON substring inside a line: a clipped,
/// prefixed or decorated line is not an answer.
fn herdr_answers(value: &Value) -> Vec<Value> {
    let mut answers = Vec::new();
    let mut budget = RESULT_LIMIT;
    collect_answers(value, &mut answers, &mut budget, 0);
    answers
}

fn collect_answers(value: &Value, answers: &mut Vec<Value>, budget: &mut usize, depth: u8) {
    if answer_line(value) {
        answers.push(value.clone());
        return;
    }
    if let Some(text) = value.as_str() {
        if text.len() > *budget {
            *budget = 0;
            return;
        }
        *budget -= text.len();
        answers.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
                .filter(answer_line),
        );
        return;
    }
    // Known provider envelopes; never recursively search arbitrary metadata.
    if depth > 0 {
        return;
    }
    for field in ["stdout", "stderr", "output", "structuredContent"] {
        if let Some(output) = value.get(field) {
            collect_answers(output, answers, budget, depth + 1);
        }
    }
    let parts = value
        .get("content")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    for part in parts.into_iter().flatten() {
        if part["type"] == "text" {
            collect_answers(&part["text"], answers, budget, depth + 1);
        }
    }
}

fn answer_line(value: &Value) -> bool {
    matches!(value["id"].as_str(), Some(PROMPT_ANSWER | START_ANSWER))
        && (value.get("result").is_some() ^ value.get("error").is_some())
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

/// Native command-hook payloads. Post has the actual executed input, so no
/// Pre spool, target query, detached helper or transcript scan is needed.
pub fn native(
    runtime: &str,
    payload: &[u8],
    truncated: bool,
    failure: bool,
    now: u64,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Observed> {
    use hide_agent_adapter::{HookDialect, ObservationSupport};
    let adapter = hide_agent_adapter::adapter(runtime)?;
    let ObservationSupport::Native(dialect) = adapter.observation else {
        return None;
    };
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
    let agy = dialect == HookDialect::Antigravity;
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
    let grok = dialect == HookDialect::Grok;
    observe(
        ToolResult {
            runtime_id: adapter.id,
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

/// No retries of the original operation. Each owned CLI child only submits
/// metadata and inherits the same proof/route as existing hook requests;
/// prompts and starts share the one deadline.
pub fn send(home: &Path, observed: &Observed, deadline: Instant) -> bool {
    if observed.over_capacity {
        delivery::diagnose(home, "capacity");
    }
    [
        (&observed.prompts, &["observation", "--hook"][..]),
        (&observed.starts, &["observation", "--hook", "--start"][..]),
    ]
    .into_iter()
    .filter(|(items, _)| !items.is_empty())
    .fold(true, |accepted, (items, arguments)| {
        if items
            .iter()
            .any(|item| serde_json::to_vec(item).map_or(true, |bytes| bytes.len() > ITEM_LIMIT))
        {
            delivery::diagnose(home, "capacity");
            return false;
        }
        let Ok(bytes) = serde_json::to_vec(items) else {
            return false;
        };
        match delivery::run_cli_with_input(arguments, &bytes, deadline) {
            Ok(_) => accepted,
            Err(failure) => {
                delivery::diagnose_failure(home, &failure);
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(pane: &str, native: &str) -> Value {
        json!({"pane_id":pane,"terminal_id":format!("term-{pane}"),"agent_session":{
            "source":"herdr:codex","agent":"codex","kind":"id","value":native
        }})
    }

    fn answer() -> Value {
        json!({"id":"cli:agent:prompt","result":{"type":"agent_prompted","agent":agent("w1:p2","recipient-native")}})
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

    /// The call's one prompt observation.
    fn one(observed: Observed) -> DirectObservation {
        assert!(observed.starts.is_empty());
        assert_eq!(observed.prompts.len(), 1, "{observed:?}");
        observed.prompts.into_iter().next().unwrap()
    }

    fn panes(observed: &[DirectObservation]) -> Vec<&str> {
        observed
            .iter()
            .map(|fact| fact.target.as_ref().unwrap().pane_id.as_str())
            .collect()
    }

    #[test]
    fn canonical_complete_output_keeps_only_metadata() {
        let result = json!({"stdout":answer().to_string(),"stderr":""});
        let fact = one(observe(
            call("herdr agent prompt worker 'private prompt body'", &result),
            42,
            &|_| None,
        )
        .unwrap());
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
        assert_eq!(target.terminal_id.as_deref(), Some("term-w1:p2"));
        let wire = serde_json::to_string(&fact).unwrap();
        assert!(!wire.contains("private prompt body"));
        assert!(!wire.contains("stdout"));
        assert!(!wire.contains("worker"));
        assert_eq!(fact.observed_at_unix_ms, 42);
    }

    /// PRD direct-prompt-observation-widen B1, B2: the shapes agents
    /// actually write keep Herdr's answer as the recipient.
    #[test]
    fn piped_compound_and_waiting_calls_keep_herdr_s_answer() {
        let wait = json!({"id":"cli:agent:prompt","result":{"type":"agent_info","agent":agent("w1:p2","recipient-native")}});
        for (command, output) in [
            ("herdr agent prompt w1:p2 'body' | jq .", answer()),
            ("herdr agent prompt worker \"body\"; echo done", answer()),
            ("herdr agent prompt worker body 2>&1 | head -5", answer()),
            ("herdr agent prompt worker body --wait", wait.clone()),
            (
                "herdr agent prompt worker body --wait --until working --timeout 5000",
                wait,
            ),
            ("cd /tmp && FOO=1 herdr agent prompt worker body", answer()),
            ("$HERDR_BIN_PATH agent prompt worker body", answer()),
        ] {
            let result = json!({"stdout":format!("{}\ndone\n", output)});
            let fact = one(observe(call(command, &result), 1, &|_| None).unwrap());
            assert_eq!(
                fact.status,
                ObservationStatus::SubmittedObserved,
                "{command}"
            );
            assert_eq!(fact.reason, ObservationReason::Complete, "{command}");
            assert_eq!(fact.target.unwrap().pane_id, "w1:p2", "{command}");
        }
    }

    /// B4, B5, B6: the command's own target stands in only when Herdr's
    /// answer is not in the output, and never against it.
    #[test]
    fn literal_targets_stand_in_only_without_herdr_s_answer() {
        let hidden = json!({"stdout":"agent_prompted\n"});
        for (command, target) in [
            (
                "herdr agent prompt w1:p3 body | jq -r .result.type",
                "w1:p3",
            ),
            ("herdr agent prompt reviewer body > /dev/null", "reviewer"),
            (
                "herdr agent prompt --wait reviewer body >/dev/null",
                "reviewer",
            ),
        ] {
            let fact = one(observe(call(command, &hidden), 1, &|_| None).unwrap());
            assert_eq!(
                (fact.status, fact.reason),
                (
                    ObservationStatus::OutcomeUnknown,
                    ObservationReason::ResultMissing
                ),
                "{command}"
            );
            let named = fact.target.unwrap();
            assert_eq!(named.pane_id, target, "{command}");
            assert!(named.terminal_id.is_none() && named.native_session.is_none());
        }
        // Herdr's answer names another pane than the text: the answer wins.
        let result = json!({"stdout":answer().to_string()});
        let fact =
            one(observe(call("herdr agent prompt w9:p9 body", &result), 1, &|_| None).unwrap());
        assert_eq!(fact.target.unwrap().pane_id, "w1:p2");
        // A variable, substitution or glob names nothing a reader can know.
        for command in [
            "herdr agent prompt $p body | jq .",
            "herdr agent prompt \"$p\" body >/dev/null",
            "herdr agent prompt $(cat target) body >/dev/null",
            "herdr agent prompt 'w*' body >/dev/null",
        ] {
            let fact = one(observe(call(command, &hidden), 1, &|_| None).unwrap());
            assert_eq!(
                fact.reason,
                ObservationReason::CommandAmbiguous,
                "{command}"
            );
            assert!(fact.target.is_none(), "{command}");
        }
    }

    /// A command substitution is a line the shell runs too: its literal
    /// target counts as one written anywhere else in the call.
    #[test]
    fn a_prompt_inside_a_command_substitution_is_read() {
        let hidden = json!({"stdout":"prompt ok\n"});
        let fact = one(observe(
            call(
                "r=$(herdr agent prompt w1:p9 \"$q\" 2>&1); echo \"$r\" | grep -q agent_prompted && echo \"prompt ok\"",
                &hidden,
            ),
            1,
            &|_| None,
        )
        .unwrap());
        assert_eq!(fact.target.unwrap().pane_id, "w1:p9");
        assert!(
            observe(
                call("echo \"$(echo herdr agent prompt w1:p9 hi)\"", &hidden),
                1,
                &|_| None
            )
            .is_none()
        );
    }

    /// B3, B10: each answer line is one recipient, once per pane.
    #[test]
    fn a_loop_yields_one_observation_per_distinct_recipient() {
        let lines = [
            answer(),
            json!({"id":"cli:agent:prompt","result":{"type":"agent_prompted","agent":agent("w1:p3","third")}}),
            answer(),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let result = json!({"stdout":lines});
        let observed = observe(
            call(
                "for p in $A $B; do herdr agent prompt \"$p\" hi | jq -c .; done",
                &result,
            ),
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(panes(&observed.prompts), ["w1:p2", "w1:p3"]);
        assert!(observed.prompts.iter().all(|fact| fact.call_id == "tool-1"));
        let again = observe(
            call(
                "for p in $A $B; do herdr agent prompt \"$p\" hi | jq -c .; done",
                &result,
            ),
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(again, observed);
    }

    /// B9: past the recipient cap the call keeps one record without one.
    #[test]
    fn more_than_the_recipient_cap_keeps_one_record_without_a_recipient() {
        let lines = (0..=RECIPIENT_LIMIT)
            .map(|index| {
                json!({"id":"cli:agent:prompt","result":{"type":"agent_prompted",
                    "agent":agent(&format!("w1:p{index}"),"native")}})
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let result = json!({"stdout":lines});
        let observed = observe(
            call(
                "for p in $PANES; do herdr agent prompt $p hi; done",
                &result,
            ),
            1,
            &|_| None,
        )
        .unwrap();
        assert!(observed.over_capacity);
        let fact = one(observed);
        assert!(fact.target.is_none());
        assert_eq!(fact.reason, ObservationReason::CommandAmbiguous);
    }

    /// B7, D-13: Herdr's refusal names no recipient; a wait that gave up
    /// after Herdr took the input falls back to the command's target.
    #[test]
    fn refusals_name_no_recipient_and_wait_timeouts_fall_back() {
        for (code, status, target) in [
            ("agent_blocked", ObservationStatus::RejectedObserved, None),
            ("agent_not_found", ObservationStatus::OutcomeUnknown, None),
            ("timeout", ObservationStatus::OutcomeUnknown, Some("worker")),
            (
                "agent_prompt_stalled",
                ObservationStatus::OutcomeUnknown,
                Some("worker"),
            ),
        ] {
            let result = json!({"stderr":json!({"id":"cli:agent:prompt","error":{"code":code}}).to_string()});
            let mut input = call("herdr agent prompt worker hi --wait | jq .", &result);
            input.failed = true;
            let fact = one(observe(input, 1, &|_| None).unwrap());
            assert_eq!(fact.status, status, "{code}");
            assert_eq!(
                fact.target.as_ref().map(|target| target.pane_id.as_str()),
                target,
                "{code}"
            );
        }
    }

    #[test]
    fn uncertain_execution_never_takes_the_command_s_target() {
        let hidden = json!({"stdout":""});
        for (field, reason) in [
            (0, ObservationReason::ResultTruncated),
            (1, ObservationReason::ToolError),
            (2, ObservationReason::Interrupted),
            (3, ObservationReason::AsyncUnconfirmed),
        ] {
            let mut input = call("herdr agent prompt worker hi", &hidden);
            match field {
                0 => input.truncated = true,
                1 => input.failed = true,
                2 => input.interrupted = true,
                _ => input.asynchronous = true,
            }
            let fact = one(observe(input, 1, &|_| None).unwrap());
            assert_eq!(fact.reason, reason);
            assert!(fact.target.is_none());
        }
        // A complete answer line survives a clipped or failed rest.
        let result = json!({"stdout":format!("{}\n{{\"id\":\"cli:agent:pro", answer())});
        let mut input = call("herdr agent prompt worker hi; false", &result);
        input.truncated = true;
        input.failed = true;
        assert_eq!(
            one(observe(input, 1, &|_| None).unwrap())
                .target
                .unwrap()
                .pane_id,
            "w1:p2"
        );
    }

    #[test]
    fn only_whole_answer_lines_count() {
        let mut missing = answer();
        missing["result"]["agent"]
            .as_object_mut()
            .unwrap()
            .remove("agent_session");
        let fact =
            one(observe(call("herdr agent prompt worker hi", &missing), 1, &|_| None).unwrap());
        assert_eq!(fact.reason, ObservationReason::MissingNativeReference);
        assert_eq!(fact.target.unwrap().pane_id, "w1:p2");
        for result in [
            json!(format!("prefix {}", answer())),
            json!("{\"ok\":true}"),
            json!({"stdout":"{\"id\":\"cli:agent:prompt\",\"result\":{},\"error\":{}}"}),
            json!({"metadata":{"stdout":answer().to_string()}}),
        ] {
            let fact =
                one(observe(call("herdr agent prompt $T hi", &result), 1, &|_| None).unwrap());
            assert!(fact.target.is_none(), "{result}");
        }
        for result in [
            json!({"stdout":answer().to_string(),"stderr":"unrelated warning"}),
            json!({"content":[{"type":"image"},{"type":"text","text":answer().to_string()}]}),
            json!({"content":[{"type":"text","text":format!("{}\n\n\nWall time: 0.34 seconds", answer())}]}),
        ] {
            let fact =
                one(observe(call("herdr agent prompt $T hi", &result), 1, &|_| None).unwrap());
            assert_eq!(fact.target.unwrap().pane_id, "w1:p2", "{result}");
        }
    }

    #[test]
    fn foreign_routes_and_quoted_text_are_not_this_server_s_inputs() {
        let result = json!({"stdout":answer().to_string()});
        for command in [
            "HERDR_SOCKET_PATH=/foreign herdr agent prompt worker hi",
            "herdr --session foreign agent prompt worker hi",
            "export HERDR_SOCKET_PATH=/foreign; herdr agent prompt worker hi",
        ] {
            let fact = one(observe(call(command, &result), 1, &|_| None).unwrap());
            assert_eq!(
                fact.reason,
                ObservationReason::RoutingUnverified,
                "{command}"
            );
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
        assert!(observe(call("herdr agent prompt --help", &result), 1, &|_| None).is_none());
    }

    /// D-23: an agent start yields a start observation of its own.
    #[test]
    fn agent_starts_name_the_started_agent() {
        let started = json!({"id":"cli:agent:start","result":{"type":"agent_started",
            "argv":["codex"],"agent":agent("w1:p3","child-native")}});
        let result = json!({"stdout":started.to_string()});
        let observed = observe(
            call(
                "herdr agent start helper --kind codex --pane w1:p3 | jq .",
                &result,
            ),
            1,
            &|_| None,
        )
        .unwrap();
        assert!(observed.prompts.is_empty());
        let target = observed.starts[0].target.as_ref().unwrap();
        assert_eq!(
            (target.pane_id.as_str(), target.terminal_id.as_deref()),
            ("w1:p3", Some("term-w1:p3"))
        );
        let hidden = json!({"stdout":""});
        for (command, target) in [
            (
                "herdr agent start helper --kind codex --pane w1:p3 >/dev/null",
                "helper",
            ),
            ("herdr pane run w1:p4 codex", "w1:p4"),
        ] {
            let observed = observe(call(command, &hidden), 1, &|_| None).unwrap();
            assert_eq!(panes(&observed.starts), [target], "{command}");
        }
        let refused = json!({"stderr":json!({"id":"cli:agent:start","error":{"code":"agent_not_ready"}}).to_string()});
        let mut input = call(
            "herdr agent start helper --kind codex --pane w1:p3",
            &refused,
        );
        input.failed = true;
        assert!(observe(input, 1, &|_| None).unwrap().starts.is_empty());
        for command in [
            "herdr pane run w1:p4 ls",
            "herdr agent start helper --kind nothing --pane w1:p3",
        ] {
            assert!(
                observe(call(command, &hidden), 1, &|_| None).is_none(),
                "{command}"
            );
        }
        let both = json!({"stdout":format!("{started}\n{}", answer())});
        let observed = observe(
            call(
                "herdr agent start helper --kind codex --pane w1:p3 && herdr agent prompt helper go",
                &both,
            ),
            1,
            &|_| None,
        )
        .unwrap();
        assert_eq!(
            (panes(&observed.prompts), panes(&observed.starts)),
            (vec!["w1:p2"], vec!["w1:p3"])
        );
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
            "toolInput":{"command":"herdr agent prompt $T hi"},
            "tool_input":{"command":"herdr agent prompt $T hi"},
            "toolResult":response, "tool_response":response,
            "toolInputTruncated":false, "toolResultTruncated":false,
            "isBackgrounded":false})
    }

    fn grok(payload: &Value) -> Option<DirectObservation> {
        native(
            "grok",
            &serde_json::to_vec(payload).unwrap(),
            false,
            false,
            1,
            &|_| None,
        )
        .map(one)
    }

    #[test]
    fn grok_native_byte_output_preserves_complete_server_metadata() {
        let fact = grok(&grok_payload()).unwrap();
        assert_eq!(fact.reason, ObservationReason::Complete);
        assert_eq!(
            fact.target.unwrap().native_session.unwrap().value,
            "recipient-native"
        );
    }

    #[test]
    fn grok_reads_only_its_raw_bytes() {
        for (field, value, reason) in [
            ("total_bytes", json!(1), ObservationReason::CommandAmbiguous),
            ("output", json!([256]), ObservationReason::CommandAmbiguous),
            ("output", json!([255]), ObservationReason::CommandAmbiguous),
            ("output", json!(null), ObservationReason::CommandAmbiguous),
            ("exit_code", json!(1), ObservationReason::Complete),
            ("timed_out", json!(true), ObservationReason::Complete),
        ] {
            let mut payload = grok_payload();
            for alias in ["toolResult", "tool_response"] {
                payload[alias][field] = value.clone();
                if field == "output" {
                    payload[alias]["total_bytes"] = json!(1);
                }
            }
            let fact = grok(&payload).unwrap();
            assert_eq!(fact.reason, reason, "{field}: {value}");
        }
        let mut payload = grok_payload();
        payload["isBackgrounded"] = json!(true);
        for alias in ["toolResult", "tool_response"] {
            payload[alias]["output"] = json!([]);
            payload[alias]["total_bytes"] = json!(0);
        }
        assert_eq!(
            grok(&payload).unwrap().reason,
            ObservationReason::AsyncUnconfirmed
        );
        let mut payload = grok_payload();
        payload["toolResult"]["truncated"] = json!(true);
        assert!(grok(&payload).is_none());
    }

    #[test]
    fn opencode_native_content_output_preserves_complete_server_metadata() {
        // OpenCode 2.0.29's captured completed shell callback is normalized
        // to text content, without modifying the server's complete output.
        let result = json!({"content":[{"type":"text", "text":format!("{}\n", answer())}]});
        let mut input = call("herdr agent prompt worker hi", &result);
        input.runtime_id = "opencode";
        input.session_id = "ses_native";
        let fact = one(observe(input, 1, &|_| None).unwrap());
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
            let fact = one(native(
                runtime,
                &serde_json::to_vec(&payload).unwrap(),
                false,
                false,
                1,
                &|_| None,
            )
            .unwrap());
            assert_eq!(fact.session_id, "native");
            assert_eq!(fact.status, ObservationStatus::SubmittedObserved);
        }
        // Antigravity supplies no result: its command's own target is all
        // there is, under its weak conversation-step correlation (D-14).
        let agy = json!({"conversationId":"native-agy","stepIdx":2,"toolCall":{"name":"run_command", "args":{"CommandLine":"herdr agent prompt worker hi"}},"tool_response":answer()});
        let fact = one(native(
            "antigravity",
            &serde_json::to_vec(&agy).unwrap(),
            false,
            false,
            1,
            &|_| None,
        )
        .unwrap());
        assert_eq!(
            fact.correlation_kind,
            ObservationCorrelation::ConversationStep
        );
        assert_eq!(fact.status, ObservationStatus::OutcomeUnknown);
        let target = fact.target.unwrap();
        assert_eq!(target.pane_id, "worker");
        assert!(target.native_session.is_none());
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
