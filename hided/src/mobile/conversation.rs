//! An agent's conversation on the phone: the 대화 half of its detail.
//!
//! Claude Code and Codex draw in the terminal's alternate screen, so Herdr
//! keeps no scrollback for their panes and `pane.read` holds only the screen.
//! The whole conversation is in the agent's own transcript, found from the
//! session Herdr reports for the pane and read through `hide-session`, on
//! this Mac only: an SSH device's transcript is on that device, so its detail
//! shows the terminal alone. A pane with no reported session shows the
//! terminal too, because the newest transcript in its folder may be a
//! neighbour's.
//!
//! A page is the newest `PAGE_MESSAGES` messages before a native ordinal, read
//! backwards in `WINDOW_BYTES` windows and never more than
//! `PAGE_BUDGET_BYTES` a request; the offset of its oldest message is the
//! cursor the phone sends back for the page before it. What the agent appends
//! is read the same way down to where the last read ended, so a transcript
//! line of any size is passed over rather than stopping the reader. Only the
//! operator's messages, the agent's text and an interruption leave hided:
//! tool calls, tool output and injected context never do. OpenCode keeps
//! its sessions in a database rather than a transcript file; its pages are
//! read by message ([`opencode`]).

mod opencode;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use herdr_core::wire::{self, PaneSessionKind};
use hide_herdr_client::ApiConnector;
use hide_session::{
    Agent, EventKind, SessionError, SessionIdentity, SessionLocator, parse_events_at,
    read_page_before,
};
use serde::Serialize;

use super::pane::PaneError;

/// The messages one page carries at most.
pub const PAGE_MESSAGES: usize = 30;
/// The text one page carries at most, past its newest message.
const PAGE_TEXT_BYTES: usize = 256 * 1024;
/// One backwards read.
const WINDOW_BYTES: u64 = 512 * 1024;
/// The most one page or one catch-up reads; a longer catch-up starts over
/// from the newest page.
const PAGE_BUDGET_BYTES: u64 = 8 * 1024 * 1024;
/// The characters one message carries; the rest is cut and marked.
pub const MESSAGE_CHARS: usize = 20_000;

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Message {
    /// The native record's ordinal: unique and in order.
    pub id: u64,
    /// `you`, `agent`, or `stopped` for an interruption.
    pub who: &'static str,
    pub text: String,
    pub truncated: bool,
    pub at_ms: u64,
    /// The native message this record is a part of (Grok writes a prompt's
    /// blocks and an answer's text runs as separate records).
    #[serde(skip)]
    part: Option<String>,
}

impl Message {
    fn continued_by(&self, next: &Self) -> bool {
        self.part.is_some() && self.part == next.part && self.who == next.who
    }
}

/// Join consecutive records of one native message into the first one.
fn join(messages: Vec<Message>) -> Vec<Message> {
    let mut joined: Vec<Message> = Vec::with_capacity(messages.len());
    for message in messages {
        match joined.last_mut() {
            Some(last) if last.continued_by(&message) => {
                if !last.truncated {
                    last.text
                        .push_str(if last.who == "agent" { "\n\n" } else { "\n" });
                    last.text.push_str(&message.text);
                    if let Some((cut, _)) = last.text.char_indices().nth(MESSAGE_CHARS) {
                        last.text.truncate(cut);
                        last.truncated = true;
                    }
                    last.truncated |= message.truncated;
                }
            }
            _ => joined.push(message),
        }
    }
    joined
}

#[derive(Debug, Eq, PartialEq)]
pub struct Page {
    /// Oldest first.
    pub messages: Vec<Message>,
    /// The cursor for the page before this one; `None` at the start.
    pub before: Option<u64>,
}

/// Which transcript a pane's agent writes, as Herdr reports it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Source {
    agent: Agent,
    identity: SessionIdentity,
    cwd: Option<String>,
}

/// The pane's transcript source; `None` when its agent has no conversation
/// reader or reported no session.
pub fn source(
    connector: &Arc<dyn ApiConnector>,
    pane_id: &str,
) -> Result<Option<Source>, PaneError> {
    let value = hide_herdr_client::request_with_connector(
        connector.as_ref(),
        "pane.get",
        serde_json::json!({"pane_id": pane_id}),
        TIMEOUT,
    )
    .map_err(|error| match error.code() {
        Some(code) if code.contains("not_found") => PaneError::Gone,
        _ => PaneError::Unavailable(error.to_string()),
    })?;
    let (cwd, session) = wire::pane_session(value).map_err(PaneError::Unavailable)?;
    let Some(session) = session else {
        return Ok(None);
    };
    let Some(agent) = Agent::from_kind(&session.agent)
        .and_then(|agent| agent.format().adapter().conversation)
        .map(Agent::from_format)
    else {
        return Ok(None);
    };
    let identity = match session.kind {
        PaneSessionKind::Id => SessionIdentity::id(session.value),
        PaneSessionKind::Path => SessionIdentity::path(session.value),
    };
    Ok(Some(Source {
        agent,
        identity,
        cwd,
    }))
}

