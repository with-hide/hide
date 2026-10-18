//! Read completed assistant step-finish parts, never message.tokens: OpenCode
//! replaces those tokens on each step while accumulating message.cost.
//! OpenCode 2.x has no step-finish parts: it writes one assistant message per
//! step and sets that message's tokens and cost once, when the step ends, so
//! a completed assistant message is one usage event.
//! A database 2.x migrated from 1.x holds both: a message the 1.x tables hold
//! is counted from its parts, as before the upgrade, so a ledger keeps its
//! keys, and only a message `session_message` alone holds is counted the 2.x
//! way.
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::records::{bounded, required};
use super::{Checkpoint, Event, PARSER_VERSION, Refusal, Step, Tokens, key};
use crate::opencode::{Legacy, Schema};

const ROW_BYTES: u64 = 64 * 1024;
const READ_BYTES: u64 = crate::SESSION_INCREMENT_READ_LIMIT_BYTES;
const ROWS: usize = 512;
/// An OpenCode 2.x assistant message row holds its content as well, so one is
/// read in place by SQLite's JSON functions, and only the step's few fields
/// come back. The row is never loaded into Hide, so this bounds the scan, not
/// memory: a row over it is reported as unread (`record_capacity`), and one
/// read scans at most this many bytes of rows, the next read going on from the
/// cursor. The first row of a read is always scanned, so a read moves on.
const ROW_2_BYTES: u64 = 8 * 1024 * 1024;
/// The step's fields as SQLite hands them back from a row read in place. Each
/// text is bounded where it is taken, so a field as large as the row cannot
/// come back whole.
const FIELDS_2: &str = "json_object(
              'completed',json_extract(data,'$.time.completed'),
              'model',CASE WHEN octet_length(json_extract(data,'$.model.id'))<=4096
                THEN json_extract(data,'$.model.id') END,
              'cost',json_extract(data,'$.cost'),
              'tokens',json(CASE WHEN octet_length(json_extract(data,'$.tokens'))<=4096
                THEN json_extract(data,'$.tokens') END),
              'failed',json_extract(data,'$.error') IS NOT NULL,
              'cwd',(SELECT CASE WHEN length(directory)>0 AND octet_length(directory)<=4096
                THEN directory END FROM session_v2 WHERE id=m.session_id))";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cursor {
    after_time: u64,
    after_id: String,
    horizon: Option<u64>,
    active: Option<Active>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Active {
    id: String,
    updated: u64,
    session: String,
    model: String,
    cwd: Option<String>,
    completed: u64,
    single_step_cost: Option<u64>,
    part_after: String,
}

#[derive(Deserialize)]
struct Message {
    role: String,
    #[serde(rename = "modelID")]
    model: Option<String>,
    time: Time,
    path: Option<MessagePath>,
    cost: Option<f64>,
}
#[derive(Deserialize)]
struct Time {
    completed: Option<u64>,
}
#[derive(Deserialize)]
struct MessagePath {
    cwd: Option<String>,
}
#[derive(Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: String,
    tokens: Option<PartTokens>,
    cost: Option<f64>,
}
#[derive(Deserialize)]
struct PartTokens {
    input: u64,
    output: u64,
    #[serde(default)]
    reasoning: u64,
    cache: Cache,
}
#[derive(Deserialize)]
struct Cache {
    read: u64,
    write: u64,
}

fn cost(value: Option<f64>) -> Result<Option<u64>, &'static str> {
    match value {
        Some(n) if !n.is_finite() || n < 0. || n * 1_000_000. >= u64::MAX as f64 => {
            Err("cost_invalid")
        }
        Some(n) if n > 0. => Ok(Some((n * 1_000_000.).round() as u64)),
        _ => Ok(None),
    }
}

/// A step's tokens, with output including reasoning.
fn usage_tokens(t: &PartTokens) -> Result<Tokens, &'static str> {
    let output = t
        .output
        .checked_add(t.reasoning)
        .ok_or("tokens_inconsistent")?;
    t.input
        .checked_add(output)
        .and_then(|n| n.checked_add(t.cache.read))
        .and_then(|n| n.checked_add(t.cache.write))
        .ok_or("tokens_inconsistent")?;
    Ok(Tokens {
        input: t.input,
        output,
        cache_read: t.cache.read,
        cache_write: t.cache.write,
        cache_write_1h: 0,
        reasoning: Some(t.reasoning),
    })
}

