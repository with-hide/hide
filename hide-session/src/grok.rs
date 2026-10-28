//! Grok 1.0.46 sessions: one directory per session,
//! `~/.grok/sessions/<url-encoded cwd>/<session id>/`.
//!
//! `updates.jsonl` is the append-only conversation authority (one
//! `{timestamp, method, params}` envelope per line); `summary.json` owns the
//! native id, the cwd and the current title, and is replaced whole on every
//! change; `plan_mode.json` says whether plan approval is awaited and
//! `plan.md` holds that plan. Semantics come from xai-org/grok-build
//! (`xai-grok-shell/src/session/`, `xai-grok-config/src/paths.rs`) and the
//! bundled user guide, `17-sessions.md` and `19-plan-mode.md`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::turns::{TurnMark, UserTurnContent};
use crate::{ConversationEvent, EventKind, LineResult, SkipReason};

/// The conversation file inside a session directory.
pub(crate) const UPDATES: &str = "updates.jsonl";
const SUMMARY: &str = "summary.json";
const PLAN_MODE: &str = "plan_mode.json";
const PLAN: &str = "plan.md";
/// Grok names a cwd's group by its URL encoding up to this many bytes and
/// by a slug and a blake3 prefix above it; only the first is proven here.
const GROUP_NAME_LIMIT: usize = 255;
const PLAN_MODE_LIMIT_BYTES: u64 = 4 * 1024;

pub(crate) struct Summary {
    pub id: String,
    pub cwd: PathBuf,
    /// The automatic title and the operator's `/rename`, with an empty
    /// string standing for none so a cleared title revokes an older one.
    pub title: (String, String),
    /// When this session itself was made: a fork's own time, while the
    /// records it copied keep their parent's.
    pub created_at_unix_ms: Option<u64>,
}

/// `urlencoding::encode` of the cwd, which Grok uses as the group name.
pub(crate) fn group_name(cwd: &Path) -> Result<String> {
    let spelling = cwd
        .to_str()
        .ok_or_else(|| anyhow!("label_session_cwd_unconfirmed"))?;
    let mut name = String::with_capacity(spelling.len());
    for byte in spelling.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            name.push(byte as char);
        } else {
            name.push_str(&format!("%{byte:02X}"));
        }
    }
    if name.len() > GROUP_NAME_LIMIT {
        return Err(anyhow!("session_route_unsupported_cwd"));
    }
    Ok(name)
}

/// The group directory of a session's conversation file.
pub(crate) fn group_of(path: &Path) -> Option<&Path> {
    if path.file_name()? != UPDATES {
        return None;
    }
    path.parent()?.parent()
}