/// One pane's open conversation.
#[derive(Debug)]
pub struct Transcript(Open);

// One per open phone conversation and never collected, so its size is
// not worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Open {
    File(FileTranscript),
    OpenCode(Source, opencode::Transcript),
}

/// One pane's open transcript file.
#[derive(Debug)]
struct FileTranscript {
    source: Source,
    path: PathBuf,
    /// Where the messages the phone holds end: a line boundary.
    end: u64,
    /// The file's length at the last read, so an idle poll reads nothing.
    length: u64,
    proof: Option<NativeProof>,
    /// The newest message the phone holds, which an append may continue.
    newest: Option<Message>,
    /// Cursor's graph position; SQLite file length is not its append position.
    cursor: Option<hide_session::cursor::Checkpoint>,
}

/// The phone retains no native authority beyond its source. Every native page,
/// poll (including idle) and cloned pager proves that same source again.
#[derive(Clone, Debug)]
struct NativeProof {
    home: PathBuf,
    source: Source,
    confirmed: hide_session::ConfirmedLabelSession,
    stamp: String,
}

impl NativeProof {
    fn confirm(
        home: &Path,
        source: &Source,
        path: &Path,
    ) -> Result<hide_session::ConfirmedLabelSession, SessionError> {
        let reported_id = match &source.identity {
            SessionIdentity::Id(id) => Some(id.as_str()),
            SessionIdentity::Path(_) => None,
        };
        hide_session::confirm_session_file(
            home,
            source.agent,
            path,
            reported_id,
            source.cwd.as_deref(),
        )
        .map_err(|error| SessionError::Checkpoint(error.to_string()))
    }

    fn new(home: &Path, source: &Source, path: &Path) -> Result<Option<Self>, SessionError> {
        if !source.agent.requires_native_file_proof() {
            return Ok(None);
        }
        let confirmed = Self::confirm(home, source, path)?;
        let stamp = hide_session::stamp::stamp_at(path).ok_or(SessionError::SessionFileMissing)?;
        Ok(Some(Self {
            home: home.to_path_buf(),
            source: source.clone(),
            confirmed,
            stamp,
        }))
    }

    fn current(&self, path: &Path) -> Result<Self, SessionError> {
        let confirmed = Self::confirm(&self.home, &self.source, path)?;
        if confirmed.owner != self.confirmed.owner {
            return Err(SessionError::Checkpoint(
                "label_session_read_changed".to_owned(),
            ));
        }
        let stamp = hide_session::stamp::stamp_at(path).ok_or(SessionError::SessionFileMissing)?;
        Ok(Self {
            confirmed,
            stamp,
            ..self.clone()
        })
    }

    fn require_same(&self, path: &Path) -> Result<Self, SessionError> {
        let current = self.current(path)?;
        if current.confirmed.incarnation != self.confirmed.incarnation
            || current.confirmed.bytes < self.confirmed.bytes
            || (current.confirmed.bytes == self.confirmed.bytes && current.stamp != self.stamp)
            || (self.source.agent == Agent::Cursor && current.stamp != self.stamp)
        {
            return Err(SessionError::Checkpoint(
                "label_session_read_changed".to_owned(),
            ));
        }
        Ok(current)
    }
}

/// What the agent appended since the last poll.
#[derive(Debug, Eq, PartialEq)]
pub enum Tail {
    Messages(Vec<Message>),
    /// The file shrank, was replaced, or ran too far ahead: read it anew.
    Reset,
}

impl Transcript {
    /// Finds the conversation and reads its newest page. `SessionFileMissing`
    /// means the session is reported but has written nothing yet.
    pub fn open(home: &Path, pane_id: &str, source: Source) -> Result<(Self, Page), SessionError> {
        if source.agent == Agent::OpenCode {
            // OpenCode reports its session by id; a path names nothing here.
            let SessionIdentity::Id(id) = &source.identity else {
                return Err(SessionError::UnsupportedSessionKind);
            };
            let (open, page) = opencode::Transcript::open(home, id, source.cwd.as_deref())?;
            return Ok((Self(Open::OpenCode(source, open)), page));
        }
        let (open, page) = FileTranscript::open(home, pane_id, source)?;
        Ok((Self(Open::File(open)), page))
    }

    pub fn source(&self) -> &Source {
        match &self.0 {
            Open::File(open) => &open.source,
            Open::OpenCode(source, _) => source,
        }
    }

