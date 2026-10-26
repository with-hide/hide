//! Tool calls a turn left failed (PRD hide-ai-suggestions B4, D-45): a call
//! whose result says it failed, unless a later call of the same kind in the
//! same turn succeeded. Each format's records are read for their calls and
//! results by structure; the tracker folds them across reads, as
//! [`crate::turns::TurnTracker`] folds turns, and hands each failure on once
//! its turn is over.
//!
//! A failure is named by its kind only ([`head`]): a shell command's program
//! and subcommand, or the tool's name. Neither the command line nor the
//! result leaves this module.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most calls waiting for their result, and the most failed kinds one
/// turn holds; past either the oldest is dropped, so a turn of a thousand
/// calls is read in bounded memory.
pub const TOOL_CALL_LIMIT: usize = 64;
pub const TOOL_FAILURE_LIMIT: usize = 32;
/// The longest kind kept, in bytes.
pub const HEAD_LIMIT: usize = 120;

/// One record's word about a tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolStep {
    /// A call of the kind `head`.
    Call { call: String, head: String },
    /// The result of `call`.
    Result { call: String, failed: bool },
    /// A call and its result in one record (OpenCode's tool part).
    Both { head: String, failed: bool },
}

pub(crate) type Parser = fn(&Value) -> Vec<ToolStep>;

/// A kind of call that failed and stayed failed until its turn ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedTool {
    pub head: String,
    pub at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OpenCall {
    call: String,
    head: String,
}

/// The current turn's calls and failures, carried from read to read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTracker {
    /// The offset of the first record not folded yet, so a read that starts
    /// earlier folds nothing twice.
    #[serde(default)]
    through: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    calls: Vec<OpenCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failed: Vec<FailedTool>,
}

impl ToolTracker {
    /// Folds one record's steps, written at `at_unix_ms`.
    pub(crate) fn fold(&mut self, offset: u64, at_unix_ms: u64, steps: Vec<ToolStep>) {
        if offset < self.through {
            return;
        }
        self.through = offset.saturating_add(1);
        for step in steps {
            match step {
                ToolStep::Call { call, head } => {
                    if self.calls.len() == TOOL_CALL_LIMIT {
                        self.calls.remove(0);
                    }
                    self.calls.push(OpenCall { call, head });
                }
                ToolStep::Result { call, failed } => {
                    let Some(index) = self.calls.iter().position(|open| open.call == call) else {
                        continue;
                    };
                    let head = self.calls.remove(index).head;
                    self.settle(head, failed, at_unix_ms);
                }
                ToolStep::Both { head, failed } => self.settle(head, failed, at_unix_ms),
            }
        }
    }

    fn settle(&mut self, head: String, failed: bool, at_unix_ms: u64) {
        self.failed.retain(|failure| failure.head != head);
        if failed {
            if self.failed.len() == TOOL_FAILURE_LIMIT {
                self.failed.remove(0);
            }
            self.failed.push(FailedTool { head, at_unix_ms });
        }
    }

    /// A person's message at `offset` began a new turn: what the last one
    /// left failed is over.
    pub(crate) fn human(&mut self, offset: u64, ended: &mut Vec<FailedTool>) {
        if offset < self.through {
            return;
        }
        self.through = offset.saturating_add(1);
        ended.append(&mut self.end_turn());
    }

    /// The turn is over (its agent stopped): what it left failed, each kind
    /// once, and nothing left open.
    pub fn end_turn(&mut self) -> Vec<FailedTool> {
        self.calls.clear();
        std::mem::take(&mut self.failed)
    }
}

/// The kind of a call: a shell command's program, with its subcommand when
/// it names one, or else the tool's own name.
pub fn head(tool: &str, command: Option<&str>) -> String {
    let named = command.and_then(command_head);
    let head = named.unwrap_or_else(|| tool.trim().to_owned());
    let mut end = head.len().min(HEAD_LIMIT);
    while !head.is_char_boundary(end) {
        end -= 1;
    }
    head[..end].to_owned()
}