fn event(id: String, bytes: &str, active: &Active) -> Result<Option<Event>, &'static str> {
    let part: Part = serde_json::from_str(bytes).map_err(|_| "record_invalid")?;
    if part.kind != "step-finish" {
        return Ok(None);
    }
    let tokens = usage_tokens(&part.tokens.ok_or("usage_missing")?)?;
    let recorded = match part.cost {
        Some(_) => cost(part.cost)?,
        None => active.single_step_cost,
    };
    Ok(Some(Event {
        key: key(&[&required(Some(id), 128)?]),
        at_ms: active.completed,
        model: active.model.clone(),
        tokens,
        recorded_cost_micro_usd: recorded,
        session: active.session.clone(),
        cwd: active.cwd.clone(),
        provider: None,
        incomplete: false,
    }))
}

fn active(
    db: &Connection,
    id: &str,
    updated: u64,
    session: String,
    data: &str,
) -> Result<Option<Active>, &'static str> {
    let message: Message = serde_json::from_str(data).map_err(|_| "record_invalid")?;
    if message.role != "assistant" {
        return Ok(None);
    }
    let Some(completed) = message.time.completed else {
        return Ok(None);
    };
    let count: u64 = db
        .query_row(
            "SELECT count(*) FROM (SELECT 1 FROM part WHERE message_id=?1
         AND octet_length(data)<=?2 AND json_valid(data)
         AND json_extract(data,'$.type')='step-finish' LIMIT 2)",
            params![id, ROW_BYTES],
            |row| row.get(0),
        )
        .map_err(|_| "database_budget")?;
    Ok(Some(Active {
        id: required(Some(id.to_owned()), 128)?,
        updated,
        session: required(Some(session), 128)?,
        model: required(message.model, 128)?,
        cwd: bounded(message.path.and_then(|p| p.cwd), 4096)?,
        completed,
        single_step_cost: if count == 1 {
            cost(message.cost)?
        } else {
            None
        },
        part_after: String::new(),
    }))
}

pub(super) fn read(home: &Path, saved: Option<Checkpoint>) -> Result<Step, Refusal> {
    crate::opencode::with_read_legacy(home, |db, schema, legacy| {
        read_connection(home, db, schema, legacy, saved)
    })
    .map_err(|reason| {
        if reason == crate::opencode::SCHEMA_UNKNOWN {
            Refusal::Schema
        } else if reason == crate::environment::OPENCODE_DATA_HOME_INVALID {
            Refusal::Environment
        } else {
            Refusal::Database
        }
    })?
}