    /// What reads the page before a cursor, apart from the transcript, so it
    /// can run while the transcript keeps polling.
    pub fn pager(&self) -> Pager {
        Pager(match &self.0 {
            Open::File(open) => Paging::File(open.pager()),
            Open::OpenCode(_, open) => Paging::OpenCode(open.pager()),
        })
    }

    /// The messages appended since the last read.
    pub fn poll(&mut self) -> Result<Tail, SessionError> {
        match &mut self.0 {
            Open::File(open) => open.poll(),
            Open::OpenCode(_, open) => open.poll(),
        }
    }
}

impl FileTranscript {
    fn open(home: &Path, pane_id: &str, source: Source) -> Result<(Self, Page), SessionError> {
        let path = SessionLocator::new(home).locate(
            pane_id,
            source.agent,
            Some(&source.identity),
            source.cwd.as_deref(),
        )?;
        let proof = NativeProof::new(home, &source, &path)?;
        let length = file_length(&path)?;
        let (page, end, cursor) = if source.agent == Agent::Cursor {
            let native = cursor_page(
                proof.as_ref().ok_or(SessionError::SessionFileMissing)?,
                &path,
                None,
            )?;
            let end = native.tail.offset();
            let page = Page {
                messages: messages_from(native.parsed),
                before: native.before,
            };
            (page, end, Some(native.tail))
        } else {
            let (page, end) = page_before(&path, source.agent, None)?;
            (page, end, None)
        };
        let proof = proof.map(|proof| proof.require_same(&path)).transpose()?;
        Ok((
            Self {
                source,
                path,
                end,
                length,
                proof,
                newest: page.messages.last().cloned(),
                cursor,
            },
            page,
        ))
    }

    fn pager(&self) -> FilePager {
        FilePager {
            path: self.path.clone(),
            agent: self.source.agent,
            proof: self.proof.clone(),
        }
    }

    fn poll(&mut self) -> Result<Tail, SessionError> {
        let proof = self
            .proof
            .as_ref()
            .map(|proof| proof.current(&self.path))
            .transpose()?;
        if self.source.agent == Agent::Cursor {
            return self.poll_cursor(proof.ok_or(SessionError::SessionFileMissing)?);
        }
        if let (Some(before), Some(after)) = (&self.proof, &proof)
            && (before.confirmed.incarnation != after.confirmed.incarnation
                || after.confirmed.bytes < before.confirmed.bytes
                || (before.confirmed.bytes == after.confirmed.bytes && before.stamp != after.stamp))
        {
            return Ok(Tail::Reset);
        }
        let length = file_length(&self.path)?;
        if length == self.length {
            if let Some(proof) = proof {
                self.proof = Some(proof.require_same(&self.path)?);
            }
            return Ok(Tail::Messages(Vec::new()));
        }
        if length < self.end {
            return Ok(Tail::Reset);
        }
        // A small append is read whole and no further back: the newest line
        // ends at or before `length`, so this window reaches `self.end`.
        let window = (length - self.end).clamp(1, WINDOW_BYTES);
        let mut messages = Vec::new();
        let mut end = None;
        let mut newest_end = None;
        let mut read = 0;
        loop {
            let chunk = read_page_before(&self.path, end, window)?;
            newest_end.get_or_insert(chunk.end_offset);
            read += chunk.end_offset - chunk.start_offset;
            let mut found = messages_in(self.source.agent, &chunk.contents, chunk.start_offset);
            found.retain(|message| message.id >= self.end);
            found.append(&mut messages);
            messages = found;
            if chunk.start_offset <= self.end {
                break;
            }
            if read > PAGE_BUDGET_BYTES {
                return Ok(Tail::Reset);
            }
            end = Some(chunk.start_offset);
        }
        let messages = join(messages);
        // A record that continues the message the phone already holds
        // changes that message: read the newest page anew.
        if let (Some(newest), Some(first)) = (&self.newest, messages.first())
            && newest.continued_by(first)
        {
            return Ok(Tail::Reset);
        }
        if let Some(proof) = proof {
            self.proof = Some(proof.require_same(&self.path)?);
        }
        self.length = length;
        self.end = newest_end.unwrap_or(self.end);
        if let Some(last) = messages.last() {
            self.newest = Some(last.clone());
        }
        Ok(Tail::Messages(messages))
    }

