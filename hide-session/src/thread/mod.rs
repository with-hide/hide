//! One agent session read as a thread of ACP session updates (PRD
//! session-thread-acp): the operator's messages and images, the agent's
//! answers and thoughts, tool calls with their results, and plans, in the
//! shape of the Agent Client Protocol's `session/update` notifications.
//!
//! Every format is read from the agent's own records, never through an ACP
//! adapter's `session/load`, which writes to the session it loads (D-03).
//! The read writes nothing.
//!
//! Each record format has one converter, chosen by [`reader`], which matches
//! every [`SessionFormat`]: a new format does not compile until it has a
//! converter or is declared unreadable here, and the adapter row's `thread`
//! capability must agree with that choice (`tests/it/thread.rs`). A converter
//! reads one record and tells the builder ([`build::Thread`]) what it holds;
//! only the builder makes ACP values.
//!
//! Pages run newest first. A page is at most [`THREAD_PAGE_BYTES`] of whole
//! records, given in record order; its [`ThreadPage::before`] cursor reads the
//! page before it. The page a session is still writing is read forward from
//! its start ([`ThreadRead::From`]), at most [`THREAD_TAIL_BYTES`] past it,
//! so a reader that follows a working session reads the same records again
//! only from where its last whole page ended.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use hide_agent_adapter::SessionFormat;
use serde_json::Value;

use crate::{Agent, ReadRefusal, SessionReadScope};

pub use agent_client_protocol_schema::v1 as acp;

mod antigravity;
mod build;
mod claude;
mod codex;
mod grok;
mod hide_call;
mod opencode;
mod pi;

pub use hide_call::{HideCall, ViewKind, hide_call, spawned_agent};

use build::Thread;

/// The most record bytes one page reads; a record longer than this is
/// passed over and counted as `line_capacity`.
pub const THREAD_PAGE_BYTES: u64 = 4 * 1024 * 1024;
/// The record bytes past which a forward page ends, at the next record a
/// page may begin at: the page a session is still writing stays this small,
/// so reading it again costs this much and not a whole page.
pub const THREAD_TAIL_BYTES: u64 = 512 * 1024;
/// The most bytes the session's first record may hold when a format reads
/// every record beside it.
const HEADER_BYTES: usize = 1024 * 1024;
/// How many of the session's first records the header is looked for in.
const HEADER_RECORDS: usize = 8;
/// The most characters one message, output or file side keeps; past it the
/// text is cut and marked `_meta.hide.truncated`.
pub const TEXT_LIMIT_CHARS: usize = 20_000;
/// The most base64 bytes one image keeps; a larger image keeps its type and
/// size and leaves its data out, marked `_meta.hide.truncated`.
pub const IMAGE_LIMIT_BYTES: usize = 2 * 1024 * 1024;

/// Where the page before a [`ThreadPage`] ends, for the session it came from.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreadCursor {
    /// A record boundary: a byte offset in a session file.
    position: u64,
    /// The file the position belongs to; a file replaced at the same path
    /// refuses the cursor instead of reading from a stranger's boundary.
    incarnation: String,
}

impl ThreadCursor {
    /// Whether the cursor stands at the session's first record.
    pub fn at_start(&self) -> bool {
        self.position == 0
    }
}

/// One page of a session's thread.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ThreadPage {
    /// The page's updates in record order.
    pub updates: Vec<acp::SessionUpdate>,
    /// Where the page begins: the cursor a forward read of it starts from.
    pub start: ThreadCursor,
    /// The page before this one, `None` at the session's start.
    pub before: Option<ThreadCursor>,
    /// Where the page's records end and the page after it begins: the
    /// cursor a forward read of what the session writes next starts from.
    pub end: ThreadCursor,
    /// Whether a forward read stopped before the session's end, with
    /// records already written past [`ThreadPage::end`].
    #[serde(default)]
    pub more: bool,
    /// The records the page passed over, by reason, such as a record past
    /// the page cap (`line_capacity`) or one whose shape the converter does
    /// not know.
    pub skipped: BTreeMap<String, usize>,
}

/// Which page of a session a read asks for.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadRead {
    /// The newest page.
    Newest,
    /// The page that ends where an earlier page began.
    Before(ThreadCursor),
    /// The records from a page's start toward the session's end, ending past
    /// [`THREAD_TAIL_BYTES`] at a record a page may begin at.
    From(ThreadCursor),
}

