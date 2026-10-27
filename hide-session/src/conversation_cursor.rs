use crate::turns::native::TOOL_MARK_LIMIT;
use crate::turns::wake::{Record, starts_work, stops_work};
use crate::turns::{
    NATIVE_ID_LIMIT_BYTES, StopOutcome, ToolTurnMark, TurnMark, TurnMode, WakeMark,
};
use crate::{
    Agent, AppendedBytes, ParsedSession, Result, SESSION_LINE_LIMIT_BYTES, SessionCursor,
    SessionError, SkipReason, parse_events_followed,
};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Search-only durable progress, including a skipped oversized tool record.
/// No transcript bytes or Memory cursor state are retained.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConversationCheckpoint {
    cursor: crate::CursorCheckpoint,
    discarded_bytes: u64,
    has_more: bool,
    /// Boxed: a scan exists only inside an oversized record, and every
    /// label request carries a checkpoint.
    #[serde(default)]
    classifier: Option<Box<LargeRecord>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native: Option<Box<crate::cursor::Checkpoint>>,
}

impl ConversationCheckpoint {
    /// A node-proven JSONL turn boundary. Keep the current file identity so
    /// replacement cannot turn the saved byte offset into another turn.
    pub(crate) fn for_turn(path: &Path, offset: u64) -> Result<Self> {
        let mut file = crate::open_session_file(path)
            .map_err(|error| SessionError::io("open", path, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| SessionError::io("metadata", path, error))?;
        if offset >= metadata.len() {
            return Err(SessionError::Checkpoint("turn_offset_invalid".into()));
        }
        if offset > 0 {
            file.seek(SeekFrom::Start(offset - 1))
                .map_err(|error| SessionError::io("seek", path, error))?;
            let mut before = [0];
            file.read_exact(&mut before)
                .map_err(|error| SessionError::io("read", path, error))?;
            if before[0] != b'\n' {
                return Err(SessionError::Checkpoint("turn_offset_invalid".into()));
            }
        }
        Ok(crate::CursorCheckpoint {
            offset,
            identity: Some(
                crate::FileIdentity::from_file(&file)
                    .map_err(|error| SessionError::io("identity", path, error))?,
            ),
            pending: Vec::new(),
        }
        .into())
    }
    /// A position that is a count rather than a file offset: OpenCode's
    /// messages read so far, with its witness: the session's creation time
    /// and a digest of the last message read, so a session rewound and grown
    /// back to the same count is read again rather than continued.
    pub(crate) fn at_message(offset: u64, created: u64, last: Option<u64>) -> Self {
        Self {
            cursor: crate::CursorCheckpoint {
                offset,
                identity: last.map(|last| crate::FileIdentity {
                    first: created,
                    second: last,
                    high: 0,
                }),
                pending: Vec::new(),
            },
            ..Self::default()
        }
    }

    /// The session creation time and last-message digest
    /// [`Self::at_message`] recorded; `None` from an older checkpoint.
    pub(crate) fn message_witness(&self) -> Option<(u64, u64)> {
        self.cursor
            .identity
            .map(|identity| (identity.first, identity.second))
    }

    pub fn offset(&self) -> u64 {
        self.native
            .as_ref()
            .map(|native| native.offset())
            .unwrap_or(self.cursor.offset)
    }
    pub fn has_more(&self) -> bool {
        self.has_more
    }
}

/// A line cursor's position, for a reader that saved one before it read
/// through this cursor (Project Memory's): it resumes at the same offset of
/// the same file.
impl From<crate::CursorCheckpoint> for ConversationCheckpoint {
    fn from(cursor: crate::CursorCheckpoint) -> Self {
        Self {
            cursor,
            ..Self::default()
        }
    }
}

/// One provider-neutral page on an authenticated reader boundary.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConversationRead {
    pub events: Vec<crate::ConversationEvent>,
    pub event_offsets: Vec<u64>,
    pub checkpoint: ConversationCheckpoint,
    pub has_more: bool,
    pub rescanned: bool,
    pub read_bytes: u64,
    /// The records this read passed over, by [`crate::SkipReason`]: a
    /// record past the line cap is read past without being held.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub skipped_reasons: std::collections::BTreeMap<String, usize>,
}

pub fn read_conversation(
    home: &Path,
    agent: Agent,
    path: &Path,
    scope: &crate::SessionReadScope,
    saved: Option<ConversationCheckpoint>,
) -> Result<ConversationRead> {
    let mut cursor = saved.map(ConversationCursor::restore).unwrap_or_default();
    let parsed = cursor.read_confirmed(home, agent, path, scope)?;
    Ok(ConversationRead {
        events: parsed.events,
        event_offsets: parsed.event_offsets,
        checkpoint: cursor.checkpoint(),
        has_more: cursor.has_more(),
        rescanned: parsed.rescan_reason.is_some(),
        read_bytes: cursor.read_bytes(),
        skipped_reasons: parsed
            .skipped_reasons
            .iter()
            .map(|(reason, count)| (reason.as_str().to_owned(), *count))
            .collect(),
    })
}