    fn poll_cursor(&mut self, proof: NativeProof) -> Result<Tail, SessionError> {
        if self.proof.as_ref().is_none_or(|before| {
            before.confirmed.incarnation != proof.confirmed.incarnation
                || proof.confirmed.bytes < before.confirmed.bytes
        }) {
            return Ok(Tail::Reset);
        }
        let native_id = proof
            .confirmed
            .native_session_id
            .as_deref()
            .ok_or(SessionError::SessionFileMissing)?;
        let cwd = proof
            .source
            .cwd
            .as_deref()
            .ok_or(SessionError::SessionFileMissing)?;
        let mut checkpoint = self
            .cursor
            .clone()
            .ok_or_else(|| SessionError::Checkpoint("cursor_checkpoint_missing".into()))?;
        let mut read_bytes = 0u64;
        let mut messages = Vec::new();
        for _ in 0..hide_session::cursor::ARCHIVE_PAGE_LIMIT {
            let read = match hide_session::cursor::read(
                &proof.home,
                &self.path,
                native_id,
                cwd,
                Some(checkpoint),
                PAGE_BUDGET_BYTES.saturating_sub(read_bytes),
            ) {
                Err(SessionError::Capacity {
                    resource: "cursor_read_bytes",
                    ..
                }) if read_bytes > 0 => return Ok(Tail::Reset),
                other => other?,
            };
            if read.parsed.rescan_reason.is_some() {
                return Ok(Tail::Reset);
            }
            read_bytes = read_bytes.saturating_add(read.read_bytes);
            messages.extend(messages_from(read.parsed));
            checkpoint = read.checkpoint;
            if !read.has_more {
                self.proof = Some(proof.require_same(&self.path)?);
                self.end = checkpoint.offset();
                self.cursor = Some(checkpoint);
                if let Some(last) = messages.last() {
                    self.newest = Some(last.clone());
                }
                return Ok(Tail::Messages(messages));
            }
            if read_bytes >= PAGE_BUDGET_BYTES {
                return Ok(Tail::Reset);
            }
        }
        Ok(Tail::Reset)
    }
}

/// Reads older pages of one conversation.
#[derive(Clone, Debug)]
pub struct Pager(Paging);

#[derive(Clone, Debug)]
enum Paging {
    File(FilePager),
    OpenCode(opencode::Pager),
}

impl Pager {
    pub fn before(&self, cursor: u64) -> Result<Page, SessionError> {
        match &self.0 {
            Paging::File(pager) => pager.before(cursor),
            Paging::OpenCode(pager) => pager.before(cursor),
        }
    }
}

/// Reads older pages of one transcript file.
#[derive(Clone, Debug)]
struct FilePager {
    path: PathBuf,
    agent: Agent,
    proof: Option<NativeProof>,
}

impl FilePager {
    fn before(&self, cursor: u64) -> Result<Page, SessionError> {
        let proof = self
            .proof
            .as_ref()
            .map(|proof| proof.require_same(&self.path))
            .transpose()?;
        let page = if self.agent == Agent::Cursor {
            let native = cursor_page(
                proof.as_ref().ok_or(SessionError::SessionFileMissing)?,
                &self.path,
                Some(cursor),
            )?;
            Page {
                messages: messages_from(native.parsed),
                before: native.before,
            }
        } else {
            page_before(&self.path, self.agent, Some(cursor))?.0
        };
        if let Some(proof) = proof {
            proof.require_same(&self.path)?;
        }
        Ok(page)
    }
}

fn cursor_page(
    proof: &NativeProof,
    path: &Path,
    before: Option<u64>,
) -> Result<hide_session::cursor::Page, SessionError> {
    let native_id = proof
        .confirmed
        .native_session_id
        .as_deref()
        .ok_or(SessionError::SessionFileMissing)?;
    let cwd = proof
        .source
        .cwd
        .as_deref()
        .ok_or(SessionError::SessionFileMissing)?;
    hide_session::cursor::page_before(
        &proof.home,
        path,
        native_id,
        cwd,
        before,
        hide_session::cursor::PageLimits::default(),
    )
}

fn file_length(path: &Path) -> Result<u64, SessionError> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|_| SessionError::SessionFileMissing)
}

/// The newest page before `end` (the file's end when `None`), and where it ends.
fn page_before(path: &Path, agent: Agent, end: Option<u64>) -> Result<(Page, u64), SessionError> {
    let mut messages: Vec<Message> = Vec::new();
    let mut end = end;
    let mut newest_end = None;
    let mut read = 0;
    let before = loop {
        let chunk = read_page_before(path, end, WINDOW_BYTES)?;
        newest_end.get_or_insert(chunk.end_offset);
        read += chunk.end_offset - chunk.start_offset;
        let mut found = messages_in(agent, &chunk.contents, chunk.start_offset);
        found.append(&mut messages);
        messages = join(found);
        if let Some(keep) = page_start(&messages) {
            messages.drain(..keep);
            break messages.first().map(|message| message.id);
        }
        if chunk.start_offset == 0 {
            break None;
        }
        if read >= PAGE_BUDGET_BYTES {
            break Some(chunk.start_offset);
        }
        end = Some(chunk.start_offset);
    };
    let end = newest_end.unwrap_or(0);
    Ok((Page { messages, before }, end))
}