/// A page of the session an agent pane runs, named the way Herdr reports
/// the pane's session (`agent_session.kind` and its value), as the label
/// read names it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadRequest {
    pub agent: Agent,
    /// `id` or `path`.
    pub reference_kind: String,
    pub reference_value: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub read: ThreadRead,
}

/// How one record format is read.
#[derive(Clone, Copy)]
enum Reader {
    /// One JSON record per line.
    Lines(Lines),
    /// OpenCode's database, both of its schemas.
    OpenCode,
    /// Declared unreadable; the adapter row says why.
    Unsupported,
}

#[derive(Clone, Copy)]
struct Lines {
    /// Picks, among the session's first records, the one that says how the
    /// others are written (Codex's history mode, Pi's checkout), so each
    /// record is read beside it.
    header: Option<fn(&Value) -> bool>,
    /// What one record adds to the thread.
    record: fn(&Value, &Session<'_>, &mut Thread),
    /// Whether a page may begin at this record: a format whose results
    /// follow their calls in order keeps a call and its results on one page.
    may_start_page: fn(&Value) -> bool,
}

/// When a line record was written: its `timestamp`, an RFC 3339 time or
/// Unix seconds or milliseconds (Grok), or agy's `created_at`.
fn record_at(record: &Value) -> Option<u64> {
    crate::timestamp_ms(record.get("timestamp").or_else(|| record.get("created_at"))).ok()
}

/// What a converter knows of the session besides the record it reads.
struct Session<'a> {
    /// The record [`Lines::header`] picked, `Value::Null` without one.
    header: &'a Value,
    /// The session file, for a format that keeps a record's attachments
    /// beside it (omp's blobs).
    path: &'a Path,
}

const fn reader(format: SessionFormat) -> Reader {
    match format {
        SessionFormat::Claude => Reader::Lines(claude::LINES),
        SessionFormat::Codex => Reader::Lines(codex::LINES),
        SessionFormat::Grok => Reader::Lines(grok::LINES),
        SessionFormat::Pi | SessionFormat::Omp => Reader::Lines(pi::LINES),
        SessionFormat::OpenCode => Reader::OpenCode,
        SessionFormat::Antigravity => Reader::Lines(antigravity::LINES),
        SessionFormat::Cursor => Reader::Unsupported,
    }
}

/// Whether sessions of `format` are read as threads.
pub const fn reads(format: SessionFormat) -> bool {
    !matches!(reader(format), Reader::Unsupported)
}

/// The page of `agent`'s session that ends at `before`, or its newest page.
///
/// OpenCode's database is read for the root session `scope` names. A file
/// format reads `path`, which must lie in the agent's session folder
/// and, for a format that proves its owner from native records, belong to
/// `scope`. A read that cannot answer returns a named refusal, never an
/// empty page: a format declared unreadable, a missing or replaced file, a
/// stale cursor, or a page whose every conversation record failed to parse.
pub fn read_thread(
    home: &Path,
    agent: Agent,
    path: Option<&Path>,
    scope: &SessionReadScope,
    before: Option<&ThreadCursor>,
) -> Result<ThreadPage, ReadRefusal> {
    let read = match before {
        Some(cursor) => ThreadRead::Before(cursor.clone()),
        None => ThreadRead::Newest,
    };
    read_page(home, agent, path, scope, &read)
}

/// The page of `agent`'s session that begins at `from`, read forward: the
/// records from there toward the session's end, ending past
/// [`THREAD_TAIL_BYTES`] at a record a page may begin at, its
/// [`ThreadPage::more`] saying whether records follow its end already. A record still being
/// written is left for the next read. Refused as [`read_thread`] refuses,
/// and a page that holds no record yet is answered empty.
pub fn read_thread_from(
    home: &Path,
    agent: Agent,
    path: Option<&Path>,
    scope: &SessionReadScope,
    from: &ThreadCursor,
) -> Result<ThreadPage, ReadRefusal> {
    read_page(home, agent, path, scope, &ThreadRead::From(from.clone()))
}