fn read_connection(
    home: &Path,
    db: &Connection,
    schema: Schema,
    legacy: Legacy,
    saved: Option<Checkpoint>,
) -> Result<Step, Refusal> {
    // A message 2.x migrated from 1.x is counted from its 1.x parts, so the
    // ledger keeps the keys it took before the upgrade. Tables that are not
    // the whole of what 2.x leaves cannot say which messages those are.
    if legacy == Legacy::Partial {
        return Err(Refusal::Schema);
    }
    let migrated = schema == Schema::V2 && legacy == Legacy::Complete;
    let started = Instant::now();
    // A 2.x row is scanned in place up to its own cap, which SQLite refuses to
    // load past this limit; a 1.x value is never over the read budget.
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        (if schema == Schema::V2 {
            ROW_2_BYTES.max(READ_BYTES)
        } else {
            READ_BYTES
        }) as i32,
    );
    let database = crate::opencode::database_path(home).map_err(|_| Refusal::Environment)?;
    let id = hide_platform::fs::identity::file_id(&database).map_err(|_| Refusal::Unreadable)?;
    // VACUUM changes schema_version even when it retains the same file id.
    let version: u64 = db
        .query_row("PRAGMA schema_version", [], |row| row.get(0))
        .map_err(|_| Refusal::Database)?;
    let witness = format!("{}:{}:{version}", id.volume(), id.index());
    let mut checkpoint = saved.unwrap_or_default();
    let rescanned = checkpoint.identity.is_some()
        && (checkpoint.identity.as_ref() != Some(&witness)
            || checkpoint.parser_version != PARSER_VERSION);
    if rescanned {
        checkpoint = Checkpoint::default();
    }
    checkpoint.identity = Some(witness);
    checkpoint.parser_version = PARSER_VERSION;
    if checkpoint.database.horizon.is_none() {
        let horizon: Option<u64> = db
            .query_row(
                &if migrated {
                    "SELECT max(newest) FROM (SELECT max(time_updated) AS newest FROM message \
                     UNION ALL SELECT max(time_updated) FROM session_message)"
                        .to_owned()
                } else {
                    format!("SELECT max(time_updated) FROM {}", schema.messages())
                },
                [],
                |r| r.get(0),
            )
            .map_err(|_| Refusal::Database)?;
        checkpoint.database.horizon = horizon;
    }
    let mut step = Step {
        events: Vec::new(),
        checkpoint,
        read_bytes: 0,
        has_more: false,
        rescanned,
        skipped: BTreeMap::new(),
    };
    let Some(horizon) = step.checkpoint.database.horizon else {
        return Ok(step);
    };
    // Bytes of 2.x rows SQLite has scanned in place in this read.
    let mut scanned = 0_u64;
    for _ in 0..ROWS {
        if started.elapsed() >= super::STEP_TIME || step.read_bytes + ROW_BYTES + 256 > READ_BYTES {
            step.has_more = true;
            break;
        }
        if let Some(active) = &mut step.checkpoint.database.active {
            let row: Option<(String, u64, Option<String>)> = db
                .query_row(
                    "SELECT CASE WHEN octet_length(id)<=128 THEN id END,octet_length(data),
                 CASE WHEN octet_length(data)<=?3 THEN data END
                 FROM part WHERE message_id=?1 AND id>?2 ORDER BY id LIMIT 1",
                    params![active.id, active.part_after, ROW_BYTES],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|_| Refusal::Database)?;
            if let Some((id, length, data)) = row {
                if id.len() > 128 {
                    return Err(Refusal::Database);
                }
                active.part_after = id.clone();
                step.read_bytes += id.len() as u64 + data.as_ref().map_or(0, |s| s.len() as u64);
                let result = data
                    .as_deref()
                    .ok_or(if length > ROW_BYTES {
                        "record_capacity"
                    } else {
                        "record_invalid"
                    })
                    .and_then(|data| event(id, data, active));
                match result {
                    Ok(Some(event)) => step.events.push(event),
                    Ok(None) => (),
                    Err(reason) => *step.skipped.entry(reason.into()).or_default() += 1,
                }
                step.has_more = true;
                continue;
            }
            step.checkpoint.database.after_time = active.updated;
            step.checkpoint.database.after_id = active.id.clone();
            step.checkpoint.database.active = None;
        }
        if migrated {
            match next_migrated(db, &mut step, horizon, &mut scanned)? {
                Next::Row => continue,
                Next::End => break,
                Next::Full => {
                    step.has_more = true;
                    break;
                }
            }
        }
        if schema == Schema::V2 {
            match next_2(db, &mut step, horizon, &mut scanned)? {
                Next::Row => continue,
                Next::End => break,
                Next::Full => {
                    step.has_more = true;
                    break;
                }
            }
        }
        let cursor = &step.checkpoint.database;
        let row: Option<(String, u64, String, Option<String>)> = db
            .query_row(
                "SELECT CASE WHEN octet_length(id)<=128 THEN id END,time_updated,
             CASE WHEN octet_length(session_id)<=128 THEN session_id END,
             CASE WHEN octet_length(data)<=?4 THEN data END
             FROM message WHERE (time_updated>?1 OR (time_updated=?1 AND id>?2))
             AND time_updated<=?3 ORDER BY time_updated,id LIMIT 1",
                params![cursor.after_time, cursor.after_id, horizon, ROW_BYTES],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|_| Refusal::Database)?;
        let Some((id, updated, session, data)) = row else {
            // Revisit the boundary timestamp next round: later commits may
            // insert another id with the same millisecond, in either order.
            step.checkpoint.database.after_time = horizon;
            step.checkpoint.database.after_id.clear();
            step.checkpoint.database.horizon = None;
            step.has_more = false;
            break;
        };
        if id.len() > 128 || session.len() > 128 {
            return Err(Refusal::Database);
        }
        step.read_bytes +=
            (id.len() + session.len()) as u64 + data.as_ref().map_or(0, |s| s.len() as u64);
        step.checkpoint.database.after_time = updated;
        step.checkpoint.database.after_id = id.clone();
        take_legacy(db, &mut step, id, updated, session, data.as_deref())?;
        step.has_more = true;
    }
    Ok(step)
}