/// What runs first in `line` that is not only a change of place or of
/// environment (`cd`, `export`, an assignment).
fn command_head(line: &str) -> Option<String> {
    hide_shell::simple_commands(line)
        .into_iter()
        .find_map(|words| {
            let mut words = words.into_iter().skip_while(|word| {
                hide_shell::is_assignment(word) || hide_shell::is_prefix(word, false)
            });
            let program = words.next()?;
            let program = program
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&program)
                .to_owned();
            if matches!(
                program.as_str(),
                "cd" | "pushd" | "popd" | "export" | "set" | "source" | "." | "unset" | "true"
            ) || program.is_empty()
            {
                return None;
            }
            let sub = words.next().filter(|word| {
                word.len() <= 32
                    && word.starts_with(|c: char| c.is_ascii_alphabetic())
                    && word
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
            });
            Some(match sub {
                Some(sub) => format!("{program} {sub}"),
                None => program,
            })
        })
}

/// A shell tool's command line, wherever the format keeps it.
fn command_of(input: &Value) -> Option<String> {
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(line) => Some(line.clone()),
        // Codex's older `shell` tool: an argv, `bash -lc <line>` included.
        Value::Array(argv) => {
            let words = argv.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            match words.as_slice() {
                [shell, flag, line] if flag.starts_with('-') && flag.ends_with('c') => {
                    let _ = shell;
                    Some((*line).to_owned())
                }
                _ => Some(words.join(" ")),
            }
        }
        _ => None,
    }
}

fn call_id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= crate::turns::NATIVE_ID_LIMIT_BYTES)
        .map(str::to_owned)
}

/// Claude Code: an assistant's `tool_use` blocks and a user record's
/// `tool_result` blocks, `is_error` on a failed one.
pub(crate) fn claude(item: &Value) -> Vec<ToolStep> {
    let role = item.get("type").and_then(Value::as_str);
    let Some(blocks) = item.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(
            |block| match (role, block.get("type").and_then(Value::as_str)) {
                (Some("assistant"), Some("tool_use")) => Some(ToolStep::Call {
                    call: call_id(block.get("id"))?,
                    head: head(
                        block.get("name").and_then(Value::as_str)?,
                        block.get("input").and_then(command_of).as_deref(),
                    ),
                }),
                (Some("user"), Some("tool_result")) => Some(ToolStep::Result {
                    call: call_id(block.get("tool_use_id"))?,
                    failed: block.get("is_error").and_then(Value::as_bool) == Some(true),
                }),
                _ => None,
            },
        )
        .collect()
}

/// Codex: `function_call`/`function_call_output` for its shell tools, whose
/// output carries the exit code, and `custom_tool_call`/`_output` for a
/// script that runs commands (`exec`), whose output carries each command's
/// `exit_code`.
pub(crate) fn codex(item: &Value) -> Vec<ToolStep> {
    if item.get("type").and_then(Value::as_str) != Some("response_item") {
        return Vec::new();
    }
    let Some(payload) = item.get("payload") else {
        return Vec::new();
    };
    let call = || call_id(payload.get("call_id"));
    let step = match payload.get("type").and_then(Value::as_str) {
        Some("function_call") => (|| {
            let name = payload.get("name").and_then(Value::as_str)?;
            let arguments = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
            Some(ToolStep::Call {
                call: call()?,
                head: head(name, arguments.as_ref().and_then(command_of).as_deref()),
            })
        })(),
        Some("custom_tool_call") => (|| {
            let name = payload.get("name").and_then(Value::as_str)?;
            let script = payload.get("input").and_then(Value::as_str);
            Some(ToolStep::Call {
                call: call()?,
                head: head(name, script.and_then(script_command).as_deref()),
            })
        })(),
        Some("function_call_output" | "custom_tool_call_output") => {
            call().map(|call| ToolStep::Result {
                call,
                failed: codex_failed(payload.get("output")),
            })
        }
        _ => None,
    };
    step.into_iter().collect()
}

/// The first command a Codex script runs: the string after its first
/// `cmd:`, read as the script's string literal.
fn script_command(script: &str) -> Option<String> {
    let start = script.find("cmd:")? + "cmd:".len();
    let rest = script[start..].trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut line = String::new();
    let mut chars = rest[quote.len_utf8()..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => line.push('\n'),
                't' => line.push('\t'),
                other => line.push(other),
            },
            c if c == quote => return Some(line),
            c => line.push(c),
        }
    }
    None
}