/// The page `request` asks for of the session an agent pane runs, located
/// and proven its native owner's as the label read proves it (an `id`
/// reference must be a plain id, a reported path must lie in the agent's
/// session folder and belong to the session it names).
pub fn read_pane_thread(home: &Path, request: &ThreadRequest) -> Result<ThreadPage, ReadRefusal> {
    if request.reference_kind == "id"
        && !crate::label_transcript::is_session_id(&request.reference_value)
    {
        return Err("label_session_id_invalid".into());
    }
    if !reads(request.agent.format()) {
        return Err("thread_unsupported".into());
    }
    if request.agent == Agent::OpenCode {
        if request.reference_kind != "id" {
            return Err("session_kind_unsupported".into());
        }
        let scope = SessionReadScope {
            id: request.reference_value.clone(),
            cwd: request.cwd.clone().ok_or("session_route_unconfirmed")?,
        };
        return read_page(home, request.agent, None, &scope, &request.read);
    }
    let (path, confirmed) = crate::label_transcript::locate_confirmed(
        home,
        request.agent,
        &request.reference_kind,
        &request.reference_value,
        request.cwd.as_deref(),
    )?;
    let scope = SessionReadScope {
        id: match confirmed.native_session_id {
            Some(id) => id,
            None if request.reference_kind == "id" => request.reference_value.clone(),
            None => return Err("label_session_id_unconfirmed".into()),
        },
        cwd: request.cwd.clone().unwrap_or_default(),
    };
    read_page(home, request.agent, Some(&path), &scope, &request.read)
}

fn read_page(
    home: &Path,
    agent: Agent,
    path: Option<&Path>,
    scope: &SessionReadScope,
    read: &ThreadRead,
) -> Result<ThreadPage, ReadRefusal> {
    match reader(agent.format()) {
        Reader::Unsupported => Err(ReadRefusal::from("thread_unsupported")),
        Reader::OpenCode => opencode::read(home, scope, read),
        Reader::Lines(lines) => {
            let path = path.ok_or(ReadRefusal::from("thread_path_required"))?;
            crate::inside_session_root(home, &[agent], path)
                .map_err(|_| ReadRefusal::from("thread_outside_session_root"))?;
            crate::read_session_file(home, agent, path, Some(scope), || match read {
                ThreadRead::Newest => read_lines(path, lines, None),
                ThreadRead::Before(cursor) => read_lines(path, lines, Some(cursor)),
                ThreadRead::From(cursor) => read_lines_from(path, lines, cursor),
            })
            .map_err(ReadRefusal::from)
        }
    }
}

fn read_lines(
    path: &Path,
    lines: Lines,
    before: Option<&ThreadCursor>,
) -> Result<ThreadPage, String> {
    let incarnation = incarnation(path)?;
    if before.is_some_and(|cursor| cursor.incarnation != incarnation) {
        return Err("thread_cursor_stale".to_owned());
    }
    let page = crate::read_page_before(
        path,
        before.map(|cursor| cursor.position),
        THREAD_PAGE_BYTES,
    )
    .map_err(|_| "thread_file_unreadable".to_owned())?;
    let mut thread = Thread::default();
    if page.contents.is_empty() && page.start_offset < page.end_offset {
        thread.skip("line_capacity");
    }
    let mut records = Vec::new();
    let mut offset = page.start_offset;
    for line in page.contents.split_inclusive('\n') {
        let at = offset;
        offset += line.len() as u64;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(record) => records.push((at, record)),
            Err(_) => thread.unreadable("record_unparsable"),
        }
    }
    // A page that does not start the file begins at the first record a page
    // may begin at; the records before it are read with the older page.
    let mut start = page.start_offset;
    if start > 0
        && let Some(first) = records
            .iter()
            .position(|(_, record)| (lines.may_start_page)(record))
    {
        start = records[first].0;
        records.drain(..first);
    }
    let header = match lines.header {
        Some(picks) => header(path, picks)?,
        None => Value::Null,
    };
    let session = Session {
        header: &header,
        path,
    };
    for (_, record) in &records {
        thread.at(record_at(record));
        (lines.record)(record, &session, &mut thread);
    }
    thread.end_turn();
    if thread.nothing_readable() {
        return Err("thread_records_unreadable".to_owned());
    }
    let cursor = |position| ThreadCursor {
        position,
        incarnation: incarnation.clone(),
    };
    Ok(page_of(
        thread,
        cursor(start),
        cursor(page.end_offset),
        false,
    ))
}

/// The page a converted run of records makes.
fn page_of(thread: Thread, start: ThreadCursor, end: ThreadCursor, more: bool) -> ThreadPage {
    let (updates, skipped) = thread.into_parts();
    ThreadPage {
        updates,
        before: (!start.at_start()).then(|| start.clone()),
        start,
        end,
        more,
        skipped: skipped
            .into_iter()
            .map(|(reason, count)| (reason.to_owned(), count))
            .collect(),
    }
}

