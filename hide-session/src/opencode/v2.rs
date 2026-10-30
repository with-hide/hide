//! OpenCode 2.x's messages, read from `session_message`.
//!
//! A 2.x message is one row whose `data` holds everything the 1.x `message`
//! and `part` rows held apart: a person's `text` and attached `files`, and an
//! assistant's `content` items (text, reasoning, tool calls with their
//! output). The row's `type` names what it is, and only `user`, `synthetic`
//! and `assistant` messages carry a conversation; the others (agent, model and
//! location switches, `system`, `skill`, `shell`, `compaction`, `idle`) are
//! counted and passed without being read. A person's message can carry a
//! screenshot inline, so a row is never loaded whole: SQLite's JSON functions
//! scan it in place and hand back only the pieces Hide reads, each within the
//! row limit of the 1.x reader. A row over [`SCAN_LIMIT_BYTES`] is not
//! scanned, and one read scans at most that many bytes of rows.
//!
//! A message is written in order of its `seq`, which a message's index
//! counts. An assistant message OpenCode is still writing has no
//! `time.completed`, as in 1.x; the read stops before it and takes it whole on
//! a later read. A row too large to scan that is the session's newest and an
//! assistant's is waited for the same way, because it cannot be shown to be
//! finished; once another message follows it, it is skipped.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::{
    MESSAGE_LIMIT_BYTES, MESSAGES_PER_READ, PART_OVERHEAD_BYTES, PARTS_PER_MESSAGE, PARTS_PER_READ,
    READ_BUDGET_BYTES, ROW_LIMIT_BYTES, Walk, refusal, witness,
};
use crate::SkipReason;
use crate::turns::{ToolTurnMark, TurnMark};

/// The most bytes of one message row SQLite scans, and of rows in one read.
/// The first row of a read is always scanned, so a read moves on.
const SCAN_LIMIT_BYTES: u64 = 8 * 1024 * 1024;

/// The `content` item kinds that carry words or tool output; reasoning is
/// neither read nor counted.
const CONTENT_KINDS: &str = "json_extract(value, '$.type') IN ('text', 'tool')";

