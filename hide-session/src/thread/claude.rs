//! Claude Code's transcript records: one `user` or `assistant` record per
//! message block, a tool's result in the next `user` record with the call's
//! id. Tool titles and kinds follow claude-agent-acp's reporters (D-04).

use agent_client_protocol_schema::v1::{PlanEntryStatus, ToolKind};
use serde_json::Value;

use super::Lines;
use super::build::{Call, FileDiff, Origin, Outcome, Part, Step, Thread};

pub(super) const LINES: Lines = Lines {
    record,
    may_start_page: |_| true,
};

fn record(record: &Value, thread: &mut Thread) {
    let role = match record["type"].as_str() {
        Some(role @ ("user" | "assistant")) => role,
        // Summaries, snapshots, hook attachments and the like are not
        // conversation.
        _ => return,
    };
    // A subagent's records in its parent's file belong to the subagent's
    // own thread.
    if record["isSidechain"] == true {
        thread.skip("sidechain");
        return;
    }
    let message = &record["message"];
    let cwd = record["cwd"].as_str().unwrap_or_default();
    match (role, &message["content"]) {
        ("user", Value::String(text)) => {
            thread.user(record["uuid"].as_str(), origin(record), [Part::Text(text)]);
        }
        ("user", Value::Array(blocks)) => user_blocks(record, blocks, thread),
        ("assistant", Value::Array(blocks)) => {
            let id = message["id"].as_str().or(record["uuid"].as_str());
            for block in blocks {
                assistant_block(id, block, cwd, thread);
            }
        }
        _ => thread.unreadable("claude_record_shape"),
    }
}

/// Provider metadata that proves a message is not the operator's words.
fn origin(record: &Value) -> Origin {
    if record["isMeta"] == true || record["isCompactSummary"] == true {
        Origin::Injected
    } else {
        Origin::Operator
    }
}

fn user_blocks(record: &Value, blocks: &[Value], thread: &mut Thread) {
    let id = record["uuid"].as_str();
    let mut parts = Vec::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => parts.push(Part::Text(block["text"].as_str().unwrap_or_default())),
            Some("image") => parts.extend(image(block)),
            Some("tool_result") => result(record, block, thread),
            _ => thread.skip("claude_block_unknown"),
        }
    }
    if !parts.is_empty() {
        thread.user(id, origin(record), parts);
    }
}

fn image(block: &Value) -> Option<Part<'_>> {
    let source = &block["source"];
    Some(Part::Image {
        data: source["data"].as_str()?,
        mime: source["media_type"].as_str()?,
    })
}

fn assistant_block(id: Option<&str>, block: &Value, cwd: &str, thread: &mut Thread) {
    match block["type"].as_str() {
        Some("text") => thread.agent(id, Part::Text(block["text"].as_str().unwrap_or_default())),
        Some("thinking") => thread.thought(id, block["thinking"].as_str().unwrap_or_default()),
        Some("redacted_thinking") => {}
        Some("tool_use") => tool_use(block, cwd, thread),
        _ => thread.skip("claude_block_unknown"),
    }
}

fn tool_use(block: &Value, cwd: &str, thread: &mut Thread) {
    let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) else {
        thread.unreadable("claude_tool_shape");
        return;
    };
    let input = &block["input"];
    let text = |key: &str| input[key].as_str().unwrap_or_default();
    let call = match name {
        // The to-do list is the plan, not a call (B5).
        "TodoWrite" => {
            thread.plan(todos(input));
            return;
        }
        "Bash" => Call::new(id, name, ToolKind::Execute).title(text("command")),
        "Read" => {
            let path = text("file_path");
            Call::new(id, name, ToolKind::Read)
                .title(format!("Read {}", shown(path, cwd)))
                .path(path)
        }
        "Write" => {
            let path = text("file_path");
            Call::new(id, name, ToolKind::Edit)
                .title(format!("Write {}", shown(path, cwd)))
                .path(path)
                .diff(FileDiff {
                    path: path.to_owned(),
                    before: None,
                    after: text("content").to_owned(),
                })
        }
        "Edit" => {
            let path = text("file_path");
            Call::new(id, name, ToolKind::Edit)
                .title(format!("Edit {}", shown(path, cwd)))
                .path(path)
                .diff(FileDiff {
                    path: path.to_owned(),
                    before: Some(text("old_string").to_owned()),
                    after: text("new_string").to_owned(),
                })
        }
        "Glob" | "Grep" => Call::new(id, name, ToolKind::Search)
            .title(format!("{name} {}", text("pattern")).trim_end().to_owned()),
        "WebFetch" => Call::new(id, name, ToolKind::Fetch).title(format!("Fetch {}", text("url"))),
        "WebSearch" => {
            Call::new(id, name, ToolKind::Fetch).title(format!("Search \"{}\"", text("query")))
        }
        "Task" | "Agent" => {
            let description = text("description");
            Call::new(id, name, ToolKind::Think).title(if description.is_empty() {
                name
            } else {
                description
            })
        }
        _ => Call::new(id, name, ToolKind::Other),
    };
    thread.call(call.input(input));
}

fn todos(input: &Value) -> Vec<Step> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|todo| {
            let status = match todo["status"].as_str() {
                Some("in_progress") => PlanEntryStatus::InProgress,
                Some("completed") => PlanEntryStatus::Completed,
                _ => PlanEntryStatus::Pending,
            };
            let active = todo["activeForm"].as_str();
            let text = match active {
                Some(active) if status == PlanEntryStatus::InProgress => active,
                _ => todo["content"].as_str().unwrap_or_default(),
            };
            Step {
                text: text.to_owned(),
                status,
            }
        })
        .collect()
}

fn result(record: &Value, block: &Value, thread: &mut Thread) {
    let Some(id) = block["tool_use_id"].as_str() else {
        thread.unreadable("claude_tool_shape");
        return;
    };
    let structured = &record["toolUseResult"];
    // The to-do list's answer belongs to the plan, which drew no call.
    if structured.get("newTodos").is_some() {
        return;
    }
    let failed = block["is_error"] == true;
    let mut outcome = Outcome {
        failed,
        ..Outcome::default()
    };
    let mut texts = Vec::new();
    match &block["content"] {
        Value::String(text) => texts.push(text.as_str()),
        Value::Array(parts) => {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => texts.push(part["text"].as_str().unwrap_or_default()),
                    Some("image") => outcome.images.extend(image(part)),
                    _ => thread.skip("claude_block_unknown"),
                }
            }
        }
        _ => {}
    }
    // A file change shows its diff; its result text only confirms it, so
    // only a failure's text is kept. A write over an existing file gains
    // the text it replaced.
    let change = matches!(structured["type"].as_str(), Some("create" | "update"))
        || structured.get("structuredPatch").is_some();
    if !change || failed {
        outcome.output = Some(texts.join("\n"));
    }
    if structured["type"] == "update"
        && let (Some(path), Some(before), Some(after)) = (
            structured["filePath"].as_str(),
            structured["originalFile"].as_str(),
            structured["content"].as_str(),
        )
    {
        outcome.diffs.push(FileDiff {
            path: path.to_owned(),
            before: Some(before.to_owned()),
            after: after.to_owned(),
        });
    }
    thread.result(id, outcome);
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
