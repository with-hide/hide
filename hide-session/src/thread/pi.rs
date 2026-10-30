//! Pi's session files and omp's, which keep Pi's shape: a `session` record
//! naming the checkout, then `message` records whose `message.role` is
//! `user`, `assistant` (text, thinking and `toolCall` blocks) or
//! `toolResult` (the call's id, its tool's name, its content and `details`).
//! omp keeps an image's bytes beside its sessions and records a
//! `blob:sha256:<hash>` reference instead (`~/.omp/agent/blobs`). Titles
//! and kinds follow pi-acp (D-04).

use std::path::Path;

use agent_client_protocol_schema::v1::{PlanEntryStatus, ToolKind};
use serde_json::Value;

use super::build::{Call, FileDiff, Origin, Outcome, Part, Step, Thread};
use super::{Lines, Session};

pub(super) const LINES: Lines = Lines {
    header: Some(|record| record["type"] == "session"),
    record,
    may_start_page: |_| true,
};

fn record(record: &Value, session: &Session<'_>, thread: &mut Thread) {
    if record["type"] != "message" {
        return;
    }
    let message = &record["message"];
    let id = record["id"].as_str();
    let cwd = session.header["cwd"].as_str().unwrap_or_default();
    match (message["role"].as_str(), &message["content"]) {
        (Some("user"), Value::String(text)) => {
            thread.user(id, Origin::Operator, [Part::Text(text)])
        }
        (Some("user"), Value::Array(blocks)) => {
            let parts = blocks
                .iter()
                .filter_map(|block| part(block, session.path))
                .collect::<Vec<_>>();
            thread.user(id, Origin::Operator, parts);
        }
        (Some("assistant"), Value::Array(blocks)) => {
            for block in blocks {
                match block["type"].as_str() {
                    Some("text") => {
                        thread.agent(id, Part::Text(block["text"].as_str().unwrap_or_default()))
                    }
                    Some("thinking") => {
                        thread.thought(id, block["thinking"].as_str().unwrap_or_default())
                    }
                    Some("toolCall") => tool_call(block, cwd, thread),
                    _ => thread.skip("pi_block_unknown"),
                }
            }
        }
        (Some("toolResult"), Value::Array(blocks)) => result(message, blocks, session, thread),
        (Some("user" | "assistant" | "toolResult"), _) => thread.unreadable("pi_message_shape"),
        // Pi's own records (bash runs the operator typed, custom messages,
        // compaction summaries) are not the conversation drawn here.
        _ => {}
    }
}

/// A message block as a thread part. Pi names an attached file in an empty
/// `<file name="…"></file>` wrapper before the operator's words; the image
/// block itself shows the file.
fn part<'a>(block: &'a Value, session_path: &Path) -> Option<Part<'a>> {
    match block["type"].as_str()? {
        "text" => {
            let text = block["text"].as_str()?;
            Some(Part::Text(strip_file_wrappers(text)))
        }
        "image" => image(block, session_path),
        _ => None,
    }
}

fn strip_file_wrappers(mut text: &str) -> &str {
    while let Some(rest) = text.strip_prefix("<file name=\"") {
        let Some(end) = rest.find("\"></file>") else {
            break;
        };
        text = rest[end + "\"></file>".len()..].trim_start_matches('\n');
    }
    text
}

fn image<'a>(block: &'a Value, session_path: &Path) -> Option<Part<'a>> {
    let data = block["data"].as_str()?;
    let mime = block["mimeType"].as_str()?;
    match data.strip_prefix("blob:sha256:") {
        Some(hash) => blob(session_path, hash).map(|path| Part::ImagePath {
            path,
            mime: Some(mime),
        }),
        None => Some(Part::Image { data, mime }),
    }
}

/// omp's blob file for `hash`: `<agent folder>/blobs/<hash>`, the agent
/// folder holding `sessions/<checkout>/<session file>`.
fn blob(session_path: &Path, hash: &str) -> Option<String> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let agent = session_path.ancestors().nth(3)?;
    Some(agent.join("blobs").join(hash).to_str()?.to_owned())
}