/// Where a full page starts in `messages` (oldest first), or `None` while
/// they do not fill one: `PAGE_MESSAGES` of them, or `PAGE_TEXT_BYTES` of
/// text past the newest.
fn page_start(messages: &[Message]) -> Option<usize> {
    let mut text = 0;
    for (taken, message) in messages.iter().rev().enumerate() {
        if taken == PAGE_MESSAGES || (taken > 0 && text + message.text.len() > PAGE_TEXT_BYTES) {
            return Some(messages.len() - taken);
        }
        text += message.text.len();
    }
    None
}

/// The messages a phone shows from one chunk of complete lines.
fn messages_in(agent: Agent, contents: &str, start_offset: u64) -> Vec<Message> {
    let parsed = parse_events_at(agent, contents, start_offset);
    messages_from(parsed)
}

fn messages_from(parsed: hide_session::ParsedSession) -> Vec<Message> {
    parsed
        .events
        .into_iter()
        .zip(parsed.event_offsets)
        .filter_map(|(event, id)| {
            let who = match event.kind {
                EventKind::Human => "you",
                EventKind::Assistant => "agent",
                EventKind::Interrupted => "stopped",
                EventKind::Injected => return None,
            };
            message(
                id,
                who,
                &event.text,
                event.at_unix_ms,
                event.part().map(str::to_owned),
            )
        })
        .collect()
}

