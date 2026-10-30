//! Grok's `updates.jsonl` already holds ACP `session/update` notifications,
//! one envelope per line (`grok.rs` owns the file). Each update is read with
//! the official ACP types and drawn again through the builder, so Grok's
//! thread gets the same caps, paths and wrappers as every other agent's.
//! Grok's own extension updates (`_x.ai/session/update`: hooks, retries,
//! usage) are not conversation.

use agent_client_protocol_schema::v1::{
    ContentBlock, PlanEntryStatus, SessionUpdate, ToolCallContent, ToolCallStatus, ToolKind,
};
use serde_json::Value;

use super::Lines;
use super::build::{Call, FileDiff, Origin, Outcome, Part, Step, Thread};

pub(super) const LINES: Lines = Lines {
    header: None,
    record: |record_, _, thread| record(record_, thread),
    may_start_page: |_| true,
};

fn record(record: &Value, thread: &mut Thread) {
    if record["method"] != "session/update" {
        return;
    }
    let update = match serde_json::from_value::<SessionUpdate>(record["params"]["update"].clone()) {
        Ok(update) => update,
        Err(_) => {
            thread.unreadable("grok_update_shape");
            return;
        }
    };
    match &update {
        SessionUpdate::UserMessageChunk(chunk) => {
            thread.user(None, Origin::Operator, part(&chunk.content));
        }
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let Some(part) = part(&chunk.content) {
                thread.agent(None, part);
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = &chunk.content {
                thread.thought(None, &text.text);
            }
        }
        SessionUpdate::ToolCall(call) => {
            // The to-do list draws a plan of its own (B5).
            if call.title == "todo_write" || is_todo(call.raw_input.as_ref()) {
                return;
            }
            let mut drawn = Call::new(
                call.tool_call_id.0.to_string(),
                call.name.clone().unwrap_or_else(|| call.title.clone()),
                call.kind,
            )
            .title(call.title.clone());
            if let Some(input) = &call.raw_input {
                drawn = drawn.input(input);
            }
            for location in &call.locations {
                drawn = drawn.path(location.path.to_string_lossy().into_owned());
            }
            for diff in diffs(&call.content) {
                drawn = drawn.diff(diff);
            }
            thread.call(drawn);
        }
        SessionUpdate::ToolCallUpdate(update) => {
            let fields = &update.fields;
            if is_todo(fields.raw_input.as_ref())
                || fields
                    .raw_output
                    .as_ref()
                    .is_some_and(|output| output["type"] == "Todo")
            {
                return;
            }
            let content = fields.content.as_deref().unwrap_or_default();
            // Grok states a call's title, kind and file change in its first
            // update; that refines the call, and only a status ends it.
            if fields.title.is_some() || fields.kind.is_some() || fields.locations.is_some() {
                let id = update.tool_call_id.0.to_string();
                let title = fields.title.clone().unwrap_or_default();
                let mut drawn =
                    Call::new(id, title.clone(), fields.kind.unwrap_or(ToolKind::Other))
                        .title(title);
                if let Some(input) = &fields.raw_input {
                    drawn = drawn.input(input);
                }
                for location in fields.locations.iter().flatten() {
                    drawn = drawn.path(location.path.to_string_lossy().into_owned());
                }
                for diff in diffs(content) {
                    drawn = drawn.diff(diff);
                }
                thread.call(drawn);
                if fields.status.is_none() {
                    return;
                }
            }
            let texts = content
                .iter()
                .filter_map(|item| match item {
                    ToolCallContent::Content(content) => match &content.content {
                        ContentBlock::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect::<Vec<_>>();
            let images = content
                .iter()
                .filter_map(|item| match item {
                    ToolCallContent::Content(content) => match &content.content {
                        ContentBlock::Image(_) => part(&content.content),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let status = fields.status;
            thread.result(
                &update.tool_call_id.0,
                Outcome {
                    failed: status == Some(ToolCallStatus::Failed),
                    running: matches!(
                        status,
                        Some(ToolCallStatus::Pending | ToolCallStatus::InProgress) | None
                    ),
                    output: (!texts.is_empty()).then(|| texts.join("\n")),
                    images,
                    diffs: diffs(content),
                },
            );
        }
        SessionUpdate::Plan(plan) => thread.plan(plan.entries.iter().map(|entry| Step {
            text: entry.content.clone(),
            status: match entry.status {
                PlanEntryStatus::InProgress => PlanEntryStatus::InProgress,
                PlanEntryStatus::Completed => PlanEntryStatus::Completed,
                _ => PlanEntryStatus::Pending,
            },
        })),
        _ => {}
    }
}

fn is_todo(input: Option<&Value>) -> bool {
    input.is_some_and(|input| input["variant"] == "TodoWrite")
}

fn part(block: &ContentBlock) -> Option<Part<'_>> {
    match block {
        ContentBlock::Text(text) => Some(Part::Text(&text.text)),
        ContentBlock::Image(image) => Some(Part::Image {
            data: &image.data,
            mime: &image.mime_type,
        }),
        ContentBlock::ResourceLink(link) => {
            link.uri
                .strip_prefix("file://")
                .map(|path| Part::ImagePath {
                    path: path.to_owned(),
                    mime: link.mime_type.as_deref(),
                })
        }
        _ => None,
    }
}

/// A file change Grok states as an ACP diff; an empty old side is a new
/// file, as Grok writes one.
fn diffs(content: &[ToolCallContent]) -> Vec<FileDiff> {
    content
        .iter()
        .filter_map(|item| match item {
            ToolCallContent::Diff(diff) => Some(FileDiff {
                path: diff.path.to_string_lossy().into_owned(),
                before: diff.old_text.clone().filter(|text| !text.is_empty()),
                after: diff.new_text.clone(),
            }),
            _ => None,
        })
        .collect()
}