/// Whether a Codex tool output says its command failed: a nonzero exit
/// code in any of the forms Codex writes it.
fn codex_failed(output: Option<&Value>) -> bool {
    let failed = |code: i64| code != 0;
    let texts: Vec<&str> = match output {
        Some(Value::String(text)) => vec![text],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        Some(Value::Object(object)) => {
            return object
                .get("metadata")
                .and_then(|metadata| metadata.get("exit_code"))
                .and_then(Value::as_i64)
                .is_some_and(failed);
        }
        _ => return false,
    };
    texts.iter().any(|text| {
        let text = text.trim_start();
        if let Ok(value) = serde_json::from_str::<Value>(text)
            && let Some(code) = value.pointer("/metadata/exit_code").and_then(Value::as_i64)
        {
            return failed(code);
        }
        ["Exit code: ", "Process exited with code ", "\"exit_code\":"]
            .iter()
            .any(|marker| codes_after(text, marker).any(failed))
    })
}

fn codes_after<'a>(text: &'a str, marker: &'a str) -> impl Iterator<Item = i64> + 'a {
    text.match_indices(marker).filter_map(move |(at, _)| {
        let digits = text[at + marker.len()..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect::<String>();
        digits.parse().ok()
    })
}

/// Pi and omp: an assistant message's `toolCall` blocks and a `toolResult`
/// message, `isError` on a failed one.
pub(crate) fn pi(item: &Value) -> Vec<ToolStep> {
    if item["type"] != "message" {
        return Vec::new();
    }
    let message = &item["message"];
    match message["role"].as_str() {
        Some("toolResult") => call_id(message.get("toolCallId"))
            .map(|call| ToolStep::Result {
                call,
                failed: message["isError"] == true,
            })
            .into_iter()
            .collect(),
        Some("assistant") => message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "toolCall")
            .filter_map(|block| {
                Some(ToolStep::Call {
                    call: call_id(block.get("id"))?,
                    head: head(
                        block.get("name").and_then(Value::as_str)?,
                        block.get("arguments").and_then(command_of).as_deref(),
                    ),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Grok: a `tool_call` update names the call, and its terminal update says
/// `completed` or `failed`.
pub(crate) fn grok(item: &Value) -> Vec<ToolStep> {
    let Some(update) = item.pointer("/params/update") else {
        return Vec::new();
    };
    let Some(call) = call_id(update.get("toolCallId")) else {
        return Vec::new();
    };
    let kind = update.get("sessionUpdate").and_then(Value::as_str);
    let mut steps = Vec::new();
    if kind == Some("tool_call") {
        let name = update
            .get("title")
            .or_else(|| update.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("tool");
        steps.push(ToolStep::Call {
            call: call.clone(),
            head: head(name, update.get("rawInput").and_then(command_of).as_deref()),
        });
    }
    if matches!(kind, Some("tool_call" | "tool_call_update")) {
        match update["status"].as_str() {
            Some("completed") => steps.push(ToolStep::Result {
                call,
                failed: false,
            }),
            Some("failed") => steps.push(ToolStep::Result { call, failed: true }),
            _ => {}
        }
    }
    steps
}

/// OpenCode: one `tool` part holds its call and, once settled, its state.
pub(crate) fn opencode_part(part: &Value) -> Option<ToolStep> {
    if part["type"] != "tool" {
        return None;
    }
    let failed = match part.pointer("/state/status").and_then(Value::as_str)? {
        "completed" => false,
        "error" => true,
        _ => return None,
    };
    let tool = part.get("tool").and_then(Value::as_str)?;
    Some(ToolStep::Both {
        head: head(
            tool,
            part.pointer("/state/input").and_then(command_of).as_deref(),
        ),
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_command_is_named_by_its_program_and_subcommand() {
        assert_eq!(head("Bash", Some("cargo test -p herdr-core")), "cargo test");
        assert_eq!(head("Bash", Some("cd web && pnpm build")), "pnpm build");
        assert_eq!(
            head("Bash", Some("RUST_LOG=1 /usr/bin/git push origin")),
            "git push"
        );
        assert_eq!(head("Bash", Some("python3 -m pytest")), "python3");
        assert_eq!(head("Edit", None), "Edit");
        assert_eq!(head("Bash", Some("cd somewhere")), "Bash");
    }

    fn tracker(steps: &[(u64, Vec<ToolStep>)]) -> ToolTracker {
        let mut tracker = ToolTracker::default();
        for (offset, steps) in steps {
            tracker.fold(*offset, *offset * 10, steps.clone());
        }
        tracker
    }

    fn call(id: &str, head: &str) -> ToolStep {
        ToolStep::Call {
            call: id.into(),
            head: head.into(),
        }
    }

    fn result(id: &str, failed: bool) -> ToolStep {
        ToolStep::Result {
            call: id.into(),
            failed,
        }
    }

    #[test]
    fn a_failure_the_same_turn_fixed_is_not_kept() {
        let mut fixed = tracker(&[
            (1, vec![call("a", "cargo test")]),
            (2, vec![result("a", true)]),
            (3, vec![call("b", "cargo test")]),
            (4, vec![result("b", false)]),
        ]);
        assert!(fixed.end_turn().is_empty());

        let mut left = tracker(&[
            (1, vec![call("a", "cargo test"), call("b", "npm run")]),
            (2, vec![result("a", true), result("b", true)]),
            (3, vec![call("c", "cargo test")]),
            (4, vec![result("c", true)]),
        ]);
        let ended = left.end_turn();
        assert_eq!(
            ended.iter().map(|f| f.head.as_str()).collect::<Vec<_>>(),
            ["npm run", "cargo test"],
            "each kind once, the last failure's time"
        );
        assert_eq!(ended[1].at_unix_ms, 40);
    }

    #[test]
    fn a_new_turn_ends_the_last_and_a_reread_folds_nothing_twice() {
        let mut tracker = tracker(&[
            (1, vec![call("a", "git push")]),
            (2, vec![result("a", true)]),
        ]);
        tracker.fold(2, 20, vec![result("a", true)]);
        let mut ended = Vec::new();
        tracker.human(5, &mut ended);
        assert_eq!(ended.len(), 1);
        tracker.human(5, &mut ended);
        assert_eq!(ended.len(), 1, "the same message read again");
        assert!(tracker.end_turn().is_empty());
    }

    #[test]
    fn each_format_says_which_call_failed() {
        let claude_call = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}});
        let claude_result = json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}});
        assert_eq!(claude(&claude_call), [call("t1", "cargo test")]);
        assert_eq!(claude(&claude_result), [result("t1", true)]);

        let codex_call = json!({"type":"response_item","payload":{"type":"function_call",
            "name":"shell","call_id":"c1","arguments":"{\"command\":[\"bash\",\"-lc\",\"npm test\"]}"}});
        let codex_out = json!({"type":"response_item","payload":{"type":"function_call_output",
            "call_id":"c1","output":"Exit code: 2\nWall time: 1s\nOutput:\nfail"}});
        assert_eq!(codex(&codex_call), [call("c1", "npm test")]);
        assert_eq!(codex(&codex_out), [result("c1", true)]);
        let script = json!({"type":"response_item","payload":{"type":"custom_tool_call",
            "name":"exec","call_id":"c2","input":"text(await tools.exec_command({cmd:\"docker version\"}))"}});
        let script_out = json!({"type":"response_item","payload":{"type":"custom_tool_call_output",
            "call_id":"c2","output":[{"type":"input_text","text":"Script completed\n"},
            {"type":"input_text","text":"{\"chunk_id\":\"a\",\"exit_code\":1,\"output\":\"x\"}"}]}});
        assert_eq!(codex(&script), [call("c2", "docker version")]);
        assert_eq!(codex(&script_out), [result("c2", true)]);
        let passed = json!({"type":"response_item","payload":{"type":"custom_tool_call_output",
            "call_id":"c2","output":[{"type":"input_text","text":"{\"exit_code\":0}"}]}});
        assert_eq!(codex(&passed), [result("c2", false)]);

        let pi_call = json!({"type":"message","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"p1","name":"bash","arguments":{"command":"make check"}}]}});
        let pi_result = json!({"type":"message","message":{"role":"toolResult","toolCallId":"p1","isError":true}});
        assert_eq!(pi(&pi_call), [call("p1", "make check")]);
        assert_eq!(pi(&pi_result), [result("p1", true)]);

        let grok_failed = json!({"params":{"update":{"sessionUpdate":"tool_call_update",
            "toolCallId":"g1","status":"failed"}}});
        assert_eq!(grok(&grok_failed), [result("g1", true)]);

        let part = json!({"type":"tool","tool":"bash","callID":"o1",
            "state":{"status":"error","input":{"command":"go test ./..."}}});
        assert_eq!(
            opencode_part(&part),
            Some(ToolStep::Both {
                head: "go test".into(),
                failed: true
            })
        );
    }
}
