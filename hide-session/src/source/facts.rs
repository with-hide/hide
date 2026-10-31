//! What a session's own records and files say about it, per agent: its
//! native id, its checkout, when it was made, and the record that names
//! them. Every reader that needs one of these facts (the owner proof, the
//! catalog, the locator, the thread view, links, the omp title) reads it
//! here, so each fact has one answer for every feature.
//!
//! - Claude Code names its id on the first person's or assistant's record
//!   and its cwd on nearly every record. The first person's record can carry
//!   pasted images far larger than any record cap, so a record over the cap
//!   is passed over and the scan goes on, bounded like a whole-file read.
//! - Codex names both in its `session_meta` record, which it writes first.
//! - Pi and omp name both in a strict header on the first line (omp may put
//!   its title slot first); Grok in its `summary.json`; agy in its index.
//!
//! A record that names a session holds no paste, so it may be up to
//! [`NAMING_RECORD_LIMIT_BYTES`]; one longer is refused for its size,
//! never read as a session without one.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde_json::Value;

use super::frame::{Head, HeadRecord};
use crate::{Agent, Result, SESSION_LINE_LIMIT_BYTES, SESSION_READ_LIMIT_BYTES, SessionError};

/// The largest record that names a session (Codex `session_meta`, a Pi or
/// omp header, Grok's `summary.json`) a reader keeps.
pub const NAMING_RECORD_LIMIT_BYTES: usize = 1024 * 1024;

/// The id, checkout and opening time a session's native header names.
pub(crate) struct Header {
    pub id: String,
    pub cwd: PathBuf,
    /// When the session itself was made, which a fork's copied records do
    /// not tell: Pi's and omp's header `timestamp`, Grok's summary
    /// `created_at`.
    pub opened_at_unix_ms: Option<u64>,
}

fn naming_capacity() -> SessionError {
    SessionError::Capacity {
        resource: "line_bytes",
        limit: NAMING_RECORD_LIMIT_BYTES as u64,
    }
}

/// What a Claude Code session's head names.
#[derive(Debug, Default)]
pub(crate) struct ClaudeHead {
    /// `sessionId` of the first person's or assistant's record.
    pub id: Option<String>,
    /// `cwd` of the first record that names one.
    pub cwd: Option<PathBuf>,
}

/// Which of [`ClaudeHead`]'s facts a scan stops once it has.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wanted {
    Id,
    Cwd,
}

/// Claude Code's id or cwd, read from the start of `reader` past any record
/// over the record cap, at most [`SESSION_READ_LIMIT_BYTES`] in all.
pub(crate) fn claude(reader: impl Read, wanted: Wanted) -> std::io::Result<ClaudeHead> {
    let mut head = Head::new(
        reader.take(SESSION_READ_LIMIT_BYTES),
        SESSION_LINE_LIMIT_BYTES,
    );
    let mut found = ClaudeHead::default();
    while let Some(record) = head.next_record()? {
        // A last record without its newline still names its facts once it
        // is whole JSON; one cut mid-record parses to nothing.
        let (record, last) = match record {
            HeadRecord::Complete(record) => (record, false),
            HeadRecord::Unfinished(record) => (record, true),
            HeadRecord::Passed => continue,
        };
        let Ok(record) = serde_json::from_slice::<Value>(&record) else {
            if last {
                break;
            }
            continue;
        };
        if found.cwd.is_none()
            && let Some(cwd) = record.get("cwd").and_then(Value::as_str)
        {
            found.cwd = Some(PathBuf::from(cwd));
        }
        if found.id.is_none() && matches!(record["type"].as_str(), Some("user" | "assistant")) {
            found.id = record["sessionId"].as_str().map(str::to_owned);
        }
        let done = match wanted {
            Wanted::Id => found.id.is_some(),
            Wanted::Cwd => found.cwd.is_some(),
        };
        if done {
            break;
        }
    }
    Ok(found)
}

