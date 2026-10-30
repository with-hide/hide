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
//! page before it.

use std::collections::BTreeMap;
use std::path::Path;

use hide_agent_adapter::SessionFormat;
use serde_json::Value;

use crate::{Agent, ReadRefusal, SessionReadScope};

pub use agent_client_protocol_schema::v1 as acp;

mod build;
mod claude;

use build::Thread;

/// The most record bytes one page reads; a record longer than this is
/// passed over and counted as `line_capacity`.
pub const THREAD_PAGE_BYTES: u64 = 4 * 1024 * 1024;
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

/// One page of a session's thread.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ThreadPage {
    /// The page's updates in record order.
    pub updates: Vec<acp::SessionUpdate>,
    /// The page before this one, `None` at the session's start.
    pub before: Option<ThreadCursor>,
    /// The records the page passed over, by reason, such as a record past
    /// the page cap (`line_capacity`) or one whose shape the converter does
    /// not know.
    pub skipped: BTreeMap<&'static str, usize>,
}

/// How one record format is read.
#[derive(Clone, Copy)]
enum Reader {
    /// One JSON record per line.
    Lines(Lines),
    /// Declared unreadable; the adapter row says why.
    Unsupported,
}

#[derive(Clone, Copy)]
struct Lines {
    /// What one record adds to the thread.
    record: fn(&Value, &mut Thread),
    /// Whether a page may begin at this record: a format whose results
    /// follow their calls in order keeps a call and its results on one page.
    may_start_page: fn(&Value) -> bool,
}

const fn reader(format: SessionFormat) -> Reader {
    match format {
        SessionFormat::Claude => Reader::Lines(claude::LINES),
        SessionFormat::Codex
        | SessionFormat::Grok
        | SessionFormat::Pi
        | SessionFormat::Omp
        | SessionFormat::OpenCode
        | SessionFormat::Antigravity
        | SessionFormat::Cursor => Reader::Unsupported,
    }
}

/// Whether sessions of `format` are read as threads.
pub const fn reads(format: SessionFormat) -> bool {
    !matches!(reader(format), Reader::Unsupported)
}

/// The page of `agent`'s session that ends at `before`, or its newest page.
///
/// A file format reads `path`, which must lie in the agent's session folder
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
    match reader(agent.format()) {
        Reader::Unsupported => Err(ReadRefusal::from("thread_unsupported")),
        Reader::Lines(lines) => {
            let path = path.ok_or(ReadRefusal::from("thread_path_required"))?;
            crate::inside_session_root(home, &[agent], path)
                .map_err(|_| ReadRefusal::from("thread_outside_session_root"))?;
            crate::read_session_file(home, agent, path, Some(scope), || {
                read_lines(path, lines, before)
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
    for (_, record) in &records {
        (lines.record)(record, &mut thread);
    }
    thread.forget_awaiting();
    if thread.nothing_readable() {
        return Err("thread_records_unreadable".to_owned());
    }
    let (updates, skipped) = thread.into_parts();
    Ok(ThreadPage {
        updates,
        before: (start > 0).then(|| ThreadCursor {
            position: start,
            incarnation,
        }),
        skipped,
    })
}

fn incarnation(path: &Path) -> Result<String, String> {
    let file = crate::open_session_file(path).map_err(|_| "thread_file_unreadable".to_owned())?;
    let id = hide_platform::fs::identity::file_id_of(&file)
        .map_err(|_| "thread_file_unreadable".to_owned())?;
    Ok(format!("{}:{}", id.volume(), id.index()))
}