/// Opens a 1.x message for its step-finish parts, once the cursor is on it.
fn take_legacy(
    db: &Connection,
    step: &mut Step,
    id: String,
    updated: u64,
    session: String,
    data: Option<&str>,
) -> Result<(), Refusal> {
    match data
        .ok_or("record_capacity")
        .and_then(|data| active(db, &id, updated, session, data))
    {
        Ok(active) => step.checkpoint.database.active = active,
        Err("database_budget") => return Err(Refusal::Database),
        Err(reason) => *step.skipped.entry(reason.into()).or_default() += 1,
    }
    Ok(())
}

/// What an OpenCode 2.x assistant message says of its step, as SQLite hands
/// it back from the row it read in place.
#[derive(Deserialize)]
struct Step2 {
    completed: Option<u64>,
    model: Option<String>,
    cost: Option<f64>,
    tokens: Option<PartTokens>,
    /// 1 when the step failed, which may end without usage.
    failed: u8,
    /// The session's checkout.
    cwd: Option<String>,
}

fn event_2(id: String, session: String, step: &str) -> Result<Option<Event>, &'static str> {
    let step: Step2 = serde_json::from_str(step).map_err(|_| "record_invalid")?;
    // A step OpenCode is still writing has no completion yet; it is read
    // again when its row is updated.
    let Some(completed) = step.completed else {
        return Ok(None);
    };
    let Some(tokens) = step.tokens else {
        return if step.failed == 1 {
            Ok(None)
        } else {
            Err("usage_missing")
        };
    };
    Ok(Some(Event {
        key: key(&[&required(Some(id), 128)?]),
        at_ms: completed,
        model: required(step.model, 128)?,
        tokens: usage_tokens(&tokens)?,
        recorded_cost_micro_usd: cost(step.cost)?,
        session: required(Some(session), 128)?,
        cwd: bounded(step.cwd, 4096)?,
        provider: None,
        incomplete: false,
    }))
}

/// What taking the next 2.x row came to.
enum Next {
    /// A row was taken, or reported unread; the cursor is past it.
    Row,
    /// None is left after the cursor: the round is over.
    End,
    /// The next row would take the read past [`ROW_2_BYTES`] of scanned rows
    /// and waits, the cursor still before it, for the next read.
    Full,
}

/// Takes one 2.x assistant row's step as a usage event, or reports it unread.
/// `length` is the row's size from its record header and `fields` what SQLite
/// read of it in place, absent when it was not scanned.
fn take_2(
    step: &mut Step,
    id: String,
    session: String,
    length: Option<u64>,
    fields: Option<String>,
    scanned: &mut u64,
) -> Next {
    let result = match (&fields, length) {
        (Some(fields), _) => {
            *scanned += length.unwrap_or(0);
            event_2(id, session, fields)
        }
        (None, Some(length)) if length > ROW_2_BYTES => Err("record_capacity"),
        (None, Some(length)) if length > ROW_2_BYTES.saturating_sub(*scanned) => {
            return Next::Full;
        }
        (None, _) => Err("record_invalid"),
    };
    match result {
        Ok(Some(event)) => step.events.push(event),
        Ok(None) => (),
        Err(reason) => *step.skipped.entry(reason.into()).or_default() += 1,
    }
    Next::Row
}