/// The records from `from` toward the end of the file, read forward: the
/// page ends past [`THREAD_TAIL_BYTES`] at the next record a page may begin
/// at, or past [`THREAD_PAGE_BYTES`] wherever it is, and never takes a last
/// line that has no newline yet.
fn read_lines_from(path: &Path, lines: Lines, from: &ThreadCursor) -> Result<ThreadPage, String> {
    let incarnation = incarnation(path)?;
    if from.incarnation != incarnation {
        return Err("thread_cursor_stale".to_owned());
    }
    let mut file =
        crate::open_session_file(path).map_err(|_| "thread_file_unreadable".to_owned())?;
    file.seek(SeekFrom::Start(from.position))
        .map_err(|_| "thread_file_unreadable".to_owned())?;
    let mut reader = BufReader::new(file);
    let mut thread = Thread::default();
    let mut records = Vec::new();
    let mut offset = from.position;
    let mut after = None;
    while let Some((line, length)) =
        next_line(&mut reader, THREAD_PAGE_BYTES as usize).map_err(|_| "thread_file_unreadable")?
    {
        let read = offset - from.position;
        let full = read >= THREAD_TAIL_BYTES;
        if read >= THREAD_PAGE_BYTES {
            after = Some(offset);
            break;
        }
        match line {
            None => thread.skip("line_capacity"),
            Some(line) if line.trim_ascii().is_empty() => {}
            Some(line) => match serde_json::from_slice::<Value>(&line) {
                Ok(record) if full && (lines.may_start_page)(&record) => {
                    after = Some(offset);
                    break;
                }
                Ok(record) => records.push(record),
                Err(_) => thread.unreadable("record_unparsable"),
            },
        }
        offset += length;
    }
    let header = match lines.header {
        Some(picks) => header(path, picks)?,
        None => Value::Null,
    };
    let session = Session {
        header: &header,
        path,
    };
    for record in &records {
        thread.at(record_at(record));
        (lines.record)(record, &session, &mut thread);
    }
    thread.end_turn();
    if thread.nothing_readable() {
        return Err("thread_records_unreadable".to_owned());
    }
    let end = ThreadCursor {
        position: after.unwrap_or(offset),
        incarnation,
    };
    Ok(page_of(thread, from.clone(), end, after.is_some()))
}

/// The next whole line `reader` holds and how many bytes it took: the line
/// itself when it is at most `limit` bytes, `None` for a longer one, which
/// is read past without being kept. `None` overall at the end of the file,
/// where a line without its newline is still being written.
fn next_line(
    reader: &mut impl BufRead,
    limit: usize,
) -> std::io::Result<Option<(Option<Vec<u8>>, u64)>> {
    let mut kept = Some(Vec::new());
    let mut length = 0u64;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(None);
        }
        let (chunk, ended) = match buffer.iter().position(|byte| *byte == b'\n') {
            Some(newline) => (&buffer[..=newline], true),
            None => (buffer, false),
        };
        let taken = chunk.len();
        if let Some(line) = kept.as_mut() {
            if line.len() + taken > limit + 1 {
                kept = None;
            } else {
                line.extend_from_slice(chunk);
            }
        }
        reader.consume(taken);
        length += taken as u64;
        if ended {
            return Ok(Some((kept, length)));
        }
    }
}

/// The first of the session's first [`HEADER_RECORDS`] records that `picks`
/// chooses, read on its own whatever page is asked for.
fn header(path: &Path, picks: fn(&Value) -> bool) -> Result<Value, String> {
    let file = crate::open_session_file(path).map_err(|_| "thread_file_unreadable".to_owned())?;
    let mut reader = std::io::BufReader::new(file);
    for _ in 0..HEADER_RECORDS {
        let Some(line) = crate::read_bounded_line(&mut reader, HEADER_BYTES)
            .map_err(|_| "thread_header_unreadable".to_owned())?
        else {
            break;
        };
        if let Ok(record) = serde_json::from_slice::<Value>(&line)
            && picks(&record)
        {
            return Ok(record);
        }
    }
    Err("thread_header_unreadable".to_owned())
}

fn incarnation(path: &Path) -> Result<String, String> {
    let file = crate::open_session_file(path).map_err(|_| "thread_file_unreadable".to_owned())?;
    let id = hide_platform::fs::identity::file_id_of(&file)
        .map_err(|_| "thread_file_unreadable".to_owned())?;
    Ok(format!("{}:{}", id.volume(), id.index()))
}
