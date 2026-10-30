//! Codex rollouts. Messages and reasoning are `response_item` records in
//! every rollout; tools are written two ways, chosen per session by
//! `session_meta.history_mode` (`codex-rs/rollout/src/policy.rs`):
//!
//! - `paginated`: each finished tool is one `item_completed` event holding
//!   the command and its output, or the file changes with their diffs. The
//!   model's own calls (`exec` scripts in code mode) are also written, and
//!   are not drawn, since the items already show what they did.
//! - `legacy`, the default when the field is absent: the model's calls and
//!   their outputs (`function_call`, `custom_tool_call` and their
//!   `*_output` records, paired by `call_id`).
//!
//! Titles and kinds follow codex-acp (D-04).

use agent_client_protocol_schema::v1::{PlanEntryStatus, ToolKind};
use serde_json::Value;

use super::Lines;
use super::build::{Call, FileDiff, Origin, Outcome, Part, Step, Thread};

pub(super) const LINES: Lines = Lines {
    header: Some(|record| record["type"] == "session_meta"),
    record: |record_, session, thread| record(record_, session.header, thread),
    may_start_page: |_| true,
};

fn record(record: &Value, header: &Value, thread: &mut Thread) {
    let payload = &record["payload"];
    let paginated = header["payload"]["history_mode"] == "paginated";
    let cwd = header["payload"]["cwd"].as_str().unwrap_or_default();
    match (record["type"].as_str(), payload["type"].as_str()) {
        (Some("response_item"), Some("message")) => message(payload, thread),
        (Some("response_item"), Some("reasoning")) => reasoning(payload, thread),
        (Some("event_msg"), Some("item_completed")) => {
            item(&payload["item"], paginated, cwd, thread)
        }
        (Some("response_item"), Some(kind)) if !paginated => legacy(kind, payload, cwd, thread),
        _ => {}
    }
}

fn message(payload: &Value, thread: &mut Thread) {
    let id = payload["id"].as_str();
    let Some(content) = payload["content"].as_array() else {
        thread.unreadable("codex_message_shape");
        return;
    };
    match payload["role"].as_str() {
        Some("user") => {
            let parts = content.iter().filter_map(user_part).collect::<Vec<_>>();
            thread.user(id, Origin::Operator, parts);
        }
        Some("assistant") => {
            for part in content {
                if let Some(text) = part["text"].as_str() {
                    thread.agent(id, Part::Text(text));
                }
            }
        }
        // Instructions the harness gives the model are not conversation.
        _ => {}
    }
}

fn user_part(part: &Value) -> Option<Part<'_>> {
    match part["type"].as_str()? {
        "input_text" => {
            let text = part["text"].as_str()?;
            // Codex wraps an attached image in tags of its own; the image
            // block itself says what was attached.
            let wrapper =
                text == "</image>" || (text.starts_with("<image ") && text.ends_with('>'));
            (!wrapper).then_some(Part::Text(text))
        }
        "input_image" => data_url(part["image_url"].as_str()?),
        _ => None,
    }
}

/// A `data:<mime>;base64,<data>` URL as an image part.
fn data_url(url: &str) -> Option<Part<'_>> {
    let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    Some(Part::Image { data, mime })
}

fn reasoning(payload: &Value, thread: &mut Thread) {
    let id = payload["id"].as_str();
    for key in ["summary", "content"] {
        for part in payload[key].as_array().into_iter().flatten() {
            if let Some(text) = part["text"].as_str() {
                thread.thought(id, text);
            }
        }
    }
}