/// Takes the next assistant message after the cursor as one usage event. The
/// row is read in place by SQLite's JSON functions up to what is left of
/// [`ROW_2_BYTES`] for the read; only the step's own fields come back. A row
/// over [`ROW_2_BYTES`] is reported as `record_capacity`, never dropped
/// unseen.
fn next_2(
    db: &Connection,
    step: &mut Step,
    horizon: u64,
    scanned: &mut u64,
) -> Result<Next, Refusal> {
    let cursor = &step.checkpoint.database;
    type Row = (String, u64, String, Option<u64>, Option<String>);
    let row: Option<Row> = db
        .query_row(
            &format!(
                "SELECT CASE WHEN octet_length(id)<=128 THEN id END,time_updated,
             CASE WHEN octet_length(session_id)<=128 THEN session_id END,
             octet_length(data),
             CASE WHEN octet_length(data)<=?4 AND json_valid(data) THEN {FIELDS_2} END
             FROM session_message AS m WHERE type='assistant'
             AND (time_updated>?1 OR (time_updated=?1 AND id>?2)) AND time_updated<=?3
             ORDER BY time_updated,id LIMIT 1"
            ),
            params![
                cursor.after_time,
                cursor.after_id,
                horizon,
                ROW_2_BYTES.saturating_sub(*scanned)
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|_| Refusal::Database)?;
    let Some((id, updated, session, length, fields)) = row else {
        // Revisit the boundary timestamp next round: later commits may
        // insert another id with the same millisecond, in either order.
        step.checkpoint.database.after_time = horizon;
        step.checkpoint.database.after_id.clear();
        step.checkpoint.database.horizon = None;
        step.has_more = false;
        return Ok(Next::End);
    };
    if id.len() > 128 || session.len() > 128 {
        return Err(Refusal::Database);
    }
    let bytes = (id.len() + session.len()) as u64 + fields.as_ref().map_or(0, |s| s.len() as u64);
    let taken = take_2(step, id.clone(), session, length, fields, scanned);
    if matches!(taken, Next::Full) {
        return Ok(taken);
    }
    step.read_bytes += bytes;
    step.checkpoint.database.after_time = updated;
    step.checkpoint.database.after_id = id;
    step.has_more = true;
    Ok(taken)
}

/// Takes the next assistant message after the cursor in a database 2.x
/// migrated from 1.x, which holds both sets of tables. A message the 1.x
/// `message` table holds (the migration keeps its id) is a 1.x message: its
/// step-finish parts are read and keyed as before the upgrade. A message only
/// `session_message` holds was written after, and is one event keyed by its
/// id. The two walk one order, so one cursor and one horizon serve both.
fn next_migrated(
    db: &Connection,
    step: &mut Step,
    horizon: u64,
    scanned: &mut u64,
) -> Result<Next, Refusal> {
    let cursor = &step.checkpoint.database;
    type Row = (
        String,
        u64,
        String,
        u8,
        Option<String>,
        Option<u64>,
        Option<String>,
    );
    let row: Option<Row> = db
        .query_row(
            &format!(
                "SELECT CASE WHEN octet_length(id)<=128 THEN id END,time_updated,
             CASE WHEN octet_length(session_id)<=128 THEN session_id END,src,legacy,length,fields
             FROM (SELECT id,time_updated,session_id,0 AS src,
              CASE WHEN octet_length(data)<=?4 THEN data END AS legacy,
              NULL AS length,NULL AS fields
              FROM message
              UNION ALL
              SELECT id,time_updated,session_id,1,NULL,octet_length(data),
              CASE WHEN octet_length(data)<=?5 AND json_valid(data) THEN {FIELDS_2} END
              FROM session_message AS m WHERE type='assistant'
              AND NOT EXISTS (SELECT 1 FROM message WHERE message.id=m.id))
             WHERE (time_updated>?1 OR (time_updated=?1 AND id>?2)) AND time_updated<=?3
             ORDER BY time_updated,id LIMIT 1"
            ),
            params![
                cursor.after_time,
                cursor.after_id,
                horizon,
                ROW_BYTES,
                ROW_2_BYTES.saturating_sub(*scanned)
            ],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| Refusal::Database)?;
    let Some((id, updated, session, src, legacy, length, fields)) = row else {
        step.checkpoint.database.after_time = horizon;
        step.checkpoint.database.after_id.clear();
        step.checkpoint.database.horizon = None;
        step.has_more = false;
        return Ok(Next::End);
    };
    if id.len() > 128 || session.len() > 128 {
        return Err(Refusal::Database);
    }
    if src == 0 {
        step.read_bytes +=
            (id.len() + session.len()) as u64 + legacy.as_ref().map_or(0, |s| s.len() as u64);
        step.checkpoint.database.after_time = updated;
        step.checkpoint.database.after_id = id.clone();
        step.has_more = true;
        take_legacy(db, step, id, updated, session, legacy.as_deref())?;
        return Ok(Next::Row);
    }
    let bytes = (id.len() + session.len()) as u64 + fields.as_ref().map_or(0, |s| s.len() as u64);
    let taken = take_2(step, id.clone(), session, length, fields, scanned);
    if matches!(taken, Next::Full) {
        return Ok(taken);
    }
    step.read_bytes += bytes;
    step.checkpoint.database.after_time = updated;
    step.checkpoint.database.after_id = id;
    step.has_more = true;
    Ok(taken)
}