/// The first record among the first `records` of `reader`'s head that
/// `picks` chooses, each kept up to [`NAMING_RECORD_LIMIT_BYTES`] and the
/// scan bounded by [`SESSION_READ_LIMIT_BYTES`]. A record over the cap is
/// passed over; when nothing is chosen and the first record was one, the
/// session's naming record is over the cap and that is the answer.
pub(crate) fn naming_record(
    reader: impl Read,
    records: usize,
    picks: impl Fn(&Value) -> bool,
) -> Result<Option<Value>> {
    let mut head = Head::new(
        reader.take(SESSION_READ_LIMIT_BYTES),
        NAMING_RECORD_LIMIT_BYTES,
    );
    let mut first_passed = false;
    for index in 0..records {
        let record = match head.next_record() {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(_) => return Ok(None),
        };
        match record {
            HeadRecord::Complete(record) => {
                if let Ok(record) = serde_json::from_slice::<Value>(&record)
                    && picks(&record)
                {
                    return Ok(Some(record));
                }
            }
            HeadRecord::Passed => first_passed |= index == 0,
            HeadRecord::Unfinished(_) => break,
        }
    }
    if first_passed {
        return Err(naming_capacity());
    }
    Ok(None)
}

/// Codex's `session_meta`, the record that names the session's id and cwd.
/// `None` is a file that cannot be opened or names no session.
pub(crate) fn codex_meta(path: &Path) -> Result<Option<Value>> {
    let Ok(file) = super::open(path) else {
        return Ok(None);
    };
    codex_meta_of(file)
}

/// [`codex_meta`] of an open file.
pub(crate) fn codex_meta_of(reader: impl Read) -> Result<Option<Value>> {
    naming_record(reader, usize::MAX, |record| {
        record["type"] == "session_meta"
    })
}

/// The cwd Codex's `session_meta` names.
pub(crate) fn codex_cwd(path: &Path) -> Result<Option<String>> {
    Ok(codex_meta(path)?.and_then(|value| {
        value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }))
}

/// The native header of a Pi, omp, Grok or agy session.
pub(crate) fn header(agent: Agent, path: &Path) -> anyhow::Result<Header> {
    let mut remaining = u64::MAX;
    header_budgeted(agent, path, &mut remaining)
}

/// [`header`], charging the bytes it reads to `remaining`.
pub(crate) fn header_budgeted(
    agent: Agent,
    path: &Path,
    remaining: &mut u64,
) -> anyhow::Result<Header> {
    if agent == Agent::Antigravity {
        // The index row, not the transcript, names the conversation's checkout.
        return crate::antigravity::header(path);
    }
    if agent == Agent::Grok {
        let mut read = 0;
        let summary = crate::grok::summary(path, &mut read);
        // A summary over the naming cap is refused for its size first: that
        // is what is wrong with it, whatever the budget left.
        if let Err(error) = &summary
            && error.to_string() == "label_session_metadata_line_capacity"
        {
            return summary.map(|_| unreachable!());
        }
        if read > *remaining {
            return Err(anyhow!("session_discovery_read_capacity"));
        }
        *remaining -= read;
        let summary = summary?;
        return Ok(Header {
            id: summary.id,
            cwd: summary.cwd,
            opened_at_unix_ms: summary.created_at_unix_ms,
        });
    }
    let file = super::open(path).map_err(|_| anyhow!("label_session_file_unavailable"))?;
    native_header(agent, file, remaining)
}

/// The next record of a strict header: whole, within the naming cap and
/// JSON, its bytes charged to `remaining`.
fn header_record(head: &mut Head<impl Read>, remaining: &mut u64) -> anyhow::Result<Value> {
    let before = head.consumed();
    let record = head
        .next_record()
        .map_err(|_| anyhow!("label_session_metadata_read_failed"))?;
    let read = head.consumed() - before;
    // A header over the naming cap is refused for its size first: that is
    // what is wrong with it, whatever the budget left.
    if record == Some(HeadRecord::Passed) {
        return Err(anyhow!("label_session_metadata_line_capacity"));
    }
    if read > *remaining {
        return Err(anyhow!("session_discovery_read_capacity"));
    }
    *remaining -= read;
    match record {
        Some(HeadRecord::Complete(record)) => serde_json::from_slice(&record)
            .map_err(|_| anyhow!("label_session_metadata_unconfirmed")),
        Some(HeadRecord::Passed) => Err(anyhow!("label_session_metadata_line_capacity")),
        Some(HeadRecord::Unfinished(record)) if record.len() as u64 > *remaining => {
            Err(anyhow!("session_discovery_read_capacity"))
        }
        Some(HeadRecord::Unfinished(_)) | None => {
            Err(anyhow!("label_session_metadata_unconfirmed"))
        }
    }
}