/// The message a phone shows, cut at `MESSAGE_CHARS`; `None` for one with
/// no text, such as a message of images alone. `part` names the native
/// message a split record belongs to.
fn message(
    id: u64,
    who: &'static str,
    text: &str,
    at_ms: u64,
    part: Option<String>,
) -> Option<Message> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let truncated = text.chars().nth(MESSAGE_CHARS).is_some();
    let text = if truncated {
        text.chars().take(MESSAGE_CHARS).collect()
    } else {
        text.to_owned()
    };
    Some(Message {
        id,
        who,
        text,
        truncated,
        at_ms,
        part,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn cursor_transcript() -> (tempfile::TempDir, PathBuf, Source, serde_json::Value) {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("checkout");
        std::fs::create_dir(&cwd).unwrap();
        let id = "a1b2c3d4-0000-4000-8000-000000000001";
        let bucket = format!("{:x}", md5::compute(cwd.to_str().unwrap().as_bytes()));
        let path = home
            .path()
            .join(hide_session::cursor::SESSIONS)
            .join(bucket)
            .join(id)
            .join("store.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let graph: serde_json::Value = serde_json::from_str(include_str!(
            "../../../hide-session/tests/fixtures/cursor-2026.10.01/graph.json"
        ))
        .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=1; CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT); CREATE TABLE blobs(id TEXT PRIMARY KEY, data BLOB);").unwrap();
        for (id, value) in graph["blobs"].as_object().unwrap() {
            db.execute(
                "INSERT INTO blobs VALUES (?1,?2)",
                rusqlite::params![id, hex::decode(value.as_str().unwrap()).unwrap()],
            )
            .unwrap();
        }
        drop(db);
        std::fs::write(path.with_file_name("meta.json"), serde_json::json!({"schemaVersion":1,"createdAtMs":1_790_989_200_000_u64,"hasConversation":true,"cwd":cwd}).to_string()).unwrap();
        cursor_root(&path, &graph, "first");
        let source = Source {
            agent: Agent::Cursor,
            identity: SessionIdentity::id(id),
            cwd: Some(cwd.to_str().unwrap().into()),
        };
        (home, path, source, graph)
    }

    fn cursor_root(path: &Path, graph: &serde_json::Value, variant: &str) {
        let metadata = serde_json::json!({"agentId":"a1b2c3d4-0000-4000-8000-000000000001","latestRootBlobId":graph["roots"][variant],"createdAt":1_790_989_200_000_u64});
        rusqlite::Connection::open(path)
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO meta VALUES ('0',?1)",
                [hex::encode(metadata.to_string().as_bytes())],
            )
            .unwrap();
    }

    #[test]
    fn cursor_phone_uses_graph_ordinals_append_and_reset_without_reading_sqlite_as_lines() {
        let (home, path, source, graph) = cursor_transcript();
        let files = || {
            std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.file_name(), std::fs::read(entry.path()).unwrap())
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let before = files();
        let (mut transcript, page) = Transcript::open(home.path(), "fixture", source).unwrap();
        assert_eq!(
            texts(&page.messages),
            ["Read this conversation", "Native answer"]
        );
        assert_eq!(
            page.messages
                .iter()
                .map(|message| (message.id, message.at_ms))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 1_790_989_260_000)]
        );
        assert_eq!(page.before, None);
        assert_eq!(transcript.poll().unwrap(), Tail::Messages(Vec::new()));
        assert_eq!(files(), before);
        let old_pager = transcript.pager();
        cursor_root(&path, &graph, "append");
        assert!(old_pager.before(1).is_err());
        let Tail::Messages(appended) = transcript.poll().unwrap() else {
            panic!("append reset")
        };
        assert_eq!(texts(&appended), ["Later answer"]);
        assert_eq!(appended[0].id, 3);
        cursor_root(&path, &graph, "rewrite");
        assert_eq!(transcript.poll().unwrap(), Tail::Reset);
        let (mut restored, _) =
            Transcript::open(home.path(), "fixture", transcript.source().clone()).unwrap();
        std::fs::remove_file(path.with_file_name("meta.json")).unwrap();
        assert!(restored.poll().is_err());
    }

    fn human(text: &str, second: u32) -> String {
        serde_json::json!({
            "type": "user", "userType": "external", "promptId": "p",
            "timestamp": format!("2026-09-29T10:00:{second:02}Z"),
            "message": {"role": "user", "content": text},
        })
        .to_string()
    }

    fn assistant(text: &str, second: u32) -> String {
        serde_json::json!({
            "type": "assistant", "timestamp": format!("2026-09-29T10:00:{second:02}Z"),
            "message": {"role": "assistant", "content": [{"type": "text", "text": text}]},
        })
        .to_string()
    }

    fn tool_output(size: usize) -> String {
        serde_json::json!({
            "type": "user", "timestamp": "2026-09-29T10:00:00Z",
            "message": {"role": "user", "content": [{"type": "tool_result", "content": "x".repeat(size)}]},
        })
        .to_string()
    }

    fn transcript(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        std::fs::write(
            &path,
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
        (directory, path)
    }

    fn texts(page: &[Message]) -> Vec<&str> {
        page.iter().map(|message| message.text.as_str()).collect()
    }

    #[test]
    fn a_page_holds_the_newest_messages_and_the_cursor_walks_back_to_the_start() {
        let lines: Vec<String> = (0..70)
            .map(|index| assistant(&format!("m{index}"), index % 60))
            .collect();
        let (_directory, path) = transcript(&lines);
        let (newest, _) = page_before(&path, Agent::Claude, None).unwrap();
        assert_eq!(newest.messages.len(), PAGE_MESSAGES);
        assert_eq!(newest.messages.last().unwrap().text, "m69");
        let older = FilePager {
            path: path.clone(),
            agent: Agent::Claude,
            proof: None,
        }
        .before(newest.before.unwrap())
        .unwrap();
        assert_eq!(texts(&older.messages).first(), Some(&"m10"));
        assert_eq!(texts(&older.messages).last(), Some(&"m39"));
        let oldest = FilePager {
            path,
            agent: Agent::Claude,
            proof: None,
        }
        .before(older.before.unwrap())
        .unwrap();
        assert_eq!(
            texts(&oldest.messages),
            (0..10).map(|index| format!("m{index}")).collect::<Vec<_>>()
        );
        assert_eq!(oldest.before, None);
    }

    #[test]
    fn tool_output_and_injected_context_never_reach_the_phone_and_huge_lines_are_passed_over() {
        let lines = [
            human("테스트 다시 돌려줘", 1),
            tool_output(3 * 1024 * 1024),
            human("<system-reminder>internal</system-reminder>", 2),
            assistant("다 통과했어요", 3),
        ];
        let (_directory, path) = transcript(&lines);
        let (page, _) = page_before(&path, Agent::Claude, None).unwrap();
        assert_eq!(
            texts(&page.messages),
            ["테스트 다시 돌려줘", "다 통과했어요"]
        );
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.who)
                .collect::<Vec<_>>(),
            ["you", "agent"]
        );
        assert_eq!(page.before, None);
    }

    #[test]
    fn a_poll_returns_only_appended_messages_even_past_a_huge_line() {
        let (_directory, path) = transcript(&[human("첫 질문", 1), assistant("첫 답", 2)]);
        let length = file_length(&path).unwrap();
        let (_, end) = page_before(&path, Agent::Claude, None).unwrap();
        let mut open = FileTranscript {
            source: Source {
                agent: Agent::Claude,
                identity: SessionIdentity::id("s"),
                cwd: None,
            },
            path: path.clone(),
            end,
            length,
            proof: None,
            newest: None,
            cursor: None,
        };
        assert_eq!(open.poll().unwrap(), Tail::Messages(Vec::new()));
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        // A line still being written ends the append.
        write!(
            file,
            "{}\n{}\n{{\"type\":\"assi",
            tool_output(700 * 1024),
            assistant("두 번째 답", 3)
        )
        .unwrap();
        let Tail::Messages(appended) = open.poll().unwrap() else {
            panic!("reset")
        };
        assert_eq!(texts(&appended), ["두 번째 답"]);
        writeln!(file, "stant\"}}").unwrap();
        writeln!(file, "{}", human("다음", 4)).unwrap();
        let Tail::Messages(appended) = open.poll().unwrap() else {
            panic!("reset")
        };
        assert_eq!(texts(&appended), ["다음"]);
    }

    #[test]
    fn a_long_message_is_cut_and_marked() {
        let (_directory, path) = transcript(&[assistant(&"가".repeat(MESSAGE_CHARS + 5), 1)]);
        let (page, _) = page_before(&path, Agent::Claude, None).unwrap();
        assert!(page.messages[0].truncated);
        assert_eq!(page.messages[0].text.chars().count(), MESSAGE_CHARS);
    }

    fn native_transcript(agent: Agent) -> (tempfile::TempDir, PathBuf, Source) {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("checkout");
        std::fs::create_dir(&cwd).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let (root, bucket) = match agent {
            Agent::Pi => (
                ".pi/agent/sessions",
                format!(
                    "--{}--",
                    cwd.to_string_lossy()
                        .trim_start_matches(['/', '\\'])
                        .replace(['/', '\\', ':'], "-")
                ),
            ),
            Agent::Omp => {
                let temporary = std::env::temp_dir().canonicalize().unwrap();
                let relative = cwd.strip_prefix(temporary).unwrap();
                (
                    ".omp/agent/sessions",
                    format!(
                        "-tmp-{}",
                        relative.to_string_lossy().replace(['/', '\\', ':'], "-")
                    ),
                )
            }
            _ => unreachable!("native-file fixture"),
        };
        let path = home
            .path()
            .join(root)
            .join(bucket)
            .join("timestamp_native-phone.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"session","version":3,"id":"native-phone","cwd":cwd,"timestamp":"2026-10-03T01:00:00Z"})).unwrap();
        for index in 0..70 {
            writeln!(file, "{}", serde_json::json!({"type":"message","id":format!("entry-{index}"),"parentId":null,"timestamp":"2026-10-03T01:00:00Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":format!("답변 {index}")}],"stopReason":"stop"}})).unwrap();
        }
        let source = Source {
            agent,
            identity: SessionIdentity::path(&path),
            cwd: Some(cwd.display().to_string()),
        };
        (home, path, source)
    }

    /// Grok's session folder, its id-owning summary and `records` as its
    /// conversation, each `(kind, prompt or turn, text)`.
    fn grok_transcript(records: &[(&str, &str, &str)]) -> (tempfile::TempDir, PathBuf, Source) {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("checkout");
        std::fs::create_dir(&cwd).unwrap();
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let group: String = cwd
            .to_str()
            .unwrap()
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect();
        let id = "0199b000-0000-7000-8000-0000000000aa";
        let folder = home.path().join(".grok/sessions").join(group).join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("summary.json"),
            serde_json::json!({"info":{"id":id,"cwd":cwd},"session_summary":"","created_at":"2026-10-03T01:00:00Z",
                "updated_at":"2026-10-03T01:00:00Z","num_messages":0,"current_model_id":"grok-build"})
            .to_string(),
        )
        .unwrap();
        let path = folder.join("updates.jsonl");
        std::fs::File::create(&path).unwrap();
        grok_append(&path, records);
        let source = Source {
            agent: Agent::Grok,
            identity: SessionIdentity::id(id),
            cwd: Some(cwd.display().to_string()),
        };
        (home, path, source)
    }

    fn grok_append(path: &Path, records: &[(&str, &str, &str)]) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        for (kind, part, text) in records {
            let (update, meta) = match *kind {
                "user" => (
                    serde_json::json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":text},
                        "_meta":{"promptIndex":part.parse::<u64>().unwrap()}}),
                    serde_json::json!({"agentTimestampMs":1_790_989_200_000_u64}),
                ),
                "agent" => (
                    serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}),
                    serde_json::json!({"agentTimestampMs":1_790_989_200_000_u64,"promptId":part}),
                ),
                _ => (
                    serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":text,"status":"completed",
                        "content":[{"type":"content","content":{"type":"text","text":"hidden tool output"}}]}),
                    serde_json::json!({"agentTimestampMs":1_790_989_200_000_u64,"promptId":part}),
                ),
            };
            writeln!(
                file,
                "{}",
                serde_json::json!({"timestamp":1_790_989_200_u64,"method":"session/update",
                "params":{"sessionId":"x","update":update,"_meta":meta}})
            )
            .unwrap();
        }
    }

    #[test]
    fn grok_phone_shows_a_turns_answer_runs_as_one_message_and_resets_on_a_continuation() {
        let mut records = Vec::new();
        let prompts: Vec<String> = (0..40).map(|index| index.to_string()).collect();
        let turns: Vec<String> = (0..40).map(|index| format!("p-{index}")).collect();
        let answers: Vec<String> = (0..40).map(|index| format!("답변 {index}")).collect();
        for index in 0..40 {
            records.push(("user", prompts[index].as_str(), "요청"));
            records.push(("agent", turns[index].as_str(), answers[index].as_str()));
            records.push(("tool", turns[index].as_str(), "call"));
            records.push(("agent", turns[index].as_str(), "이어서"));
        }
        let (home, path, source) = grok_transcript(&records);
        let (mut transcript, page) = Transcript::open(home.path(), "pane", source).unwrap();
        assert_eq!(page.messages.len(), PAGE_MESSAGES);
        assert_eq!(texts(&page.messages).last(), Some(&"답변 39\n\n이어서"));
        assert_eq!(texts(&page.messages)[page.messages.len() - 2], "요청");
        let older = transcript.pager().before(page.before.unwrap()).unwrap();
        assert!(texts(&older.messages).contains(&"답변 10\n\n이어서"));

        grok_append(
            &path,
            &[("user", "40", "새 요청"), ("agent", "p-40", "새 답")],
        );
        let Tail::Messages(appended) = transcript.poll().unwrap() else {
            panic!("a new turn is appended");
        };
        assert_eq!(texts(&appended), ["새 요청", "새 답"]);
        grok_append(
            &path,
            &[("tool", "p-40", "call-2"), ("agent", "p-40", "마저")],
        );
        assert_eq!(
            transcript.poll().unwrap(),
            Tail::Reset,
            "the held answer grew, so the phone takes the page anew"
        );
        let (_, page) = Transcript::open(home.path(), "pane", transcript.source().clone()).unwrap();
        assert_eq!(texts(&page.messages).last(), Some(&"새 답\n\n마저"));
    }

    #[test]
    fn native_phone_pages_native_message_units_and_only_appends_new_messages() {
        for agent in [Agent::Pi, Agent::Omp] {
            native_phone_pages(agent);
        }
    }

    fn native_phone_pages(agent: Agent) {
        let (home, path, source) = native_transcript(agent);
        let (mut transcript, page) = Transcript::open(home.path(), "pane", source).unwrap();
        assert_eq!(texts(&page.messages).first(), Some(&"답변 40"));
        assert_eq!(texts(&page.messages).last(), Some(&"답변 69"));
        let older = transcript.pager().before(page.before.unwrap()).unwrap();
        assert_eq!(texts(&older.messages).first(), Some(&"답변 10"));
        assert_eq!(transcript.poll().unwrap(), Tail::Messages(vec![]));
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        for (role, content) in [
            ("user", "다음 요청"),
            ("toolResult", "hidden tool output"),
            ("custom", "hidden context"),
            ("assistant", "다음 답변"),
        ] {
            writeln!(file, "{}", serde_json::json!({"type":"message","timestamp":"2026-10-03T01:01:00Z","message":{"role":role,"content":content}})).unwrap();
        }
        let Tail::Messages(messages) = transcript.poll().unwrap() else {
            panic!("unexpected reset");
        };
        assert_eq!(texts(&messages), ["다음 요청", "다음 답변"]);
    }

    #[test]
    fn native_idle_poll_and_cloned_pager_reject_rebound_native_owner() {
        for agent in [Agent::Pi, Agent::Omp] {
            native_rebound_phone(agent);
        }
    }

    fn native_rebound_phone(agent: Agent) {
        let (home, path, source) = native_transcript(agent);
        let (mut transcript, page) = Transcript::open(home.path(), "pane", source).unwrap();
        let pager = transcript.pager();
        let original = std::fs::read_to_string(&path).unwrap();
        let changed = original.replace("native-phone", "other-ownerx");
        assert_eq!(original.len(), changed.len());
        std::fs::write(path, changed).unwrap();
        assert!(
            transcript.poll().is_err(),
            "same length idle reads prove the original owner"
        );
        assert!(pager.before(page.before.unwrap()).is_err());
    }

    #[test]
    fn native_same_owner_same_length_edit_resets_phone_and_invalidates_old_pager() {
        for agent in [Agent::Pi, Agent::Omp] {
            native_rewritten_phone(agent);
        }
    }

    fn native_rewritten_phone(agent: Agent) {
        let (home, path, source) = native_transcript(agent);
        let (mut transcript, page) = Transcript::open(home.path(), "pane", source).unwrap();
        let pager = transcript.pager();
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace("답변 69", "수정 69")).unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().len(),
            original.len() as u64
        );
        assert_eq!(transcript.poll().unwrap(), Tail::Reset);
        assert!(pager.before(page.before.unwrap()).is_err());
    }
}
