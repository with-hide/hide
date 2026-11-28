//! Antigravity CLI (agy) transcripts: one step per line. A `PLANNER_RESPONSE`
//! step holds the model's thinking, its text and its tool calls; each call's
//! result follows as a `GENERIC` step, in the order of the calls and with no
//! call id (with-hide/hide-private#379), so results are paired by order
//! within the turn and not at all when their count differs (D-10). A read
//! image is saved beside the transcript and named by its `file:` URI.

use agent_client_protocol_schema::v1::ToolKind;
use hide_agent_adapter::HookDialect;
use serde_json::Value;

use super::build::{AnswerKey, Call, FileDiff, Origin, Outcome, Part, Thread};
use super::{Lines, Session};

pub(super) const LINES: Lines = Lines {
    header: None,
    record: |record_, session, thread| record(record_, session, thread),
    // A page never begins between a call and its results.
    may_start_page: |record| record["type"] != "GENERIC",
};

const REQUEST_OPEN: &str = "<USER_REQUEST>";
const REQUEST_CLOSE: &str = "</USER_REQUEST>";

fn record(record: &Value, _: &Session<'_>, thread: &mut Thread) {
    let id = record["step_index"]
        .as_u64()
        .map(|step| format!("step-{step}"));
    let id = id.as_deref();
    let content = record["content"].as_str().unwrap_or_default();
    match record["type"].as_str() {
        Some("USER_INPUT") => {
            thread.end_turn();
            // A step Hide's hook injected (`injectSteps`) arrives as a user
            // input whose source is the SDK.
            let origin = if record["source"] == "SYSTEM_SDK" {
                Origin::Injected
            } else {
                Origin::Operator
            };
            user(id, origin, content, thread);
        }
        Some("SYSTEM_MESSAGE") => {
            thread.end_turn();
            thread.user(id, Origin::Injected, [Part::Text(content)]);
        }
        Some("PLANNER_RESPONSE") => {
            thread.end_turn();
            thread.thought(id, record["thinking"].as_str().unwrap_or_default());
            thread.agent(id, Part::Text(content));
            for (index, call) in record["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let call_id = format!("{}-{index}", id.unwrap_or("step"));
                thread.call_awaiting_result(tool_call(call_id, call));
            }
        }
        Some("GENERIC") => thread.result_in_order(outcome(record, content)),
        _ => {}
    }
}

/// agy wraps the operator's words in `<USER_REQUEST>` and appends its own
/// notices (settings changes and the like) after them.
fn user(id: Option<&str>, origin: Origin, content: &str, thread: &mut Thread) {
    let Some((request, rest)) = content
        .strip_prefix(REQUEST_OPEN)
        .and_then(|inner| inner.split_once(REQUEST_CLOSE))
    else {
        thread.user(id, origin, [Part::Text(content)]);
        return;
    };
    thread.user(id, origin, [Part::Text(request.trim())]);
    if !rest.trim().is_empty() {
        thread.user(id, Origin::Injected, [Part::Text(rest.trim())]);
    }
}

fn tool_call(id: String, call: &Value) -> Call {
    let name = call["name"].as_str().unwrap_or_default();
    let args = &call["args"];
    let text = |key: &str| args[key].as_str().unwrap_or_default();
    let path = args["Path"].as_str().or(args["AbsolutePath"].as_str());
    let drawn = match name {
        "run_command" => Call::new(id, name, ToolKind::Execute)
            .title(text("CommandLine"))
            .shell(HookDialect::Antigravity),
        "read_file" | "view_file" => {
            Call::new(id, name, ToolKind::Read).title(format!("Read {}", path.unwrap_or_default()))
        }
        "write_file" | "write_to_file" => {
            let mut call = Call::new(id, name, ToolKind::Edit)
                .title(format!("Write {}", path.unwrap_or_default()));
            if let Some(path) = path {
                call = call.diff(FileDiff {
                    path: path.to_owned(),
                    before: None,
                    after: text("Content").to_owned(),
                });
            }
            call
        }
        "edit_file" | "replace_file_content" => {
            let mut call = Call::new(id, name, ToolKind::Edit)
                .title(format!("Edit {}", path.unwrap_or_default()));
            if let Some(path) = path {
                call = call.diff(FileDiff {
                    path: path.to_owned(),
                    before: Some(text("Target").to_owned()),
                    after: text("Replacement").to_owned(),
                });
            }
            call
        }
        "list_dir" | "grep_search" | "find_by_name" => Call::new(id, name, ToolKind::Search),
        "read_url_content" | "search_web" => Call::new(id, name, ToolKind::Fetch),
        _ => Call::new(id, name, ToolKind::Other),
    };
    let drawn = match path {
        Some(path) => drawn.path(path),
        None => drawn,
    };
    drawn.input(args)
}