/// Incrementally reads conversation events without retaining unrelated records.
#[derive(Debug, Default)]
pub struct ConversationCursor {
    cursor: SessionCursor,
    discarded_bytes: u64,
    has_more: bool,
    classifier: Option<Box<LargeRecord>>,
    read_bytes: u64,
    native: Option<Box<crate::cursor::Checkpoint>>,
    human_anchors: Vec<(u64, crate::cursor::Checkpoint)>,
}

impl ConversationCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Search owns a separate durable cursor; no Memory checkpoint is shared.
    pub fn checkpoint(&self) -> ConversationCheckpoint {
        ConversationCheckpoint {
            cursor: self.cursor.checkpoint(),
            discarded_bytes: self.discarded_bytes,
            has_more: self.has_more,
            classifier: self.classifier.clone(),
            native: self.native.clone(),
        }
    }

    /// A checkpoint of this file that resumes at `offset`, an earlier record
    /// boundary this cursor already read past (an event offset). Reading
    /// from it again yields the records from there on; nothing beyond the
    /// file identity is carried, so no transcript bytes are kept.
    pub fn checkpoint_at(&self, offset: u64) -> Result<ConversationCheckpoint> {
        if self.native.is_some() {
            let native = self
                .human_anchors
                .iter()
                .find(|(at, _)| *at == offset)
                .map(|(_, checkpoint)| checkpoint.clone())
                .ok_or_else(|| SessionError::Checkpoint("cursor_anchor_unavailable".to_owned()))?;
            return Ok(ConversationCheckpoint {
                native: Some(Box::new(native)),
                ..ConversationCheckpoint::default()
            });
        }
        let mut cursor = self.cursor.checkpoint();
        cursor.offset = offset.min(cursor.offset);
        Ok(ConversationCheckpoint {
            cursor,
            discarded_bytes: 0,
            has_more: false,
            classifier: None,
            native: None,
        })
    }

    pub fn restore(checkpoint: ConversationCheckpoint) -> Self {
        let mut cursor = SessionCursor::restore(checkpoint.cursor);
        let (discarded_bytes, classifier) = match checkpoint.classifier {
            // Older scans lack native ids, wake metadata or context boundaries.
            // They cannot certify the discarded record. Reread that record
            // from its start rather than refuse it on every later read.
            Some(scan) if !scan.native_ids || !scan.wake_aware || !scan.context_aware => {
                cursor.offset = cursor.offset.saturating_sub(checkpoint.discarded_bytes);
                (0, None)
            }
            classifier => (checkpoint.discarded_bytes, classifier),
        };
        Self {
            cursor,
            discarded_bytes,
            has_more: false,
            classifier,
            read_bytes: 0,
            native: checkpoint.native,
            human_anchors: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.cursor.reset();
        self.discarded_bytes = 0;
        self.has_more = false;
        self.classifier = None;
        self.native = None;
        self.human_anchors.clear();
    }

    /// More bytes from the last observed file size remain to be read.
    /// A torn final record at EOF waits for an append instead of spinning.
    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub(crate) fn read_bytes(&self) -> u64 {
        self.read_bytes
    }

    pub fn read(&mut self, agent: Agent, path: &Path) -> Result<ParsedSession> {
        self.read_with_budget(agent, path, crate::SESSION_INCREMENT_READ_LIMIT_BYTES)
    }

    /// Native graph stores need the node-owned root and current read scope.
    pub fn read_confirmed(
        &mut self,
        home: &Path,
        agent: Agent,
        path: &Path,
        scope: &crate::SessionReadScope,
    ) -> Result<ParsedSession> {
        if agent != Agent::Cursor {
            return self.read(agent, path);
        }
        let result = crate::cursor::read(
            home,
            path,
            &scope.id,
            &scope.cwd,
            self.native.as_deref().cloned(),
            crate::SESSION_INCREMENT_READ_LIMIT_BYTES,
        )?;
        self.native = Some(Box::new(result.checkpoint));
        self.has_more = result.has_more;
        self.read_bytes = result.read_bytes;
        self.human_anchors = result.human_anchors;
        Ok(result.parsed)
    }

    /// A shared poll gives each file only its remaining byte allowance.
    pub(crate) fn read_with_budget(
        &mut self,
        agent: Agent,
        path: &Path,
        budget: u64,
    ) -> Result<ParsedSession> {
        self.read_bytes = 0;
        let file = crate::open_session_file(path)
            .map_err(|error| SessionError::io("open", path, error))?;
        self.read_file_with_budget(agent, path, &file, budget)
    }

    fn read_file_with_budget(
        &mut self,
        agent: Agent,
        path: &Path,
        file: &File,
        budget: u64,
    ) -> Result<ParsedSession> {
        if !agent.is_jsonl() {
            return Err(SessionError::UnsupportedSessionKind);
        }
        self.has_more = false;
        self.read_bytes = 0;
        let mut plan_hold = None;
        let title = match agent {
            Agent::Omp => Some(read_current_title(
                path,
                file,
                budget,
                &mut self.read_bytes,
            )?),
            Agent::Grok => {
                let (title, hold) = read_grok_state(path, budget, &mut self.read_bytes)?;
                plan_hold = hold;
                Some(title)
            }
            _ => None,
        };
        let remaining = budget.saturating_sub(self.read_bytes);
        let mut appended_read_bytes = 0;
        let appended_result = read_appended_file(
            &mut self.cursor,
            path,
            file,
            remaining,
            &mut appended_read_bytes,
        );
        self.read_bytes += appended_read_bytes;
        let AppendedBytes {
            contents: appended,
            start_offset,
            identity,
            rescan_reason,
            has_more,
        } = appended_result?;
        if rescan_reason.is_some() {
            self.discarded_bytes = 0;
            self.classifier = None;
        }
        let mut offset = start_offset;
        let mut pending = self.cursor.pending.clone();
        let mut discarded_bytes = self.discarded_bytes;
        let mut classifier = self.classifier.clone();
        let mut parsed = ParsedSession {
            rescan_reason,
            ..ParsedSession::default()
        };
        let mut found = crate::links::LinkAccumulator::default();
        let mut consumed = 0;
        let window_start = start_offset.saturating_sub(self.cursor.pending.len() as u64);
        for fragment in records(&appended) {
            consumed += fragment.len();
            let line_start = offset.saturating_sub(pending.len() as u64 + discarded_bytes);
            offset += fragment.len() as u64;
            let complete = fragment.last() == Some(&b'\n');
            if discarded_bytes > 0 {
                discarded_bytes += fragment.len() as u64;
                if let Some(classifier) = classifier.as_mut() {
                    classifier.feed(fragment);
                }
            } else {
                let retained = fragment.len().min(SESSION_LINE_LIMIT_BYTES - pending.len());
                pending.extend_from_slice(&fragment[..retained]);
                if retained < fragment.len() {
                    // Continue a bounded structural scan across chunks. Only
                    // JSON discriminators survive a checkpoint, never bodies.
                    let mut scan = LargeRecord {
                        native_ids: true,
                        grok: (agent == Agent::Grok).then(Default::default),
                        wake_aware: true,
                        context_aware: true,
                        ..LargeRecord::default()
                    };
                    scan.feed(&pending);
                    scan.feed(&fragment[retained..]);
                    classifier = Some(Box::new(scan));
                    discarded_bytes = pending.len() as u64 + (fragment.len() - retained) as u64;
                    pending.clear();
                }
            }
            if !complete {
                continue;
            }
            if discarded_bytes > 0 {
                let scan = classifier.take();
                discarded_bytes = 0;
                if agent == Agent::Grok {
                    // The record still counts, without its bodies.
                    if let Some(line) = scan
                        .filter(|scan| !scan.invalid)
                        .and_then(|scan| scan.grok)
                        .and_then(|grok| grok.reduced())
                    {
                        parsed.skipped(SkipReason::BodyCapacity);
                        pending = line.into_bytes();
                    } else {
                        let (reason, turn) = LargeRecord::unreadable();
                        parsed.skipped(reason);
                        if let Ok(Some(mark)) = turn {
                            parsed.turn_marks.push((line_start, mark));
                        }
                        continue;
                    }
                } else {
                    let wake = LargeRecord::wake_mark(scan.as_deref(), agent);
                    if let Some(event) = scan.as_deref().and_then(|scan| scan.context_reset(agent))
                    {
                        parsed.events.push(event);
                        parsed.event_offsets.push(line_start);
                    }
                    let (reason, turn) =
                        scan.map_or_else(LargeRecord::unreadable, |scan| (*scan).discard(agent));
                    parsed.skipped(reason);
                    match turn {
                        Ok(Some(mark)) => parsed.turn_marks.push((line_start, mark)),
                        Ok(None) => {}
                        Err(reason) => parsed.skipped(reason),
                    }
                    if let Some(mark) = wake {
                        parsed.turn_marks.push((line_start, mark));
                    }
                    continue;
                }
            }
            let text = String::from_utf8_lossy(&pending);
            let mut line = parse_events_followed(
                agent,
                &text,
                line_start,
                &mut found,
                Some(&appended[consumed..]),
            );
            if line.held_command && has_more && line_start <= window_start {
                // A command that opens a full window cannot wait for a reply
                // beyond it: holding would stall the reader, so it is decided
                // on what the window shows, as a whole-file reader does.
                line = parse_events_followed(agent, &text, line_start, &mut found, None);
            }
            if line.held_command {
                // A command whose reply is not written yet: read it again
                // with the poll that holds the reply, so neither is lost.
                offset = line_start;
                pending.clear();
                break;
            }
            parsed.events.extend(line.events);
            parsed.event_offsets.extend(line.event_offsets);
            parsed.skipped_lines += line.skipped_lines;
            for (reason, count) in line.skipped_reasons {
                *parsed.skipped_reasons.entry(reason).or_default() += count;
            }
            if line.title.is_some() {
                parsed.title = line.title;
            }
            if line.custom_title.is_some() {
                parsed.custom_title = line.custom_title;
            }
            parsed.pr_sightings.extend(line.pr_sightings);
            parsed.turn_marks.extend(line.turn_marks);
            parsed.tool_steps.extend(line.tool_steps);
            pending.clear();
        }
        parsed.links = found.finish();
        parsed.coalesce();
        parsed.plan_hold = plan_hold;
        if let Some((title, custom_title)) = title {
            parsed.title = Some(title);
            parsed.custom_title = Some(custom_title);
        }
        // Commit only a successful poll: a relevant capacity failure cannot
        // silently consume a Human turn or publish a partial result.
        self.cursor.offset = offset;
        self.cursor.identity = Some(identity);
        self.cursor.pending = pending;
        self.discarded_bytes = discarded_bytes;
        self.classifier = classifier;
        self.has_more = has_more;
        Ok(parsed)
    }
}