/// Pi's and omp's header from the start of `reader`: the first line, or for
/// omp the line after its title slot, a version-3 `session` record naming a
/// valid id and an absolute cwd.
pub(crate) fn native_header(
    agent: Agent,
    reader: impl Read,
    remaining: &mut u64,
) -> anyhow::Result<Header> {
    // Past the budget by one naming record, so a header too long to name a
    // session is told from one the budget cut.
    let mut head = Head::new(
        reader.take(remaining.saturating_add(NAMING_RECORD_LIMIT_BYTES as u64 + 1)),
        NAMING_RECORD_LIMIT_BYTES,
    );
    let first = header_record(&mut head, remaining)?;
    let value = if agent == Agent::Omp && first["type"] == "title" {
        crate::native_file::title_snapshot(&first)
            .ok_or_else(|| anyhow!("label_session_metadata_unconfirmed"))?;
        header_record(&mut head, remaining)?
    } else {
        first
    };
    if value["type"] != "session" || value["version"] != 3 {
        return Err(anyhow!("label_session_metadata_unconfirmed"));
    }
    let id = value["id"]
        .as_str()
        .filter(|id| crate::label_owner::valid_native_id(id))
        .ok_or_else(|| anyhow!("label_session_id_invalid"))?;
    let cwd = value["cwd"]
        .as_str()
        .filter(|cwd| !cwd.chars().any(char::is_control))
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute())
        .ok_or_else(|| anyhow!("label_session_cwd_unconfirmed"))?;
    Ok(Header {
        id: id.to_owned(),
        cwd,
        opened_at_unix_ms: crate::timestamp_ms(value.get("timestamp")).ok(),
    })
}

/// omp's current title from its physical first line (the title slot, or a
/// header that carries the title), read through `reader` within what is left
/// of the poll's `budget`; `read_bytes` is charged every byte read.
pub(crate) fn omp_title(
    path: &Path,
    reader: impl Read,
    budget: u64,
    read_bytes: &mut u64,
) -> Result<(String, String)> {
    struct Counted<'a, R> {
        reader: R,
        read: &'a mut u64,
    }
    impl<R: Read> Read for Counted<'_, R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.reader.read(buffer)?;
            *self.read += read as u64;
            Ok(read)
        }
    }
    let spent = || SessionError::Capacity {
        resource: "title_read_bytes",
        limit: budget,
    };
    let unconfirmed = || SessionError::Checkpoint("label_session_metadata_unconfirmed".to_owned());
    let allowance = budget
        .min(crate::SESSION_INCREMENT_READ_LIMIT_BYTES)
        .saturating_sub(*read_bytes);
    if allowance == 0 {
        return Err(spent());
    }
    let before = *read_bytes;
    let record = {
        let counted = Counted {
            reader: reader.take(allowance),
            read: &mut *read_bytes,
        };
        Head::new(counted, NAMING_RECORD_LIMIT_BYTES)
            .with_block(TITLE_BLOCK_BYTES)
            .next_record()
            .map_err(|error| SessionError::io("read", path, error))?
    };
    match record {
        Some(HeadRecord::Complete(record)) => {
            let value = serde_json::from_slice(&record).map_err(|_| unconfirmed())?;
            crate::native_file::title_snapshot(&value).ok_or_else(unconfirmed)
        }
        Some(HeadRecord::Passed) => Err(naming_capacity()),
        Some(HeadRecord::Unfinished(_)) if *read_bytes - before >= allowance => Err(spent()),
        Some(HeadRecord::Unfinished(_)) | None => Err(unconfirmed()),
    }
}

/// The block a title read takes at a time, so a short first line costs the
/// poll little of its allowance.
const TITLE_BLOCK_BYTES: usize = 4 * 1024;