fn item(item: &Value, paginated: bool, cwd: &str, thread: &mut Thread) {
    let Some(id) = item["id"].as_str() else {
        thread.unreadable("codex_item_shape");
        return;
    };
    match item["type"].as_str() {
        // A proposed plan, written in both modes, reads as the agent's text.
        Some("Plan") => thread.agent(
            Some(id),
            Part::Text(item["text"].as_str().unwrap_or_default()),
        ),
        // A legacy rollout writes only these few items, and its tools as
        // calls; the rest of its items repeat what a call already says.
        _ if !paginated => {}
        Some("CommandExecution") => {
            let command = command_line(&item["command"]);
            thread.call(
                Call::new(id, "exec_command", ToolKind::Execute)
                    .title(command.clone())
                    .input(&serde_json::json!({ "command": command })),
            );
            let failed = item["exit_code"].as_i64().is_some_and(|code| code != 0)
                || matches!(item["status"].as_str(), Some("failed" | "declined"));
            thread.result(
                id,
                Outcome {
                    failed,
                    output: item["aggregated_output"].as_str().map(str::to_owned),
                    ..Outcome::default()
                },
            );
        }
        Some("FileChange") => {
            let mut call = Call::new(id, "apply_patch", ToolKind::Edit);
            let mut names = Vec::new();
            for (path, change) in item["changes"].as_object().into_iter().flatten() {
                names.push(shown(path, cwd).to_owned());
                call = call.path(path.as_str());
                if let Some(diff) = file_change(path, change) {
                    call = call.diff(diff);
                }
            }
            thread.call(call.title(format!("Edit {}", names.join(", "))));
            let failed = matches!(item["status"].as_str(), Some("failed" | "declined"));
            let output = failed
                .then(|| item["stderr"].as_str().map(str::to_owned))
                .flatten();
            thread.result(
                id,
                Outcome {
                    failed,
                    output,
                    ..Outcome::default()
                },
            );
        }
        Some("McpToolCall") => {
            let server = item["server"].as_str().unwrap_or_default();
            let tool = item["tool"].as_str().unwrap_or_default();
            thread.call(
                Call::new(id, tool, ToolKind::Other)
                    .title(format!("{server}: {tool}"))
                    .input(&item["arguments"]),
            );
            let texts = item["result"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .chain(item["error"]["message"].as_str())
                .collect::<Vec<_>>();
            thread.result(
                id,
                Outcome {
                    failed: item["status"] == "failed",
                    output: Some(texts.join("\n")),
                    ..Outcome::default()
                },
            );
        }
        Some("Extension") if item["kind"] == "web.search" => {
            let action = &item["action"];
            let title = match action["type"].as_str() {
                Some("openPage") => format!("Open {}", action["url"].as_str().unwrap_or_default()),
                Some("findInPage") => {
                    format!(
                        "Find \"{}\"",
                        action["pattern"].as_str().unwrap_or_default()
                    )
                }
                _ => format!("Search \"{}\"", item["query"].as_str().unwrap_or_default()),
            };
            thread.call(Call::new(id, "web_search", ToolKind::Fetch).title(title));
            thread.result(id, Outcome::default());
        }
        // The operator's and the agent's messages and the reasoning are
        // read from their `response_item` records; the rest (compaction,
        // sleep, subagent activity) is not conversation.
        Some(
            "UserMessage" | "AgentMessage" | "Reasoning" | "ContextCompaction" | "Extension"
            | "SubAgentActivity" | "FunctionCallOutput",
        ) => {}
        _ => thread.skip("codex_item_unknown"),
    }
}

/// The shell script of `[shell, -lc, script]`, or the words joined.
fn command_line(command: &Value) -> String {
    let words = command
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    match words.as_slice() {
        [_, flag, script] if matches!(*flag, "-lc" | "-c") => (*script).to_owned(),
        _ => command
            .as_str()
            .map_or_else(|| words.join(" "), str::to_owned),
    }
}

fn file_change(path: &str, change: &Value) -> Option<FileDiff> {
    let target = change["move_path"].as_str().unwrap_or(path).to_owned();
    match change["type"].as_str()? {
        "add" => Some(FileDiff {
            path: target,
            before: None,
            after: change["content"].as_str()?.to_owned(),
        }),
        "delete" => Some(FileDiff {
            path: target,
            before: change["content"].as_str().map(str::to_owned),
            after: String::new(),
        }),
        "update" => {
            let (before, after) = hunks(change["unified_diff"].as_str()?);
            Some(FileDiff {
                path: target,
                before: Some(before),
                after,
            })
        }
        _ => None,
    }
}

/// The old and new sides of diff hunks, context on both.
fn hunks(diff: &str) -> (String, String) {
    let (mut before, mut after) = (String::new(), String::new());
    for line in diff.lines() {
        if line.starts_with("@@") || line.starts_with("\\ ") || line.starts_with("*** ") {
            continue;
        }
        let (sign, text) = line.split_at(line.len().min(1));
        match sign {
            "-" => push_line(&mut before, text),
            "+" => push_line(&mut after, text),
            " " | "" => {
                push_line(&mut before, text);
                push_line(&mut after, text);
            }
            _ => {}
        }
    }
    (before, after)
}

fn push_line(side: &mut String, line: &str) {
    side.push_str(line);
    side.push('\n');
}

fn legacy(kind: &str, payload: &Value, cwd: &str, thread: &mut Thread) {
    let id = payload["call_id"].as_str().unwrap_or_default();
    match kind {
        "function_call" | "custom_tool_call" | "local_shell_call" if id.is_empty() => {
            thread.unreadable("codex_tool_shape");
        }
        "function_call" => {
            let name = payload["name"].as_str().unwrap_or_default();
            let arguments = payload["arguments"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .unwrap_or(Value::Null);
            let call = match name {
                "update_plan" => {
                    thread.plan(plan(&arguments));
                    return;
                }
                "exec_command" | "shell" => {
                    let command = match &arguments["cmd"] {
                        Value::String(command) => command.clone(),
                        _ => command_line(&arguments["command"]),
                    };
                    Call::new(id, name, ToolKind::Execute).title(command)
                }
                "view_image" => {
                    let path = arguments["path"].as_str().unwrap_or_default();
                    Call::new(id, name, ToolKind::Read)
                        .title(format!("View {}", shown(path, cwd)))
                        .path(path)
                }
                _ => Call::new(id, name, ToolKind::Other),
            };
            thread.call(call.input(&arguments));
        }
        "local_shell_call" => {
            let command = command_line(&payload["action"]["command"]);
            thread.call(Call::new(id, "shell", ToolKind::Execute).title(command));
        }
        "custom_tool_call" if payload["name"] == "apply_patch" => {
            let patch = payload["input"].as_str().unwrap_or_default();
            let mut call = Call::new(id, "apply_patch", ToolKind::Edit);
            let mut names = Vec::new();
            for diff in patch_files(patch) {
                names.push(shown(&diff.path, cwd).to_owned());
                call = call.path(diff.path.clone()).diff(diff);
            }
            thread.call(call.title(format!("Edit {}", names.join(", "))));
        }
        "custom_tool_call" => {
            let name = payload["name"].as_str().unwrap_or_default();
            thread.call(Call::new(id, name, ToolKind::Other));
        }
        "function_call_output" | "custom_tool_call_output" => {
            output(id, &payload["output"], thread)
        }
        "web_search_call" => {
            let query = payload["action"]["query"].as_str().unwrap_or_default();
            let id = payload["id"].as_str().unwrap_or_default();
            thread.call(
                Call::new(id, "web_search", ToolKind::Fetch).title(format!("Search \"{query}\"")),
            );
            thread.result(id, Outcome::default());
        }
        _ => {}
    }
}

fn plan(arguments: &Value) -> Vec<Step> {
    arguments["plan"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|step| Step {
            text: step["step"].as_str().unwrap_or_default().to_owned(),
            status: match step["status"].as_str() {
                Some("in_progress") => PlanEntryStatus::InProgress,
                Some("completed") => PlanEntryStatus::Completed,
                _ => PlanEntryStatus::Pending,
            },
        })
        .collect()
}

/// A legacy call's output: a command's text says how it exited, and a
/// structured output carries its exit code beside the text.
fn output(id: &str, output: &Value, thread: &mut Thread) {
    let mut outcome = Outcome::default();
    let text = match output {
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(structured) if structured["output"].is_string() => {
                outcome.failed = structured["metadata"]["exit_code"]
                    .as_i64()
                    .is_some_and(|code| code != 0);
                structured["output"].as_str().unwrap_or_default().to_owned()
            }
            _ => text.clone(),
        },
        Value::Array(parts) => {
            let mut texts = Vec::new();
            for part in parts {
                match part["type"].as_str() {
                    Some("input_text" | "output_text") => {
                        texts.push(part["text"].as_str().unwrap_or_default());
                    }
                    Some("input_image") => {
                        outcome
                            .images
                            .extend(part["image_url"].as_str().and_then(data_url));
                    }
                    _ => {}
                }
            }
            texts.join("\n")
        }
        _ => String::new(),
    };
    let (exit, body) = exit_and_output(&text);
    if let Some(code) = exit {
        outcome.failed = code != 0;
    }
    outcome.output = Some(body.to_owned());
    thread.result(id, outcome);
}

/// `exec_command`'s text: a header naming the exit code, then `Output:`.
fn exit_and_output(text: &str) -> (Option<i64>, &str) {
    let code = text
        .lines()
        .find_map(|line| line.strip_prefix("Process exited with code "))
        .and_then(|code| code.trim().parse().ok());
    let body = match (code, text.split_once("\nOutput:\n")) {
        (Some(_), Some((_, body))) => body,
        _ => text,
    };
    (code, body)
}

/// The files of an `apply_patch` input, each with the sides its hunks show.
fn patch_files(patch: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current: Option<(FileDiff, String)> = None;
    let finish = |current: Option<(FileDiff, String)>, files: &mut Vec<FileDiff>| {
        if let Some((mut diff, body)) = current {
            let (before, after) = hunks(&body);
            diff.after = after;
            if diff.before.is_some() {
                diff.before = Some(before);
            }
            files.push(diff);
        }
    };
    for line in patch.lines() {
        let header = [
            ("*** Add File: ", None),
            ("*** Update File: ", Some(String::new())),
            ("*** Delete File: ", Some(String::new())),
        ]
        .into_iter()
        .find_map(|(prefix, before)| line.strip_prefix(prefix).map(|path| (path, before)));
        if let Some((path, before)) = header {
            finish(current.take(), &mut files);
            current = Some((
                FileDiff {
                    path: path.trim().to_owned(),
                    before,
                    after: String::new(),
                },
                String::new(),
            ));
        } else if let Some(path) = line.strip_prefix("*** Move to: ") {
            if let Some((diff, _)) = current.as_mut() {
                diff.path = path.trim().to_owned();
            }
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    finish(current, &mut files);
    files
}

/// A path as a title shows it: relative to the session's checkout when it
/// lies inside it.
fn shown<'a>(path: &'a str, cwd: &str) -> &'a str {
    if cwd.is_empty() {
        return path;
    }
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix(['/', '\\']))
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1::SessionUpdate;
    use serde_json::json;

    fn read(records: &[Value]) -> Vec<Value> {
        let header = json!({"type":"session_meta","payload":{"cwd":"/work/app"}});
        let mut thread = Thread::default();
        for record in records {
            super::record(record, &header, &mut thread);
        }
        let (updates, _) = thread.into_parts();
        updates
            .iter()
            .map(|update: &SessionUpdate| serde_json::to_value(update).unwrap())
            .collect()
    }

    fn item(kind: &str, mut payload: Value) -> Value {
        payload["type"] = json!(kind);
        json!({"type": "response_item", "payload": payload})
    }

    /// A legacy rollout's command and patch, in the shapes of
    /// `codex-rs/protocol` at `rust-v0.155.1`.
    #[test]
    fn a_legacy_rollout_draws_its_calls_and_their_outputs() {
        let updates = read(&[
            item(
                "function_call",
                json!({"call_id":"c1","name":"exec_command","arguments":"{\"cmd\":\"ls docs\"}"}),
            ),
            item(
                "function_call_output",
                json!({"call_id":"c1","output":"Chunk ID: 1\nProcess exited with code 2\nOutput:\nls: docs: No such file"}),
            ),
            item(
                "custom_tool_call",
                json!({"call_id":"c2","name":"apply_patch","input":"*** Begin Patch\n*** Add File: login.html\n+<html>\n*** Update File: /work/app/app.js\n@@\n export function greet(name) {\n-  return \"Hi\";\n+  return `Hello, ${name}!`;\n }\n*** End Patch\n"}),
            ),
            item(
                "custom_tool_call_output",
                json!({"call_id":"c2","output":"{\"output\":\"Success.\",\"metadata\":{\"exit_code\":0}}"}),
            ),
            item(
                "function_call",
                json!({"call_id":"c3","name":"update_plan","arguments":"{\"plan\":[{\"step\":\"copy the image\",\"status\":\"in_progress\"}]}"}),
            ),
        ]);
        assert_eq!(updates[0]["title"], "ls docs");
        assert_eq!(updates[0]["kind"], "execute");
        assert_eq!(updates[1]["status"], "failed");
        assert_eq!(
            updates[1]["content"][0]["content"]["text"],
            "ls: docs: No such file"
        );
        assert_eq!(updates[2]["title"], "Edit login.html, app.js");
        let diffs = updates[2]["content"].as_array().unwrap();
        assert_eq!(diffs[0]["path"], "login.html");
        assert_eq!(diffs[0]["oldText"], Value::Null);
        assert_eq!(diffs[0]["newText"], "<html>\n");
        assert_eq!(
            diffs[1]["oldText"],
            "export function greet(name) {\n  return \"Hi\";\n}\n"
        );
        assert_eq!(
            diffs[1]["newText"],
            "export function greet(name) {\n  return `Hello, ${name}!`;\n}\n"
        );
        assert_eq!(updates[3]["status"], "completed");
        assert_eq!(updates[3]["content"][0]["content"]["text"], "Success.");
        assert_eq!(updates[4]["sessionUpdate"], "plan");
        assert_eq!(updates[4]["entries"][0]["status"], "in_progress");
        assert_eq!(updates.len(), 5);
    }

    #[test]
    fn a_paginated_rollout_draws_its_items_and_not_the_calls_behind_them() {
        let header =
            json!({"type":"session_meta","payload":{"cwd":"/work/app","history_mode":"paginated"}});
        let mut thread = Thread::default();
        for record in [
            item(
                "custom_tool_call",
                json!({"call_id":"c1","name":"exec","input":"tools.exec_command({cmd:'ls'})"}),
            ),
            item(
                "custom_tool_call_output",
                json!({"call_id":"c1","output":[{"type":"input_text","text":"Script completed"}]}),
            ),
            json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","id":"e1","command":["/bin/zsh","-lc","ls"],"status":"completed","aggregated_output":"app.js\n","exit_code":0}}}),
        ] {
            super::record(&record, &header, &mut thread);
        }
        let (updates, _) = thread.into_parts();
        let updates = updates
            .iter()
            .map(|update| serde_json::to_value(update).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0]["title"], "ls");
        assert_eq!(updates[1]["content"][0]["content"]["text"], "app.js\n");
    }
}