/// `bytes` cut after each newline, as `split_inclusive` cuts them, with the
/// newlines found by `memchr`: a transcript is read a line at a time, and a
/// byte-at-a-time search was most of what reading a long one cost.
fn records(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = memchr::memchr(b'\n', rest).map_or(rest.len(), |at| at + 1);
        let (record, after) = rest.split_at(end);
        rest = after;
        Some(record)
    })
}

/// Grok's current title and plan wait live beside its conversation, each
/// replaced whole by Grok; every byte read is charged to this poll.
fn read_grok_state(
    path: &Path,
    budget: u64,
    read_bytes: &mut u64,
) -> Result<((String, String), Option<crate::turns::UserTurnContent>)> {
    let refused = |error: anyhow::Error| SessionError::Checkpoint(error.to_string());
    let summary = crate::grok::summary(path, read_bytes).map_err(refused)?;
    let hold = crate::grok::plan_hold(path, read_bytes).map_err(refused)?;
    if *read_bytes > budget.min(crate::SESSION_INCREMENT_READ_LIMIT_BYTES) {
        return Err(SessionError::Capacity {
            resource: "title_read_bytes",
            limit: budget,
        });
    }
    Ok((summary.title, hold))
}

/// Refresh the physical first line through the same descriptor without
/// changing event offsets. Every byte read, including a block's tail after
/// the newline, is charged against this poll's existing allowance.
fn read_current_title(
    path: &Path,
    file: &File,
    budget: u64,
    read_bytes: &mut u64,
) -> Result<(String, String)> {
    let mut file = file
        .try_clone()
        .map_err(|e| SessionError::io("clone", path, e))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| SessionError::io("seek", path, e))?;
    let mut prefix = Vec::new();
    loop {
        let allowance = budget
            .min(crate::SESSION_INCREMENT_READ_LIMIT_BYTES)
            .saturating_sub(*read_bytes);
        if allowance == 0 {
            return Err(SessionError::Capacity {
                resource: "title_read_bytes",
                limit: budget,
            });
        }
        let mut block = [0_u8; 256];
        let limit = block
            .len()
            .min(allowance as usize)
            .min(SESSION_LINE_LIMIT_BYTES + 1 - prefix.len());
        let n = file
            .read(&mut block[..limit])
            .map_err(|e| SessionError::io("read", path, e))?;
        *read_bytes += n as u64;
        let newline = block[..n]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| prefix.len() + index);
        prefix.extend_from_slice(&block[..n]);
        if let Some(end) = newline {
            if end + 1 > SESSION_LINE_LIMIT_BYTES {
                return Err(SessionError::Capacity {
                    resource: "line_bytes",
                    limit: SESSION_LINE_LIMIT_BYTES as u64,
                });
            }
            let value = serde_json::from_slice(&prefix[..end]).map_err(|_| {
                SessionError::Checkpoint("label_session_metadata_unconfirmed".to_owned())
            })?;
            return crate::native_file::title_snapshot(&value).ok_or_else(|| {
                SessionError::Checkpoint("label_session_metadata_unconfirmed".to_owned())
            });
        }
        if prefix.len() > SESSION_LINE_LIMIT_BYTES {
            return Err(SessionError::Capacity {
                resource: "line_bytes",
                limit: SESSION_LINE_LIMIT_BYTES as u64,
            });
        }
        if n == 0 {
            return Err(SessionError::Checkpoint(
                "label_session_metadata_unconfirmed".to_owned(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_title_prefix_spends_the_poll_budget_without_advancing_the_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.jsonl");
        let header = serde_json::json!({"type":"session","version":3,"title":"현재 제목","titleSource":"user","pad":"x".repeat(400)});
        std::fs::write(&path, format!("{header}\n")).unwrap();
        let mut cursor = ConversationCursor::new();
        let before = cursor.checkpoint();
        assert!(matches!(
            cursor.read_with_budget(Agent::Omp, &path, 256),
            Err(SessionError::Capacity {
                resource: "title_read_bytes",
                limit: 256
            })
        ));
        assert_eq!(cursor.read_bytes(), 256);
        assert_eq!(cursor.checkpoint(), before);
        let current = cursor.read(Agent::Omp, &path).unwrap();
        assert_eq!(current.custom_title.as_deref(), Some("현재 제목"));
        assert!(current.events.is_empty());
        assert!(cursor.read_bytes() <= crate::SESSION_INCREMENT_READ_LIMIT_BYTES);
        assert_eq!(
            cursor.checkpoint().offset(),
            std::fs::metadata(path).unwrap().len()
        );
    }
}

// Reads through an already-open descriptor so path replacement cannot race a
// blocking open.
fn read_appended_file(
    cursor: &mut SessionCursor,
    path: &Path,
    file: &File,
    budget: u64,
    read_bytes: &mut u64,
) -> Result<AppendedBytes> {
    let mut file = file
        .try_clone()
        .map_err(|e| SessionError::io("clone", path, e))?;
    let metadata = file
        .metadata()
        .map_err(|e| SessionError::io("stat", path, e))?;
    let identity = crate::FileIdentity::from_file(&file)
        .map_err(|error| SessionError::io("identity", path, error))?;
    let rescan_reason = match cursor.identity {
        Some(old) if old != identity => Some(crate::RescanReason::Replaced),
        Some(_) if metadata.len() < cursor.offset => Some(crate::RescanReason::Truncated),
        _ => None,
    };
    if cursor.identity.is_none() || rescan_reason.is_some() {
        cursor.offset = 0;
        cursor.pending.clear();
    }
    file.seek(SeekFrom::Start(cursor.offset))
        .map_err(|e| SessionError::io("seek", path, e))?;
    let start = cursor.offset;
    let mut contents = Vec::new();
    let read = file
        .take(budget.min(crate::SESSION_INCREMENT_READ_LIMIT_BYTES))
        .read_to_end(&mut contents);
    // Failed parsing or a partial I/O failure still spent this poll's bytes.
    *read_bytes = contents.len() as u64;
    read.map_err(|e| SessionError::io("read", path, e))?;
    Ok(AppendedBytes {
        has_more: start + (contents.len() as u64) < metadata.len(),
        contents,
        start_offset: start,
        identity,
        rescan_reason,
    })
}

// A bounded lexical/structural scan of an oversized provider envelope.
// Key order and tool-output contents cannot decide whether a record is text.
// The normal serde parser remains responsible for retained conversation JSON.
// Only discriminators and the bounded native ids a turn mark needs are kept,
// so a discarded record still answers or asks its call.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct LargeRecord {
    frames: Vec<JsonFrame>,
    token: Option<JsonString>,
    root_kind: String,
    payload_kind: String,
    /// Native reset metadata only; replacement context bodies are discarded.
    #[serde(default)]
    context_aware: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    content_array: bool,
    conversation: bool,
    invalid: bool,
    /// Scanned by a build that keeps native ids. A checkpoint without it is
    /// reread from the record's start (`ConversationCursor::restore`).
    #[serde(default)]
    native_ids: bool,
    /// Claude native tool blocks, in record order, bounded like a parsed
    /// record's marks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<LargeTool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tool_capacity: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    payload_question: bool,
    /// The payload's tool is `request_user_input_async`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    payload_async: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload_call: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload_turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload_mode: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    plan_item: bool,
    /// Open brackets past the 64 frames kept, matched by their closers.
    #[serde(default)]
    overflow: u32,
    /// Grok's oversized records are read without their bodies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grok: Option<Box<crate::grok::LargeLine>>,
    /// Scanned by a build that keeps what the wake reader needs (the two
    /// fields below). A checkpoint without it is reread from the record's
    /// start (`ConversationCursor::restore`).
    #[serde(default)]
    wake_aware: bool,
    /// The ids of the calls in the record to a tool that can start background
    /// work (`wake::starts_work`), bounded like a parsed record's marks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wake_calls: Vec<String>,
    /// The record holds a call or a result the wake reader cannot pair with
    /// its other half: a `TaskStop` (whose task is in a field the scan does
    /// not keep), a call or result with no id it can keep, or more calls than
    /// the bound.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    wake_lost: bool,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LargeTool {
    /// An `AskUserQuestion` call, with the id its block named.
    Question(Option<String>),
    /// A tool result naming the call it answers.
    Result(String),
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
enum Scope {
    #[default]
    Root,
    Payload,
    Message,
    Content,
    Block,
    Other,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct JsonFrame {
    scope: Scope,
    object: bool,
    key: String,
    expecting_key: bool,
    block_kind: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_use: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_question: bool,
    /// The block's `name` is a tool that can start background work.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_start: bool,
    /// The block's `name` is the tool that stops a task.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_stop: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    block_result: bool,
    /// A block's `id` or `tool_use_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    call: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    plan_item: bool,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct JsonString {
    key: bool,
    scope: Scope,
    field: String,
    value: String,
    escaped: bool,
}
type Discard = std::result::Result<Option<TurnMark>, SkipReason>;
/// The string values a turn mark needs from a discarded record.
fn native_id(scope: Scope, field: &str) -> bool {
    matches!(
        (scope, field),
        (Scope::Block, "id" | "tool_use_id")
            | (
                Scope::Payload,
                "call_id" | "turn_id" | "collaboration_mode_kind"
            )
    )
}
/// A kept id is cut one byte past the native limit, so an id too long for a
/// parsed record is too long here as well.
fn bounded(id: Option<String>) -> std::result::Result<Option<String>, SkipReason> {
    match id.filter(|id| !id.is_empty()) {
        Some(id) if id.len() > NATIVE_ID_LIMIT_BYTES => Err(SkipReason::UserTurnCapacity),
        id => Ok(id),
    }
}
impl LargeRecord {
    fn value_scope(&self) -> Scope {
        match self.frames.last() {
            None => Scope::Root,
            Some(f) if !f.object && f.scope == Scope::Content => Scope::Block,
            Some(f) => match (f.scope, f.key.as_str()) {
                (Scope::Root, "payload") => Scope::Payload,
                (Scope::Root, "message") => Scope::Message,
                (Scope::Message, "content") => Scope::Content,
                _ => Scope::Other,
            },
        }
    }
    fn feed(&mut self, bytes: &[u8]) {
        if let Some(grok) = self.grok.as_mut() {
            // Grok's scan alone decides a Grok record; one it cannot follow
            // is unreadable.
            if !self.invalid && grok.feed(bytes).is_err() {
                self.invalid = true;
            }
            return;
        }
        for &byte in bytes {
            if let Some(token) = self.token.as_mut() {
                let id = !token.key && native_id(token.scope, &token.field);
                if token.escaped {
                    // Escaped discriminator spellings stay unknown, never
                    // incorrectly authorizing a discarded conversation.
                    if token.value.len() < 64 {
                        token.value.push('\\');
                    }
                    token.escaped = false;
                } else if byte == b'\\' {
                    self.invalid |= token.key
                        || id
                        || token.field == "type"
                        || (token.field == "name"
                            && matches!(token.scope, Scope::Payload | Scope::Block));
                    token.escaped = true;
                } else if byte == b'"' {
                    let token = self.token.take().unwrap();
                    if token.key {
                        if let Some(frame) = self.frames.last_mut() {
                            frame.key = token.value;
                            frame.expecting_key = false;
                        }
                    } else if id {
                        match token.scope {
                            Scope::Block => {
                                if let Some(frame) = self.frames.last_mut() {
                                    frame.call = Some(token.value);
                                }
                            }
                            _ => match token.field.as_str() {
                                "call_id" => self.payload_call = Some(token.value),
                                "turn_id" => self.payload_turn = Some(token.value),
                                _ => self.payload_mode = Some(token.value),
                            },
                        }
                    } else if token.scope == Scope::Root && token.field == "timestamp" {
                        self.timestamp = Some(token.value);
                    } else if token.field == "type" {
                        match token.scope {
                            Scope::Root => self.root_kind = token.value,
                            Scope::Payload => self.payload_kind = token.value,
                            Scope::Block => {
                                let tool = matches!(
                                    token.value.as_str(),
                                    "tool_result" | "tool_use" | "thinking" | "redacted_thinking"
                                );
                                self.conversation |= !tool;
                                if let Some(frame) = self.frames.last_mut() {
                                    frame.block_kind = tool;
                                    frame.block_use = token.value == "tool_use";
                                    frame.block_result = token.value == "tool_result";
                                }
                            }
                            _ => {
                                if self.frames.last().is_some_and(|frame| frame.plan_item) {
                                    self.plan_item = token.value == "Plan";
                                }
                            }
                        }
                    } else if token.field == "name" {
                        match token.scope {
                            Scope::Payload => {
                                self.payload_question = token.value == "request_user_input";
                                self.payload_async = token.value == "request_user_input_async"
                            }
                            Scope::Block => {
                                if let Some(frame) = self.frames.last_mut() {
                                    frame.block_question = token.value == "AskUserQuestion";
                                    frame.block_start = starts_work(&token.value);
                                    frame.block_stop = stops_work(&token.value);
                                }
                            }
                            _ => (),
                        }
                    }
                } else if id {
                    // Native ids are ASCII; any other byte is not certified.
                    self.invalid |= !byte.is_ascii();
                    if token.value.len() <= NATIVE_ID_LIMIT_BYTES {
                        token.value.push(byte as char);
                    }
                } else if token.value.len() < 64
                    && (token.key
                        || matches!(token.field.as_str(), "type" | "name")
                        || (token.scope == Scope::Root && token.field == "timestamp"))
                {
                    token.value.push(byte as char);
                }
                continue;
            }
            match byte {
                b'"' => {
                    let frame = self.frames.last();
                    let key = frame.is_some_and(|f| f.object && f.expecting_key);
                    let scope = frame.map(|f| f.scope).unwrap_or_default();
                    let field = frame.map(|f| f.key.clone()).unwrap_or_default();
                    if !key && self.value_scope() == Scope::Content {
                        self.conversation = true;
                    }
                    self.token = Some(JsonString {
                        key,
                        scope,
                        field,
                        value: String::new(),
                        escaped: false,
                    });
                }
                b'{' | b'[' => {
                    if self.frames.len() >= 64 {
                        self.overflow = self.overflow.saturating_add(1);
                        self.invalid = true;
                        continue;
                    }
                    let scope = self.value_scope();
                    let plan_item = self
                        .frames
                        .last()
                        .is_some_and(|frame| frame.scope == Scope::Payload && frame.key == "item");
                    let object = byte == b'{';
                    if scope == Scope::Content {
                        self.content_array = !object;
                        self.conversation |= object;
                    }
                    self.frames.push(JsonFrame {
                        scope,
                        object,
                        key: String::new(),
                        expecting_key: object,
                        block_kind: false,
                        block_use: false,
                        block_question: false,
                        block_start: false,
                        block_stop: false,
                        block_result: false,
                        call: None,
                        plan_item,
                    });
                }
                b'}' | b']' => {
                    if self.overflow > 0 {
                        self.overflow -= 1;
                    } else if let Some(frame) = self.frames.pop() {
                        self.invalid |= frame.object != (byte == b'}');
                        if frame.scope == Scope::Block {
                            self.conversation |= !frame.block_kind;
                            self.wake_block(&frame);
                            let tool = if frame.block_use && frame.block_question {
                                Some(LargeTool::Question(frame.call))
                            } else if frame.block_result {
                                frame.call.map(LargeTool::Result)
                            } else {
                                None
                            };
                            if let Some(tool) = tool {
                                if self.tools.len() == TOOL_MARK_LIMIT {
                                    self.tool_capacity = true;
                                } else {
                                    self.tools.push(tool);
                                }
                            }
                        }
                    } else {
                        self.invalid = true;
                    }
                }
                b',' => {
                    if let Some(frame) = self.frames.last_mut() {
                        frame.expecting_key = frame.object;
                        frame.key.clear();
                    }
                }
                _ => (),
            }
        }
    }

    /// A native replacement boundary, without any replacement-context body.
    fn context_reset(&self, agent: Agent) -> Option<crate::ConversationEvent> {
        if agent != Agent::Codex || self.root_kind != "compacted" || !self.certified() {
            return None;
        }
        let at =
            crate::timestamp_ms(Some(&serde_json::Value::String(self.timestamp.clone()?))).ok()?;
        Some(
            crate::ConversationEvent::new("system", crate::EventKind::Injected, at, "")
                .with_context_reset(true),
        )
    }

    /// Whether the scan certifies the record's structure.
    fn certified(&self) -> bool {
        self.native_ids && !self.invalid && self.frames.is_empty() && self.token.is_none()
    }

    /// What a closed block says about background work. A call is kept by the
    /// id its result names it by; one the wake reader cannot pair with its
    /// result is lost.
    fn wake_block(&mut self, frame: &JsonFrame) {
        let id = frame
            .call
            .as_ref()
            .filter(|id| !id.is_empty() && id.len() <= NATIVE_ID_LIMIT_BYTES);
        if frame.block_use && frame.block_start {
            match id {
                Some(id) if self.wake_calls.len() < TOOL_MARK_LIMIT => {
                    self.wake_calls.push(id.clone())
                }
                _ => self.wake_lost = true,
            }
        } else if (frame.block_use && frame.block_stop) || (frame.block_result && id.is_none()) {
            self.wake_lost = true;
        }
    }

    /// What a discarded record leaves of the background work its agent waits
    /// on (PRD agent-blocked-state B18), told by the record's structure and
    /// never by its words: nothing for a format that reports none or a kind of
    /// record the wake reader takes no mark from; that the record may have
    /// said something, so no device is proven until the process starts again,
    /// for the kinds that carry a task's words (a notification, a hook) and
    /// for a record this scan cannot follow; the calls it names to tools that
    /// can start work; and the answers it gives to the calls it names, which
    /// the tracker knows to be unreadable (a screenshot answers a call that
    /// started nothing, a command's output may have been the start of a
    /// task).
    fn wake_mark(scan: Option<&Self>, agent: Agent) -> Option<TurnMark> {
        if !agent.reports_wake_devices() {
            return None;
        }
        let lost = || Some(TurnMark::Wake(vec![WakeMark::Lost]));
        let Some(scan) = scan.filter(|scan| scan.certified() && scan.wake_aware) else {
            return lost();
        };
        let marks: Vec<_> = match Record::of(&scan.root_kind) {
            // A record with no kind is one whose kind is not known.
            None if scan.root_kind.is_empty() => return lost(),
            None => return None,
            Some(Record::Attachment | Record::QueueOperation) => return lost(),
            _ if scan.wake_lost || scan.tool_capacity => return lost(),
            Some(Record::Assistant) => scan
                .wake_calls
                .iter()
                .map(|call| WakeMark::Call { call: call.clone() })
                .collect(),
            Some(Record::User) => scan
                .tools
                .iter()
                .filter_map(|tool| match tool {
                    LargeTool::Result(call) => Some(WakeMark::Answered {
                        call: call.clone(),
                        outcome: StopOutcome::Unreadable,
                    }),
                    LargeTool::Question(_) => None,
                })
                .collect(),
        };
        (!marks.is_empty()).then_some(TurnMark::Wake(marks))
    }

    /// A record whose structure no bounded scan certifies: what its turn
    /// waits for is not known until the next turn starts or a person writes.
    fn unreadable() -> (SkipReason, Discard) {
        (
            SkipReason::ConversationCapacity,
            Ok(Some(TurnMark::Unreadable)),
        )
    }

    /// The record's skip reason and the turn marks its parsed form would
    /// give. A record the scan certifies keeps its tool marks and loses its
    /// text (`conversation_capacity` when it had any). The marks only text
    /// gives (a person's message, an interruption) clear or end a wait and
    /// never start one, so losing them can hold a bell but never ring one
    /// into a menu. Any other record is unreadable rather than a read
    /// failure, so one record never stops every later read of the session.
    fn discard(self, agent: Agent) -> (SkipReason, Discard) {
        if !self.certified() {
            return Self::unreadable();
        }
        let turn = match agent {
            // Grok's oversized records are read by its own scan.
            Agent::OpenCode
            | Agent::Grok
            | Agent::Pi
            | Agent::Omp
            | Agent::Cursor
            | Agent::Antigravity => None,
            Agent::Codex => self.codex(),
            Agent::Claude => self.claude(),
        };
        turn.unwrap_or_else(Self::unreadable)
    }

    fn claude(self) -> Option<(SkipReason, Discard)> {
        let lost = match self.root_kind.as_str() {
            "user" | "assistant" => self.conversation || !self.content_array,
            "ai-title" => true,
            "" => return None,
            _ => return Some((SkipReason::NonConversationCapacity, Ok(None))),
        };
        let reason = if lost {
            SkipReason::ConversationCapacity
        } else {
            SkipReason::NonConversationCapacity
        };
        if self.tool_capacity {
            return Some((reason, Err(SkipReason::UserTurnCapacity)));
        }
        let mut marks = Vec::new();
        for tool in self.tools {
            let mark = match (self.root_kind.as_str(), tool) {
                ("assistant", LargeTool::Question(call)) => match bounded(call) {
                    Ok(Some(call)) => ToolTurnMark::Asked {
                        call,
                        content: None,
                    },
                    Ok(None) => return Some((reason, Err(SkipReason::UserTurnInvalid))),
                    Err(invalid) => return Some((reason, Err(invalid))),
                },
                ("user", LargeTool::Result(call)) => match bounded(Some(call)) {
                    Ok(Some(call)) => ToolTurnMark::Answered { call },
                    Ok(None) => continue,
                    Err(invalid) => return Some((reason, Err(invalid))),
                },
                _ => continue,
            };
            marks.push(mark);
        }
        Some((
            reason,
            Ok((!marks.is_empty()).then_some(TurnMark::Tools(marks))),
        ))
    }

    /// A Codex message carries no turn mark of its own: its turn's start
    /// record comes before it.
    fn codex(self) -> Option<(SkipReason, Discard)> {
        let turn = || bounded(self.payload_turn.clone());
        let mark = match (self.root_kind.as_str(), self.payload_kind.as_str()) {
            ("", _) | ("response_item", "") => return None,
            ("response_item", "message") => {
                return Some((SkipReason::ConversationCapacity, Ok(None)));
            }
            ("response_item", "function_call") if self.payload_question => {
                match bounded(self.payload_call.clone()) {
                    Ok(Some(call)) => Ok(Some(TurnMark::Tools(vec![ToolTurnMark::Asked {
                        call,
                        content: None,
                    }]))),
                    Ok(None) => Err(SkipReason::UserTurnInvalid),
                    Err(reason) => Err(reason),
                }
            }
            // How many questions a call this large asked is not known, so
            // which replies end the wait is not either.
            ("response_item", "function_call") if self.payload_async => {
                Ok(Some(TurnMark::Unreadable))
            }
            ("response_item", "function_call_output") => {
                bounded(self.payload_call.clone()).map(|call| {
                    call.map(|call| TurnMark::Tools(vec![ToolTurnMark::Answered { call }]))
                })
            }
            ("event_msg", "item_completed") if self.plan_item => {
                turn().map(|turn| Some(TurnMark::Plan { turn }))
            }
            ("event_msg", "task_started") => turn().map(|turn| {
                Some(TurnMark::Started {
                    turn,
                    mode: match self.payload_mode.as_deref() {
                        Some("plan") => TurnMode::Plan,
                        Some("default") => TurnMode::Other,
                        _ => TurnMode::Unknown,
                    },
                })
            }),
            ("event_msg", "task_complete") => turn().map(|turn| Some(TurnMark::Completed { turn })),
            ("event_msg", "turn_aborted") => turn().map(|turn| Some(TurnMark::Aborted { turn })),
            _ => Ok(None),
        };
        Some((SkipReason::NonConversationCapacity, mark))
    }
}