/// A result step: agy's timing header, then the tool's text; a command's
/// text says how it exited, and a read image is saved as a file.
fn outcome(record: &Value, content: &str) -> Outcome<'static> {
    let body = content
        .lines()
        .skip_while(|line| line.starts_with("Created At: ") || line.starts_with("Completed At: "))
        .collect::<Vec<_>>()
        .join("\n");
    let exit = body
        .lines()
        .find_map(|line| line.strip_prefix("Command exited with code "))
        .and_then(|code| code.trim_end_matches('.').parse::<i64>().ok());
    let output = match (exit, body.split_once("Command output:\n")) {
        (Some(_), Some((_, output))) => output.to_owned(),
        (Some(_), None) => String::new(),
        (None, _) => body,
    };
    let images = record["media"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|media| {
            Some(Part::ImagePath {
                path: media["uri"].as_str()?.strip_prefix("file://")?.to_owned(),
                mime: media["mime_type"].as_str().map(str::to_owned),
            })
        })
        .collect();
    Outcome {
        failed: exit.is_some_and(|exit| exit != 0) || record["status"] == "ERROR",
        running: false,
        answers: answers(&output),
        output: Some(output),
        images,
        diffs: Vec::new(),
    }
}

/// What the operator answered an `ask_question` call: its result writes one
/// `A<n>: <answer>` line per question, from `A1`. Any other result holds
/// none, and the builder keeps answers only for a call that asked.
fn answers(output: &str) -> Vec<(AnswerKey, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (place, answer) = line.strip_prefix('A')?.split_once(": ")?;
            let place = place.parse::<usize>().ok()?.checked_sub(1)?;
            Some((AnswerKey::Index(place), answer.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1::SessionUpdate;
    use serde_json::json;

    /// agy 1.5's `ask_question` step and the step that answers it, as
    /// `transcript_full.jsonl` holds them.
    #[test]
    fn an_ask_question_step_carries_its_questions_and_its_result_the_answers() {
        let session = Session {
            header: &Value::Null,
            path: std::path::Path::new("transcript_full.jsonl"),
        };
        let mut thread = Thread::default();
        for record_ in [
            json!({"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","tool_calls":[{"name":"ask_question","args":{"questions":[
                {"is_multi_select":false,"options":["(Recommended) Red","Green"],"question":"Which color?"}],"toolAction":"Asking"}}]}),
            json!({"step_index":4,"source":"MODEL","type":"GENERIC","status":"DONE","content":"Created At: 2026-11-23T00:11:02+09:00\nCompleted At: 2026-11-23T00:11:08+09:00\nA1: Green"}),
        ] {
            record(&record_, &session, &mut thread);
        }
        thread.end_turn();
        let updates = thread
            .into_parts()
            .0
            .iter()
            .map(|update: &SessionUpdate| serde_json::to_value(update).unwrap())
            .collect::<Vec<_>>();
        let call = updates.iter().find(|update| update["sessionUpdate"] == "tool_call").unwrap();
        assert_eq!(
            call["_meta"]["hide"]["question"],
            json!([{"text":"Which color?","choices":["(Recommended) Red","Green"]}])
        );
        let result = updates.iter().find(|update| update["sessionUpdate"] == "tool_call_update").unwrap();
        assert_eq!(result["_meta"]["hide"]["answers"], json!(["Green"]));
    }
}