/// A bounded, nonblocking read of a regular, unlinked sibling of the
/// conversation file. `Ok(None)` means the file is not there.
fn sibling(path: &Path, name: &str, limit: u64, read_bytes: &mut u64) -> Result<Option<Vec<u8>>> {
    let file_path = path
        .parent()
        .ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?
        .join(name);
    match std::fs::symlink_metadata(&file_path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(anyhow!("label_session_linked")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(anyhow!("label_session_metadata_read_failed")),
    }
    let file: File = match crate::open_session_file(&file_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(anyhow!("label_session_metadata_read_failed")),
    };
    if hide_platform::fs::identity::link_count(&file).ok() != Some(1) {
        return Err(anyhow!("label_session_linked"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("label_session_metadata_read_failed"))?;
    *read_bytes += bytes.len() as u64;
    Ok(Some(bytes))
}

/// Native identity, cwd and title from `summary.json`. A subagent child,
/// a headless run or a hidden session is no pane's root conversation.
pub(crate) fn summary(path: &Path, read_bytes: &mut u64) -> Result<Summary> {
    let limit = crate::SESSION_LINE_LIMIT_BYTES as u64;
    let bytes = sibling(path, SUMMARY, limit, read_bytes)?
        .ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?;
    if bytes.len() as u64 > limit {
        return Err(anyhow!("label_session_metadata_line_capacity"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("label_session_metadata_unconfirmed"))?;
    let id = value
        .pointer("/info/id")
        .and_then(Value::as_str)
        .filter(|id| crate::label_owner::valid_native_id(id))
        .ok_or_else(|| anyhow!("label_session_id_invalid"))?;
    let directory = path.parent().and_then(Path::file_name);
    if directory.is_none_or(|name| name != id) {
        return Err(anyhow!("label_session_id_mismatch"));
    }
    let cwd = value
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.chars().any(char::is_control))
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute())
        .ok_or_else(|| anyhow!("label_session_cwd_unconfirmed"))?;
    let kind = value.get("session_kind").and_then(Value::as_str);
    if kind.is_some_and(|kind| kind.starts_with("subagent") || kind == "headless")
        || value["hidden"] == true
    {
        return Err(anyhow!("label_session_not_root"));
    }
    let title = match value.get("generated_title") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(title)) => title.trim().to_owned(),
        Some(_) => return Err(anyhow!("label_session_metadata_unconfirmed")),
    };
    let title = match value.get("title_is_manual") {
        None | Some(Value::Bool(false)) => (title, String::new()),
        Some(Value::Bool(true)) => (String::new(), title),
        Some(_) => return Err(anyhow!("label_session_metadata_unconfirmed")),
    };
    Ok(Summary {
        id: id.to_owned(),
        cwd,
        title,
        created_at_unix_ms: crate::timestamp_ms(value.get("created_at")).ok(),
    })
}

/// The plan awaiting the operator's approval, if any: `plan_mode.json`
/// says `Active` and `awaiting_plan_approval`, the only native record of
/// the wait (an approval re-parked on resume writes no tool call). Its text
/// is `plan.md`, bounded by the user-turn content limit.
pub(crate) fn plan_hold(path: &Path, read_bytes: &mut u64) -> Result<Option<UserTurnContent>> {
    let Some(bytes) = sibling(path, PLAN_MODE, PLAN_MODE_LIMIT_BYTES, read_bytes)? else {
        return Ok(None);
    };
    if bytes.len() as u64 > PLAN_MODE_LIMIT_BYTES {
        return Err(anyhow!("label_session_metadata_line_capacity"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("label_session_metadata_unconfirmed"))?;
    let awaiting = match value.get("awaiting_plan_approval") {
        None => false,
        Some(Value::Bool(awaiting)) => *awaiting,
        Some(_) => return Err(anyhow!("label_session_metadata_unconfirmed")),
    };
    if !awaiting || value["state"] != "Active" {
        return Ok(None);
    }
    let limit = (crate::turns::content::TEXT_LIMIT_BYTES + 4) as u64;
    let text = match sibling(path, PLAN, limit, read_bytes)? {
        Some(mut bytes) => {
            bytes.truncate(limit as usize);
            // A cut inside the last character drops that character only.
            let valid = match std::str::from_utf8(&bytes) {
                Ok(_) => bytes.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => return Err(anyhow!("label_session_metadata_unconfirmed")),
            };
            bytes.truncate(valid);
            String::from_utf8(bytes).expect("checked UTF-8 prefix")
        }
        None => String::new(),
    };
    Ok(Some(UserTurnContent::new(text.trim(), [])))
}

fn update(item: &Value) -> Option<(&str, &Value)> {
    let update = item.pointer("/params/update")?;
    let kind = update.get("sessionUpdate")?.as_str()?;
    Some((kind, update))
}

fn is_extension(item: &Value) -> bool {
    item["method"] == "_x.ai/session/update"
}

/// The record's own time: the agent's millisecond stamp, which a fork
/// keeps, ahead of the envelope's whole seconds, which a fork rewrites.
fn at(item: &Value) -> std::result::Result<u64, SkipReason> {
    match item.pointer("/params/_meta/agentTimestampMs") {
        Some(value) => value
            .as_u64()
            .filter(|ms| *ms >= 10_000_000_000)
            .ok_or(SkipReason::InvalidTimestamp),
        None => crate::timestamp_ms(item.get("timestamp")),
    }
}

fn flag(update: &Value, name: &str) -> bool {
    update.get("_meta").and_then(|meta| meta.get(name)) == Some(&Value::Bool(true))
}

/// The native classifier of a tool call, which survives client renames.
fn tool_kind(update: &Value) -> Option<&str> {
    update.pointer("/_meta/x.ai~1tool/kind")?.as_str()
}

fn tool_text(update: &Value) -> Option<String> {
    let text = update
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|block| block.pointer("/content/text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn terminal(update: &Value) -> bool {
    matches!(update["status"].as_str(), Some("completed" | "failed"))
}

pub(crate) fn parse_line(item: &Value) -> LineResult {
    let Some((kind, update)) = update(item) else {
        return LineResult::Ignore;
    };
    if is_extension(item) {
        // A cancelled or crashed turn is the conversation's interruption.
        return match (kind, update["stop_reason"].as_str()) {
            ("turn_completed", Some("cancelled" | "interrupted")) => match at(item) {
                Ok(at) => LineResult::Event(ConversationEvent::new(
                    "assistant",
                    EventKind::Interrupted,
                    at,
                    "",
                )),
                Err(reason) => LineResult::Skip(reason),
            },
            _ => LineResult::Ignore,
        };
    }
    match kind {
        "user_message_chunk" => user_chunk(item, update),
        "agent_message_chunk" => {
            let Some(text) = update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            else {
                return LineResult::Ignore;
            };
            let at = match at(item) {
                Ok(at) => at,
                Err(reason) => return LineResult::Skip(reason),
            };
            let turn = item
                .pointer("/params/_meta/promptId")
                .and_then(Value::as_str)
                .map(|turn| format!("a:{turn}"));
            LineResult::Event(
                ConversationEvent::new("assistant", EventKind::Assistant, at, text).with_part(turn),
            )
        }
        "tool_call_update" if terminal(update) => {
            let Some(text) = tool_text(update) else {
                return LineResult::Ignore;
            };
            match at(item) {
                Ok(at) => LineResult::Sightings(crate::sightings_in(&[&text], at)),
                Err(_) => LineResult::Ignore,
            }
        }
        // Thoughts, tool calls, the TODO plan, mode and command catalogs
        // are not conversation messages.
        _ => LineResult::Ignore,
    }
}

/// One record per content block of a prompt; the blocks of one prompt
/// share its `promptIndex` and become one message.
fn user_chunk(item: &Value, update: &Value) -> LineResult {
    let meta = update.get("_meta");
    let flag = |name: &str| meta.and_then(|meta| meta.get(name)) == Some(&Value::Bool(true));
    if flag("hostTurn") {
        return LineResult::Ignore;
    }
    let content = &update["content"];
    let (text, images) = match content["type"].as_str() {
        Some("text") => {
            let typed = flag("interjection")
                .then(|| meta.and_then(|meta| meta.get("displayText")))
                .flatten();
            match typed
                .or_else(|| content.get("text"))
                .and_then(Value::as_str)
            {
                Some(text) => (text.to_owned(), 0),
                None => return LineResult::Ignore,
            }
        }
        Some("image") => (String::new(), 1),
        _ => return LineResult::Ignore,
    };
    if text.trim().is_empty() && images == 0 {
        return LineResult::Ignore;
    }
    let at = match at(item) {
        Ok(at) => at,
        Err(reason) => return LineResult::Skip(reason),
    };
    let injected = flag("hideFromScrollback");
    let kind = if injected || crate::has_injected_prefix(&text) {
        EventKind::Injected
    } else {
        EventKind::Human
    };
    let prompt = meta
        .and_then(|meta| meta.get("promptIndex"))
        .and_then(Value::as_u64)
        .map(|index| format!("u:{index}"));
    LineResult::Event(
        ConversationEvent::new("user", kind, at, text)
            .with_images(images)
            .with_provider_injected(injected)
            .with_part(prompt),
    )
}

/// Grok's waits: an unanswered `ask_user_question` holds a question, its
/// terminal tool update answers it, and `turn_completed` ends the turn.
/// Plan approval is read from `plan_mode.json` instead ([`plan_hold`]).
pub(crate) fn turn(item: &Value) -> std::result::Result<Option<TurnMark>, SkipReason> {
    use crate::turns::ToolTurnMark;
    let Some((kind, update)) = update(item) else {
        return Ok(None);
    };
    let call = || crate::turns::native::id(update.get("toolCallId"));
    if is_extension(item) {
        if kind != "turn_completed" {
            return Ok(None);
        }
        let turn = crate::turns::native::id(update.get("prompt_id"))?;
        return Ok(Some(match update["stop_reason"].as_str() {
            Some("cancelled" | "interrupted" | "error" | "rate_limit") => {
                TurnMark::Aborted { turn }
            }
            _ => TurnMark::Completed { turn },
        }));
    }
    match kind {
        // A turn the operator did not type (a task completion wake) still
        // starts a turn, as does a typed block too long to keep; any other
        // typed one starts it through its message event.
        "user_message_chunk"
            if flag(update, "hideFromScrollback")
                || (flag(update, OMITTED_TEXT) && !flag(update, "hostTurn")) =>
        {
            Ok(Some(TurnMark::HumanTurn))
        }
        "tool_call" | "tool_call_update" if terminal(update) => {
            Ok(call()?.map(|call| TurnMark::Tools(vec![ToolTurnMark::Answered { call }])))
        }
        "tool_call" | "tool_call_update"
            if tool_kind(update) == Some("ask_user") || update["title"] == "ask_user_question" =>
        {
            let call = call()?.ok_or(SkipReason::UserTurnInvalid)?;
            let content = update
                .get("rawInput")
                .and_then(crate::turns::native::question_content);
            Ok(Some(TurnMark::Tools(vec![ToolTurnMark::Asked {
                call,
                content,
            }])))
        }
        _ => Ok(None),
    }
}

/// The fields of a record kept when its line is longer than the line cap:
/// which update it is, its call and turn ids, its prompt index and flags,
/// its times and its block type. Grok writes a pasted image inline and a
/// tool's output twice, so such lines are ordinary; reading them without
/// their bodies keeps every turn mark; a question call whose body is over
/// the cap still waits, as a question without content.
const KEPT: &[&str] = &[
    "/timestamp",
    "/method",
    "/params/sessionId",
    "/params/_meta/agentTimestampMs",
    "/params/_meta/promptId",
    "/params/update/sessionUpdate",
    "/params/update/toolCallId",
    "/params/update/status",
    "/params/update/title",
    "/params/update/prompt_id",
    "/params/update/stop_reason",
    "/params/update/content/type",
    "/params/update/_meta/promptIndex",
    "/params/update/_meta/hideFromScrollback",
    "/params/update/_meta/hostTurn",
    "/params/update/_meta/interjection",
    "/params/update/_meta/x.ai~1tool/kind",
];
/// Marks a typed prompt block whose text exceeded the line cap: the turn it
/// starts is kept, its text is not invented.
const OMITTED_TEXT: &str = "hide.omittedText";
const LARGE_KEY_BYTES: usize = 64;
const LARGE_VALUE_BYTES: usize = 256;
const LARGE_DEPTH: usize = 64;

/// A bounded streaming scan of one oversized `updates.jsonl` line that
/// retains only the [`KEPT`] scalars, so a checkpoint never carries a body.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct LargeLine {
    frames: Vec<LargeFrame>,
    string: Option<LargeScalar>,
    literal: Option<LargeScalar>,
    /// At most one value per [`KEPT`] pointer, the last one written.
    kept: Vec<(String, String)>,
    invalid: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct LargeFrame {
    object: bool,
    /// The current member's JSON-pointer token; `None` for an array, a
    /// key too long to be kept or one spelled with an escape.
    key: Option<String>,
    expecting_key: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct LargeScalar {
    key: bool,
    /// The kept pointer this value fills, when it fills one.
    pointer: Option<String>,
    raw: String,
    escaped: bool,
    plain: bool,
}

impl LargeLine {
    fn pointer(&self) -> Option<String> {
        let mut pointer = String::new();
        for frame in &self.frames {
            pointer.push('/');
            pointer.push_str(frame.key.as_deref()?);
        }
        KEPT.contains(&pointer.as_str()).then_some(pointer)
    }

    fn at_key(&self) -> bool {
        self.frames
            .last()
            .is_some_and(|frame| frame.object && frame.expecting_key)
    }

    fn keep(&mut self, pointer: String, raw: String) {
        self.kept.retain(|(kept, _)| *kept != pointer);
        self.kept.push((pointer, raw));
    }

    fn finish_literal(&mut self) {
        if let Some(literal) = self.literal.take()
            && let Some(pointer) = literal.pointer.filter(|_| literal.plain)
        {
            self.keep(pointer, literal.raw);
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> std::result::Result<(), crate::SessionError> {
        for &byte in bytes {
            if let Some(string) = self.string.as_mut() {
                if string.escaped {
                    string.escaped = false;
                } else if byte == b'"' {
                    let string = self.string.take().unwrap();
                    if string.key {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.key = string
                                .plain
                                .then(|| string.raw.replace('~', "~0").replace('/', "~1"));
                            frame.expecting_key = false;
                        }
                    } else if let Some(pointer) = string.pointer.filter(|_| string.plain) {
                        self.keep(pointer, format!("\"{}\"", string.raw));
                    }
                    continue;
                } else if byte == b'\\' {
                    string.escaped = true;
                    // A key spelled with an escape is never one of ours; an
                    // escaped value keeps its JSON spelling.
                    string.plain &= !string.key;
                }
                let limit = if string.key {
                    LARGE_KEY_BYTES
                } else {
                    LARGE_VALUE_BYTES
                };
                if string.plain && (string.key || string.pointer.is_some()) {
                    // Only ASCII spellings are ever kept: every kept field
                    // and every key on a kept path is ASCII.
                    if string.raw.len() < limit && byte.is_ascii() {
                        string.raw.push(byte as char);
                    } else {
                        string.plain = false;
                    }
                }
                continue;
            }
            match byte {
                b'"' => {
                    self.finish_literal();
                    let key = self.at_key();
                    let pointer = if key { None } else { self.pointer() };
                    self.string = Some(LargeScalar {
                        key,
                        pointer,
                        raw: String::new(),
                        escaped: false,
                        plain: true,
                    });
                }
                b'{' | b'[' => {
                    self.finish_literal();
                    if self.frames.len() >= LARGE_DEPTH {
                        return Err(crate::SessionError::Capacity {
                            resource: "json_depth",
                            limit: LARGE_DEPTH as u64,
                        });
                    }
                    let object = byte == b'{';
                    self.frames.push(LargeFrame {
                        object,
                        key: None,
                        expecting_key: object,
                    });
                }
                b'}' | b']' => {
                    self.finish_literal();
                    match self.frames.pop() {
                        Some(frame) => self.invalid |= frame.object != (byte == b'}'),
                        None => self.invalid = true,
                    }
                }
                b',' => {
                    self.finish_literal();
                    if let Some(frame) = self.frames.last_mut() {
                        frame.expecting_key = frame.object;
                        frame.key = None;
                    }
                }
                b':' | b' ' | b'\t' | b'\r' | b'\n' => self.finish_literal(),
                _ => {
                    if self.literal.is_none() {
                        let pointer = self.pointer();
                        self.literal = Some(LargeScalar {
                            key: false,
                            pointer,
                            raw: String::new(),
                            escaped: false,
                            plain: true,
                        });
                    }
                    let literal = self.literal.as_mut().unwrap();
                    if literal.raw.len() < 32 && byte.is_ascii() {
                        literal.raw.push(byte as char);
                    } else {
                        literal.plain = false;
                    }
                }
            }
        }
        Ok(())
    }

    /// The record without its bodies, or `None` when it cannot be read
    /// that way: an unfinished or malformed line or an unknown update.
    pub(crate) fn reduced(mut self) -> Option<String> {
        self.finish_literal();
        if self.invalid || !self.frames.is_empty() || self.string.is_some() {
            return None;
        }
        let mut record = Value::Object(Default::default());
        for (pointer, raw) in &self.kept {
            let value = serde_json::from_str::<Value>(raw).ok()?;
            let mut at = &mut record;
            for token in pointer.split('/').skip(1) {
                let token = token.replace("~1", "/").replace("~0", "~");
                at = at
                    .as_object_mut()?
                    .entry(token)
                    .or_insert_with(|| Value::Object(Default::default()));
            }
            *at = value;
        }
        let (kind, update) = update(&record)?;
        if kind == "user_message_chunk" && update.pointer("/content/type") == Some(&"text".into()) {
            record
                .pointer_mut("/params/update")?
                .as_object_mut()?
                .entry("_meta")
                .or_insert_with(|| Value::Object(Default::default()))
                .as_object_mut()?
                .insert(OMITTED_TEXT.into(), Value::Bool(true));
        }
        serde_json::to_string(&record).ok()
    }
}

/// [`LargeLine`] over a line already in memory.
pub(crate) fn reduced_line(line: &str) -> Option<String> {
    let mut scan = LargeLine::default();
    scan.feed(line.as_bytes()).ok()?;
    scan.reduced()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanned(line: &str) -> Option<String> {
        let mut scan = LargeLine::default();
        scan.feed(line.as_bytes()).ok()?;
        scan.reduced()
    }

    /// The reduced form of every fixture record starts, asks, answers and
    /// ends the same turns and keeps its message kinds; it loses bodies,
    /// so a typed block starts its turn by a mark instead of its message.
    #[test]
    fn a_reduced_record_has_the_turn_effects_and_kinds_of_the_whole_one() {
        let fixture = include_str!("../tests/fixtures/adapters/grok-1.0.46/updates.jsonl");
        let effect = |item: &Value| {
            let event = match parse_line(item) {
                LineResult::Event(event) => {
                    format!("{:?}{:?}{}", event.kind, event.part(), event.images)
                }
                LineResult::Skip(reason) => reason.as_str().into(),
                // A dropped tool output leaves no sightings.
                _ => String::new(),
            };
            let mark = format!("{:?}", turn(item));
            match (event.as_str(), mark.as_str()) {
                // A prompt starts its turn by its typed text or by a mark;
                // only the mark survives a dropped text.
                (typed, "Ok(None)") if typed.starts_with("Human") && typed.ends_with("0") => {
                    "turn".to_owned()
                }
                (_, "Ok(Some(HumanTurn))") => "turn".to_owned(),
                // A dropped answer leaves no message.
                (answer, mark) if answer.is_empty() || answer.starts_with("Assistant") => {
                    mark.to_owned()
                }
                (event, mark) => format!("{event} {mark}"),
            }
        };
        for line in fixture.lines() {
            let whole: Value = serde_json::from_str(line).unwrap();
            let reduced: Value = serde_json::from_str(&scanned(line).unwrap()).unwrap();
            assert_eq!(effect(&whole), effect(&reduced), "{line}");
        }
    }

    #[test]
    fn a_scan_split_at_every_byte_and_saved_between_reads_the_same_fields() {
        let line = serde_json::json!({"timestamp":1,"method":"session/update","params":{
            "sessionId":"s","update":{"sessionUpdate":"tool_call_update","toolCallId":"c\"1",
            "status":"completed","content":[{"type":"content","content":{"type":"text",
            "text":"{\"status\":\"failed\",\"sessionUpdate\":\"x\"}"}}],
            "rawOutput":{"status":"nested","title":"ask_user_question"}},
            "_meta":{"agentTimestampMs":17_909_894_000_000_u64}}})
        .to_string();
        let expected = scanned(&line).unwrap();
        let value: Value = serde_json::from_str(&expected).unwrap();
        assert_eq!(value["params"]["update"]["toolCallId"], "c\"1");
        assert_eq!(value["params"]["update"]["status"], "completed");
        assert!(
            value["params"]["update"].get("title").is_none(),
            "only kept paths"
        );
        for cut in 0..line.len() {
            let mut scan = LargeLine::default();
            scan.feed(&line.as_bytes()[..cut]).unwrap();
            let saved = serde_json::to_string(&scan).unwrap();
            let mut scan: LargeLine = serde_json::from_str(&saved).unwrap();
            scan.feed(&line.as_bytes()[cut..]).unwrap();
            assert_eq!(
                scan.reduced().as_deref(),
                Some(expected.as_str()),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn a_repeated_kept_key_holds_one_value_and_a_broken_line_reads_as_nothing() {
        let repeated = format!(
            "{{\"params\":{{\"update\":{{\"sessionUpdate\":\"plan\",{}\"status\":\"last\"}}}}}}",
            "\"status\":\"x\",".repeat(10_000)
        );
        let mut scan = LargeLine::default();
        scan.feed(repeated.as_bytes()).unwrap();
        assert_eq!(scan.kept.len(), 2);
        let value: Value = serde_json::from_str(&scan.reduced().unwrap()).unwrap();
        assert_eq!(value["params"]["update"]["status"], "last");
        assert_eq!(scanned("not json"), None);
        assert_eq!(
            scanned("{\"params\":{\"update\":{}}}"),
            None,
            "no update kind"
        );
        assert_eq!(
            scanned("{\"params\":{\"update\":{\"sessionUpdate\":\"plan\""),
            None
        );
    }

    #[test]
    fn a_group_is_the_url_encoded_cwd_and_a_hashed_name_is_never_guessed() {
        // urlencoding 2.1.3: every byte but `A-Za-z0-9-_.~`, upper-case hex.
        assert_eq!(
            group_name(Path::new("/Users/me/my proj~1/요")).unwrap(),
            "%2FUsers%2Fme%2Fmy%20proj~1%2F%EC%9A%94"
        );
        let longest = format!("/{}", "a".repeat(252));
        assert_eq!(group_name(Path::new(&longest)).unwrap().len(), 255);
        let hashed = format!("/{}", "a".repeat(253));
        assert_eq!(
            group_name(Path::new(&hashed)).unwrap_err().to_string(),
            "session_route_unsupported_cwd"
        );
    }
}