/// The first request of a person's, as the catalog shows it: the text of the
/// first `user` message that has any, read within the row limit.
pub(super) fn first_request(
    connection: &Connection,
    id: &str,
) -> Result<Option<(String, u64)>, String> {
    connection
        .query_row(
            "SELECT text, time_created FROM (SELECT CASE WHEN octet_length(data) <= ?2 \
             AND json_valid(data) THEN json_extract(data, '$.text') END AS text, \
             time_created, seq FROM session_message WHERE session_id = ?1 \
             AND type = 'user' AND typeof(data) = 'text') \
             WHERE typeof(text) = 'text' AND octet_length(text) <= ?3 \
             AND length(trim(text)) > 0 ORDER BY seq LIMIT 1",
            params![id, SCAN_LIMIT_BYTES as i64, ROW_LIMIT_BYTES],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map(|found| found.map(|(text, at)| (text, u64::try_from(at).unwrap_or(0))))
        .map_err(|error| refusal(&error))
}

/// Reads one page of messages from `start`, as [`super::walk_v1`] does for a
/// 1.x session.
pub(super) fn walk(
    connection: &Connection,
    session_id: &str,
    start: u64,
    created: u64,
    walk: Walk,
) -> Result<Walk, String> {
    let Walk {
        mut transcript,
        mut turns,
        mut previous,
        mut anchor,
        mut next,
        mut unfinished,
        mut over_budget,
        mut spent,
        mut visited,
        mut part_work,
    } = walk;
    let mut statement = connection
        .prepare(
            "SELECT rowid, CASE WHEN typeof(type) = 'text' THEN type END, \
             typeof(data) = 'text', octet_length(data), octet_length(id), \
             CASE WHEN octet_length(id) <= ?4 THEN id END, \
             EXISTS(SELECT 1 FROM session_message AS later \
             WHERE later.session_id = message.session_id AND later.seq > message.seq) \
             FROM session_message AS message WHERE session_id = ?1 \
             ORDER BY seq LIMIT ?2 OFFSET ?3",
        )
        .map_err(|error| refusal(&error))?;
    let mut rows = statement
        .query(params![
            session_id,
            MESSAGES_PER_READ,
            start as i64,
            MESSAGE_LIMIT_BYTES
        ])
        .map_err(|error| refusal(&error))?;
    // Every JSON function is guarded by `json_valid`, which would otherwise
    // fail the whole read on one message OpenCode wrote badly.
    let mut envelope = connection
        .prepare(
            "SELECT json_valid(data), \
             CASE WHEN json_valid(data) THEN CAST(json_extract(data, '$.time.created') AS INTEGER) END, \
             CASE WHEN json_valid(data) THEN json_extract(data, '$.time.completed') IS NOT NULL END, \
             CASE WHEN json_valid(data) AND typeof(json_extract(data, '$.text')) = 'text' \
             THEN octet_length(json_extract(data, '$.text')) END, \
             CASE WHEN json_valid(data) THEN (SELECT count(*) FROM json_each(data, '$.files') \
             WHERE json_extract(value, '$.mime') LIKE 'image/%') END \
             FROM session_message WHERE rowid = ?1",
        )
        .map_err(|error| refusal(&error))?;
    let mut text = connection
        .prepare("SELECT json_extract(data, '$.text') FROM session_message WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    let mut headers = connection
        .prepare(&format!(
            "SELECT key, octet_length(value) FROM json_each(\
             (SELECT data FROM session_message WHERE rowid = ?1), '$.content') \
             WHERE {CONTENT_KINDS} LIMIT ?2"
        ))
        .map_err(|error| refusal(&error))?;
    let mut item = connection
        .prepare("SELECT json_extract(data, ?2) FROM session_message WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    let mut scanned = 0_u64;
    while let Some(row) = rows.next().map_err(|error| refusal(&error))? {
        if part_work >= PARTS_PER_READ {
            over_budget = true;
            break;
        }
        visited += 1;
        let message_id = row
            .get::<_, Option<String>>(5)
            .map_err(|error| refusal(&error))?;
        let id_witness = message_id.as_deref().map(witness);
        let kind = row
            .get::<_, Option<String>>(1)
            .map_err(|error| refusal(&error))?;
        let kind = kind.as_deref();
        if !matches!(kind, Some("user" | "synthetic" | "assistant")) {
            previous = id_witness;
            next += 1;
            continue;
        }
        if !row.get::<_, bool>(2).map_err(|error| refusal(&error))? {
            transcript.skip("not_text");
            previous = id_witness;
            next += 1;
            continue;
        }
        let data_bytes: u64 = row.get(3).map_err(|error| refusal(&error))?;
        let id_bytes: u64 = row.get(4).map_err(|error| refusal(&error))?;
        let newest = !row.get::<_, bool>(6).map_err(|error| refusal(&error))?;
        let rowid: i64 = row.get(0).map_err(|error| refusal(&error))?;
        let message_id = match message_id {
            Some(id) if data_bytes <= SCAN_LIMIT_BYTES => id,
            waiting => {
                // Too large to learn whether OpenCode finished it.
                if kind == Some("assistant") && newest && waiting.is_some() {
                    unfinished = waiting;
                    break;
                }
                transcript.skip("message_capacity");
                previous = id_witness;
                next += 1;
                continue;
            }
        };
        let metadata_bytes = id_bytes.saturating_add(PART_OVERHEAD_BYTES);
        if metadata_bytes > READ_BUDGET_BYTES.saturating_sub(spent)
            || (scanned > 0 && data_bytes > SCAN_LIMIT_BYTES.saturating_sub(scanned))
        {
            over_budget = true;
            break;
        }
        spent += metadata_bytes;
        scanned += data_bytes;
        let (valid, at, completed, text_bytes, images): (
            bool,
            Option<i64>,
            Option<bool>,
            Option<u64>,
            Option<u32>,
        ) = envelope
            .query_row(params![rowid], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|error| refusal(&error))?;
        if !valid {
            transcript.skip("malformed_json");
            previous = Some(witness(&message_id));
            next += 1;
            continue;
        }
        if kind == Some("assistant") && completed != Some(true) {
            unfinished = Some(message_id);
            break;
        }
        let at = at.and_then(|at| u64::try_from(at).ok()).unwrap_or(0);
        let mut words = None;
        let mut parts = Vec::new();
        if kind == Some("assistant") {
            let part_limit = PARTS_PER_MESSAGE.min(PARTS_PER_READ - part_work);
            // One extra header proves the per-message cap was crossed.
            let found = headers
                .query_map(params![rowid, part_limit + 1], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?))
                })
                .map_err(|error| refusal(&error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| refusal(&error))?;
            let crossed = found.len() as i64 > part_limit;
            let found = &found[..found.len().min(part_limit as usize)];
            let size: u64 = found
                .iter()
                .map(|(_, bytes)| {
                    (if *bytes <= ROW_LIMIT_BYTES as u64 {
                        *bytes
                    } else {
                        0
                    }) + PART_OVERHEAD_BYTES
                })
                .sum();
            // The first message of a read is always taken, so a read moves on
            // however large its parts are; admission still precedes allocation.
            if next > start && size > READ_BUDGET_BYTES.saturating_sub(spent) {
                over_budget = true;
                break;
            }
            if crossed {
                transcript.skip("part_work_capacity");
            }
            for (key, bytes) in found {
                part_work += 1;
                if PART_OVERHEAD_BYTES > READ_BUDGET_BYTES.saturating_sub(spent) {
                    transcript.skip("read_budget");
                    continue;
                }
                spent += PART_OVERHEAD_BYTES;
                if *bytes > ROW_LIMIT_BYTES as u64 {
                    transcript.skip("part_capacity");
                    continue;
                }
                if *bytes > READ_BUDGET_BYTES.saturating_sub(spent) {
                    transcript.skip("read_budget");
                    continue;
                }
                spent += bytes;
                let data = item
                    .query_row(params![rowid, format!("$.content[{key}]")], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|error| refusal(&error))?;
                parts.push(data);
            }
        } else if let Some(bytes) = text_bytes {
            if bytes > ROW_LIMIT_BYTES as u64 {
                transcript.skip("part_capacity");
            } else {
                let size = bytes + PART_OVERHEAD_BYTES;
                if next > start && size > READ_BUDGET_BYTES.saturating_sub(spent) {
                    over_budget = true;
                    break;
                }
                spent += size;
                words = Some(
                    text.query_row(params![rowid], |row| row.get::<_, String>(0))
                        .map_err(|error| refusal(&error))?,
                );
            }
        }
        let (human, marks) = if kind == Some("assistant") {
            transcript.assistant_message(at, next, &parts)
        } else if kind == Some("user") {
            transcript.user_message(at, next, words, images.unwrap_or(0))
        } else {
            if let Some(body) = words {
                transcript.receipts(&body, next, at);
            }
            (false, Vec::new())
        };
        if human {
            anchor = Some(crate::ConversationCheckpoint::at_message(
                next, created, previous,
            ));
            turns.fold(next, &TurnMark::HumanTurn);
        }
        if !marks.is_empty() {
            turns.fold(next, &TurnMark::Tools(marks));
        }
        previous = Some(witness(&message_id));
        next += 1;
    }
    drop(rows);
    Ok(Walk {
        transcript,
        turns,
        previous,
        anchor,
        next,
        unfinished,
        over_budget,
        spent,
        visited,
        part_work,
    })
}

/// The question marks of the assistant message OpenCode is still writing, or
/// `None` when they do not fit in `budget`: the 2.x counterpart of
/// [`super::unfinished_questions`], with the same limits. Only `question` tool
/// items count, an item over the row limit is skipped as `part_capacity`
/// without being parsed, and a message too large to scan shows none.
pub(super) fn unfinished_questions(
    connection: &Connection,
    message_id: &str,
    budget: u64,
    transcript: &mut super::Read,
) -> Result<Option<Vec<ToolTurnMark>>, String> {
    let (rowid, scannable): (i64, bool) = connection
        .query_row(
            "SELECT rowid, octet_length(data) <= ?2 FROM session_message WHERE id = ?1",
            params![message_id, SCAN_LIMIT_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| refusal(&error))?;
    // A message too large to scan is waited for, not skipped, and shows no
    // question: its items cannot be read.
    if !scannable {
        return Ok(Some(Vec::new()));
    }
    let content = "(SELECT data FROM session_message WHERE rowid = ?1)";
    let oversized: i64 = connection
        .query_row(
            &format!(
                "SELECT count(*) FROM json_each({content}, '$.content') \
                 WHERE octet_length(value) > ?2 AND {CONTENT_KINDS}"
            ),
            params![rowid, ROW_LIMIT_BYTES],
            |row| row.get(0),
        )
        .map_err(|error| refusal(&error))?;
    for _ in 0..oversized {
        transcript.skip("part_capacity");
    }
    // CASE, unlike AND, fixes the order: no item over the limit is parsed.
    let mut headers = connection
        .prepare(&format!(
            "SELECT key, octet_length(value) FROM json_each({content}, '$.content') \
             WHERE CASE WHEN octet_length(value) <= ?3 \
             THEN json_extract(value, '$.type') = 'tool' \
             AND json_extract(value, '$.name') = 'question' ELSE 0 END LIMIT ?2"
        ))
        .map_err(|error| refusal(&error))?;
    let headers = headers
        .query_map(
            params![
                rowid,
                crate::turns::QUESTION_CALL_LIMIT as i64 + 1,
                ROW_LIMIT_BYTES
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, u64>(1)?)),
        )
        .map_err(|error| refusal(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| refusal(&error))?;
    if headers.len() > crate::turns::QUESTION_CALL_LIMIT {
        return Err(SkipReason::UserTurnCapacity.as_str().to_owned());
    }
    let bytes: u64 = headers
        .iter()
        .map(|(_, bytes)| bytes + PART_OVERHEAD_BYTES)
        .sum();
    if bytes > budget {
        return Ok(None);
    }
    let mut data = connection
        .prepare("SELECT json_extract(data, ?2) FROM session_message WHERE rowid = ?1")
        .map_err(|error| refusal(&error))?;
    let mut parts = Vec::with_capacity(headers.len());
    for (key, _) in headers {
        let part: String = data
            .query_row(params![rowid, format!("$.content[{key}]")], |row| {
                row.get(0)
            })
            .map_err(|error| refusal(&error))?;
        parts.push(
            serde_json::from_str::<Value>(&part)
                .map_err(|_| SkipReason::UserTurnInvalid.as_str().to_owned())?,
        );
    }
    crate::turns::native::opencode_2_marks(&parts)
        .map(Some)
        .map_err(|reason| reason.as_str().to_owned())
}

impl super::Read {
    /// A `user` message: the words and the images it carries.
    fn user_message(
        &mut self,
        at: u64,
        offset: u64,
        words: Option<String>,
        images: u32,
    ) -> (bool, Vec<ToolTurnMark>) {
        self.event(
            Some("user"),
            words.into_iter().collect(),
            images,
            at,
            offset,
            Vec::new(),
        )
    }

    /// A finished `assistant` message: the words of its text items, the
    /// pull requests its tools printed and the question marks they hold.
    fn assistant_message(
        &mut self,
        at: u64,
        offset: u64,
        items: &[String],
    ) -> (bool, Vec<ToolTurnMark>) {
        let mut text = Vec::new();
        let mut questions = Vec::new();
        for item in items {
            let Ok(item) = serde_json::from_str::<Value>(item) else {
                self.skip("malformed_json");
                continue;
            };
            match crate::turns::native::opencode_2_part(&item) {
                Ok(Some(mark)) => questions.push(mark),
                Ok(None) => {}
                Err(reason) => {
                    self.turn_error.get_or_insert(reason);
                }
            }
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(body) = item.get("text").and_then(Value::as_str) {
                        text.push(body.to_owned());
                    }
                }
                Some("tool") => self.tool_output(&item, at),
                _ => {}
            }
        }
        self.event(Some("assistant"), text, 0, at, offset, questions)
    }

    /// What a finished tool call printed: its text content.
    fn tool_output(&mut self, item: &Value, message_at: u64) {
        let called = item
            .pointer("/time/created")
            .and_then(Value::as_u64)
            .unwrap_or(message_at);
        let input = item.pointer("/state/input");
        self.note_call(input, called);
        let output = item
            .pointer("/state/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|content| content.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if output.is_empty() {
            return;
        }
        let at = item
            .pointer("/time/completed")
            .and_then(Value::as_u64)
            .unwrap_or(message_at);
        let made = super::made_by(
            item.get("name").and_then(Value::as_str),
            input,
            item,
            &output,
        );
        self.note_sightings(&output, at, &made);
    }
}
