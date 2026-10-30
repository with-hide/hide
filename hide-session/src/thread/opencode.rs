//! OpenCode keeps every session in one SQLite database (`opencode.rs` owns
//! where it is, the read lock and the proof that a session is the root one
//! of this checkout). 1.x writes a `message` row per message and a `part`
//! row per piece of it; 2.x writes one `session_message` row per message,
//! the pieces inside its data. Both are drawn by the same piece reader;
//! titles and kinds follow OpenCode's own ACP server (`opencode acp`, D-04).
//!
//! A page is a run of whole messages, newest first, within the page's byte
//! cap; its cursor is the index of its first message in the session.

use agent_client_protocol_schema::v1::{PlanEntryStatus, ToolKind};
use rusqlite::{Connection, params};
use serde_json::Value;

use super::build::{Call, FileDiff, Origin, Outcome, Part, Step, Thread};
use super::{THREAD_PAGE_BYTES, ThreadCursor, ThreadPage};
use crate::opencode::{Owner, Schema, prove, sql_refusal, with_read};
use crate::{ReadRefusal, SessionReadScope};

/// The most messages one page holds, whatever their size.
const MESSAGES_PER_PAGE: u64 = 200;
/// The most pieces one 1.x message is read with; past it the rest are
/// counted as `part_capacity`.
const PARTS_PER_MESSAGE: i64 = 256;

/// One message as its row holds it, before it is drawn.
struct Message {
    id: String,
    /// The message's data, `None` when it is past the page cap.
    data: Option<Value>,
    /// 1.x: the message's pieces, each `None` past the page cap.
    parts: Vec<Option<Value>>,
    bytes: u64,
}

pub(super) fn read(
    home: &std::path::Path,
    scope: &SessionReadScope,
    before: Option<&ThreadCursor>,
) -> Result<ThreadPage, ReadRefusal> {
    let incarnation = format!("opencode:{}", scope.id);
    if before.is_some_and(|cursor| cursor.incarnation != incarnation) {
        return Err(ReadRefusal::from("thread_cursor_stale"));
    }
    with_read(home, |connection, schema| {
        prove(
            connection,
            schema,
            &scope.id,
            Owner::Root {
                cwd: Some(&scope.cwd),
            },
        )?;
        let (start, messages) = page(connection, schema, &scope.id, before.map(|c| c.position))?;
        let mut thread = Thread::default();
        for message in &messages {
            draw(message, schema, &mut thread);
        }
        if thread.nothing_readable() {
            return Err(ReadRefusal::from("thread_records_unreadable"));
        }
        let (updates, skipped) = thread.into_parts();
        Ok(ThreadPage {
            updates,
            before: (start > 0).then(|| ThreadCursor {
                position: start,
                incarnation: incarnation.clone(),
            }),
            skipped,
        })
    })?
}