fn tool_call(block: &Value, cwd: &str, thread: &mut Thread) {
    let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) else {
        thread.unreadable("pi_tool_shape");
        return;
    };
    let arguments = &block["arguments"];
    let path = arguments["path"].as_str().map(|path| absolute(path, cwd));
    let shown = arguments["path"].as_str().unwrap_or_default();
    let call = match name {
        // The to-do list's result draws the plan (B5).
        "todo" => return,
        "bash" => Call::new(id, name, ToolKind::Execute)
            .title(arguments["command"].as_str().unwrap_or_default()),
        "read" => Call::new(id, name, ToolKind::Read).title(format!("Read {shown}")),
        "write" => {
            let mut call = Call::new(id, name, ToolKind::Edit).title(format!("Write {shown}"));
            if let (Some(path), Some(content)) = (&path, arguments["content"].as_str()) {
                call = call.diff(FileDiff {
                    path: path.clone(),
                    before: None,
                    after: content.to_owned(),
                });
            }
            call
        }
        "edit" => {
            let mut call = Call::new(id, name, ToolKind::Edit).title(match shown {
                "" => "Edit".to_owned(),
                shown => format!("Edit {shown}"),
            });
            // Pi states each replacement; omp's patch is drawn from the
            // file sides its result reports.
            for edit in arguments["edits"].as_array().into_iter().flatten() {
                if let (Some(path), Some(before), Some(after)) =
                    (&path, edit["oldText"].as_str(), edit["newText"].as_str())
                {
                    call = call.diff(FileDiff {
                        path: path.clone(),
                        before: Some(before.to_owned()),
                        after: after.to_owned(),
                    });
                }
            }
            call
        }
        "grep" | "find" | "glob" | "ls" => Call::new(id, name, ToolKind::Search),
        _ => Call::new(id, name, ToolKind::Other),
    };
    let call = match path {
        Some(path) => call.path(path),
        None => call,
    };
    thread.call(call.input(arguments));
}

fn result(message: &Value, blocks: &[Value], session: &Session<'_>, thread: &mut Thread) {
    let Some(id) = message["toolCallId"].as_str() else {
        thread.unreadable("pi_tool_shape");
        return;
    };
    let tool = message["toolName"].as_str().unwrap_or_default();
    let details = &message["details"];
    if tool == "todo" {
        thread.plan(todos(details));
        return;
    }
    let failed = message["isError"] == true;
    let mut outcome = Outcome {
        failed,
        ..Outcome::default()
    };
    let mut texts = Vec::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => texts.push(block["text"].as_str().unwrap_or_default()),
            Some("image") => outcome.images.extend(image(block, session.path)),
            _ => {}
        }
    }
    // A file change shows its diff; its result text only confirms it.
    if !matches!(tool, "edit" | "write") || failed {
        outcome.output = Some(texts.join("\n"));
    }
    if let (Some(path), Some(before), Some(after)) = (
        details["path"].as_str(),
        details["oldText"].as_str(),
        details["newText"].as_str(),
    ) {
        outcome.diffs.push(FileDiff {
            path: path.to_owned(),
            before: Some(before.to_owned()),
            after: after.to_owned(),
        });
    }
    thread.result(id, outcome);
}

/// omp's to-do list as its result reports it: phases of tasks.
fn todos(details: &Value) -> Vec<Step> {
    details["phases"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|phase| phase["tasks"].as_array().into_iter().flatten())
        .map(|task| Step {
            text: task["content"].as_str().unwrap_or_default().to_owned(),
            status: match task["status"].as_str() {
                Some("in_progress") => PlanEntryStatus::InProgress,
                Some("completed" | "done") => PlanEntryStatus::Completed,
                _ => PlanEntryStatus::Pending,
            },
        })
        .collect()
}

/// A tool's path in the checkout's terms: a relative path is the session
/// checkout's.
fn absolute(path: &str, cwd: &str) -> String {
    if Path::new(path).is_absolute() || cwd.is_empty() {
        path.to_owned()
    } else {
        Path::new(cwd).join(path).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attachment_wrapper_is_not_the_operators_words() {
        assert_eq!(
            strip_file_wrappers("<file name=\"/work/app/mock.png\"></file>\n이거 봐"),
            "이거 봐"
        );
        assert_eq!(
            strip_file_wrappers("<file name=\"x\">kept</file>"),
            "<file name=\"x\">kept</file>"
        );
    }

    #[test]
    fn an_omp_blob_is_named_beside_its_sessions() {
        let session = Path::new("/work/home/.omp/agent/sessions/-work-app/1_abc.jsonl");
        let hash = "3203ebae6246af7400f28a78c7ae071feb6735f393367ce13a1c6bceb3c5822c";
        let expected = Path::new("/work/home/.omp/agent/blobs").join(hash);
        assert_eq!(blob(session, hash).as_deref(), expected.to_str());
        assert_eq!(blob(session, "../../etc/passwd"), None);
    }
}