/// The messages that end at index `end` (the newest without one), as many
/// as fit the page, and the index of the first.
fn page(
    connection: &Connection,
    schema: Schema,
    session: &str,
    end: Option<u64>,
) -> Result<(u64, Vec<Message>), ReadRefusal> {
    let (count, rows) = match schema {
        Schema::V1 => (
            "SELECT count(*) FROM message WHERE session_id = ?1",
            "SELECT id, CASE WHEN length(data) <= ?2 THEN data END, length(data) \
             + (SELECT coalesce(sum(length(data)), 0) FROM part WHERE part.message_id = message.id) \
             FROM message WHERE session_id = ?1 ORDER BY time_created, id LIMIT ?3 OFFSET ?4",
        ),
        Schema::V2 => (
            "SELECT count(*) FROM session_message WHERE session_id = ?1",
            "SELECT id, CASE WHEN length(data) <= ?2 THEN json_set(data, '$.type', type) END, \
             length(data) FROM session_message WHERE session_id = ?1 ORDER BY seq LIMIT ?3 OFFSET ?4",
        ),
    };
    let total: u64 = connection
        .query_row(count, params![session], |row| row.get::<_, i64>(0))
        .map_err(|error| sql_refusal(&error))?
        .try_into()
        .unwrap_or(0);
    let end = end.unwrap_or(total).min(total);
    let window = end.saturating_sub(MESSAGES_PER_PAGE);
    let mut statement = connection
        .prepare(rows)
        .map_err(|error| sql_refusal(&error))?;
    let mut messages = statement
        .query_map(
            params![
                session,
                THREAD_PAGE_BYTES as i64,
                (end - window) as i64,
                window as i64
            ],
            |row| {
                Ok(Message {
                    id: row.get(0)?,
                    data: row
                        .get::<_, Option<String>>(1)?
                        .and_then(|data| serde_json::from_str(&data).ok()),
                    parts: Vec::new(),
                    bytes: row.get::<_, i64>(2)?.try_into().unwrap_or(u64::MAX),
                })
            },
        )
        .map_err(|error| sql_refusal(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sql_refusal(&error))?;
    // Newest first until the page is full; the newest message always comes,
    // so a page past the cap still moves the cursor.
    let mut bytes = 0u64;
    let mut kept = messages.len();
    for (index, message) in messages.iter().enumerate().rev() {
        if kept < messages.len() && bytes.saturating_add(message.bytes) > THREAD_PAGE_BYTES {
            break;
        }
        bytes = bytes.saturating_add(message.bytes);
        kept = index;
    }
    let start = window + kept as u64;
    messages.drain(..kept);
    if schema == Schema::V1 {
        for message in &mut messages {
            message.parts = parts(connection, &message.id)?;
        }
    }
    Ok((start, messages))
}

fn parts(connection: &Connection, message: &str) -> Result<Vec<Option<Value>>, ReadRefusal> {
    let mut statement = connection
        .prepare(
            "SELECT CASE WHEN length(data) <= ?2 THEN data END FROM part \
             WHERE message_id = ?1 ORDER BY id LIMIT ?3",
        )
        .map_err(|error| sql_refusal(&error))?;
    statement
        .query_map(
            params![message, THREAD_PAGE_BYTES as i64, PARTS_PER_MESSAGE + 1],
            |row| {
                Ok(row
                    .get::<_, Option<String>>(0)?
                    .and_then(|data| serde_json::from_str(&data).ok()))
            },
        )
        .map_err(|error| sql_refusal(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sql_refusal(&error))
}

fn draw(message: &Message, schema: Schema, thread: &mut Thread) {
    let id = Some(message.id.as_str());
    let Some(data) = &message.data else {
        thread.skip("line_capacity");
        return;
    };
    match schema {
        Schema::V1 => {
            let role = data["role"].as_str();
            if !matches!(role, Some("user" | "assistant")) {
                thread.unreadable("opencode_message_shape");
                return;
            }
            if message.parts.len() as i64 > PARTS_PER_MESSAGE {
                thread.skip("part_capacity");
            }
            let user = role == Some("user");
            for part in message.parts.iter().take(PARTS_PER_MESSAGE as usize) {
                match part {
                    Some(part) => piece(id, part, user, thread),
                    None => thread.skip("line_capacity"),
                }
            }
        }
        Schema::V2 => match data["type"].as_str() {
            Some("user") => {
                let mut parts = Vec::new();
                if let Some(text) = data["text"].as_str() {
                    parts.push(Part::Text(text));
                }
                parts.extend(
                    data["files"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(file),
                );
                let origin = if data["synthetic"] == true {
                    Origin::Injected
                } else {
                    Origin::Operator
                };
                thread.user(id, origin, parts);
            }
            Some("assistant") => match data["content"].as_array() {
                Some(content) => content
                    .iter()
                    .for_each(|item| piece(id, item, false, thread)),
                None => thread.unreadable("opencode_message_shape"),
            },
            // Turn ends, compactions and the like are not conversation.
            _ => {}
        },
    }
}

/// One piece of a message: a 1.x part or an item of a 2.x message.
fn piece(id: Option<&str>, piece: &Value, user: bool, thread: &mut Thread) {
    match piece["type"].as_str() {
        Some("text") => {
            let text = piece["text"].as_str().unwrap_or_default();
            if user {
                // A part OpenCode or a plugin wrote into the operator's
                // message, such as Hide's Memory receipt.
                let origin = if piece["synthetic"] == true {
                    Origin::Injected
                } else {
                    Origin::Operator
                };
                thread.user(id, origin, [Part::Text(text)]);
            } else {
                thread.agent(id, Part::Text(text));
            }
        }
        Some("file") if user => thread.user(id, Origin::Operator, file(piece)),
        Some("reasoning") => thread.thought(id, piece["text"].as_str().unwrap_or_default()),
        Some("tool") => tool(piece, thread),
        // Step marks, snapshots, patches and the like are not conversation.
        _ => {}
    }
}

/// An attached file: 2.x keeps its data inline, 1.x as a `data:` URL; a
/// file named by its path is linked.
fn file(file: &Value) -> Option<Part<'_>> {
    let mime = file["mime"].as_str()?;
    if !mime.starts_with("image/") {
        return None;
    }
    if let Some(data) = file["data"].as_str() {
        return Some(Part::Image { data, mime });
    }
    let url = file["url"].as_str()?;
    if let Some(data) = url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .map(|(_, data)| data)
    {
        return Some(Part::Image { data, mime });
    }
    url.strip_prefix("file://").map(|path| Part::ImagePath {
        path: path.to_owned(),
        mime: Some(mime.to_owned()),
    })
}

fn tool(piece: &Value, thread: &mut Thread) {
    let name = piece["name"]
        .as_str()
        .or(piece["tool"].as_str())
        .unwrap_or_default();
    let Some(id) = piece["id"].as_str().or(piece["callID"].as_str()) else {
        thread.unreadable("opencode_tool_shape");
        return;
    };
    let state = &piece["state"];
    let input = &state["input"];
    let path = input["path"].as_str().or(input["filePath"].as_str());
    let text = |key: &str| input[key].as_str().unwrap_or_default();
    let call = match name {
        // The to-do list is the plan, not a call (B5).
        "todowrite" => {
            thread.plan(todos(input));
            return;
        }
        "shell" | "bash" => Call::new(id, name, ToolKind::Execute).title(text("command")),
        "read" => {
            Call::new(id, name, ToolKind::Read).title(format!("Read {}", path.unwrap_or_default()))
        }
        "write" => {
            let mut call = Call::new(id, name, ToolKind::Edit)
                .title(format!("Write {}", path.unwrap_or_default()));
            if let Some(path) = path {
                call = call.diff(FileDiff {
                    path: path.to_owned(),
                    before: None,
                    after: text("content").to_owned(),
                });
            }
            call
        }
        "edit" => {
            let mut call = Call::new(id, name, ToolKind::Edit)
                .title(format!("Edit {}", path.unwrap_or_default()));
            if let Some(path) = path {
                call = call.diff(FileDiff {
                    path: path.to_owned(),
                    before: Some(text("oldString").to_owned()),
                    after: text("newString").to_owned(),
                });
            }
            call
        }
        "glob" | "grep" | "list" => Call::new(id, name, ToolKind::Search),
        "webfetch" | "websearch" => Call::new(id, name, ToolKind::Fetch),
        "task" => Call::new(id, name, ToolKind::Think).title(match text("description") {
            "" => name,
            description => description,
        }),
        _ => Call::new(id, name, ToolKind::Other),
    };
    let call = match path {
        Some(path) => call.path(path),
        None => call,
    };
    thread.call(call.input(input));

    let status = state["status"].as_str();
    let exit = state["metadata"]["exit"].as_i64();
    let failed = status == Some("error") || exit.is_some_and(|exit| exit != 0);
    let mut texts = state["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["text"].as_str())
        .chain(state["output"].as_str())
        .collect::<Vec<_>>();
    texts.extend(state["error"].as_str());
    // A file change shows its diff; its result text only confirms it.
    let output = (!matches!(name, "edit" | "write") || failed).then(|| texts.join("\n"));
    thread.result(
        id,
        Outcome {
            failed,
            running: matches!(status, Some("pending" | "running")),
            output,
            images: state["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["type"] == "file" || item["type"] == "image")
                .filter_map(file)
                .collect(),
            diffs: Vec::new(),
        },
    );
}

fn todos(input: &Value) -> Vec<Step> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|todo| Step {
            text: todo["content"].as_str().unwrap_or_default().to_owned(),
            status: match todo["status"].as_str() {
                Some("in_progress") => PlanEntryStatus::InProgress,
                Some("completed") => PlanEntryStatus::Completed,
                _ => PlanEntryStatus::Pending,
            },
        })
        .collect()
}
