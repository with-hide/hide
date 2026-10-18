//! The facts a session file states about where its work went (PRD
//! link-graph D-15, D-27): the branch it worked on and when, the pull request
//! addresses its tools printed, the operator's request before each, whether
//! it was an interactive run, and the pointers that say it continues another
//! session.
//!
//! They are read by the same line parser as the conversation
//! ([`crate::parse_events_at`]): one JSON parse of a record feeds both, so a
//! session file read for search, labels or links is read the same way.
//! Nothing here keeps a conversation body: a request is cut to
//! [`REQUEST_CHARS`] characters, and only the requests that a link needs
//! (the last one per branch span and the one before each pull request
//! address) leave this module.

use crate::{EventKind, PrSighting, pull_request_addresses};
use serde_json::Value;

/// A request is kept to this many characters (D-40).
pub const REQUEST_CHARS: usize = 500;
pub const REQUESTS_PER_READ: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestMark {
    pub offset: u64,
    pub at_unix_ms: u64,
    pub text: String,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub letter_kind: Option<String>,
}

/// One stretch of records that ran on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchSpan {
    /// The branch the records name, or `None` for a record on no branch
    /// (a detached head) or one whose chunk did not say.
    pub branch: Option<String>,
    /// The chunk had not stated a branch when these records came (a Codex
    /// chunk read after its `session_meta`): the span is on the branch the
    /// file was last read on, which only its reader's cursor knows.
    #[serde(default)]
    pub inherited: bool,
    pub first_at_unix_ms: u64,
    pub last_at_unix_ms: u64,
    /// The last operator request made inside this span.
    pub last_request: Option<String>,
}

/// A pull request address the session printed, from a tool's output or from
/// the agent's own `pr-link` record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrMark {
    /// `owner/name` as printed.
    pub repository: String,
    pub number: u64,
    pub at_unix_ms: u64,
    /// The operator's last request before the address, when this chunk held
    /// one; `None` means the request before it is the session's last one
    /// recorded so far.
    pub request: Option<String>,
}

/// What one chunk of a session file says about links.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkFacts {
    #[serde(default)]
    pub requests: Vec<RequestMark>,
    #[serde(default)]
    pub requests_capped: usize,
    #[serde(default)]
    pub issues: Vec<PrMark>,
    /// The session the records belong to: Claude Code's `sessionId`, which a
    /// subagent file shares with its parent (D-28), or Codex's `session_meta`
    /// id, or for a Codex subagent thread its parent thread.
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    /// `Some(false)` for a run the agent recorded as non-interactive (Claude
    /// Code's `sdk-*` entrypoint, a Codex `exec` source), `Some(true)` for an
    /// interactive one, `None` when this chunk did not say (D-43).
    pub interactive: Option<bool>,
    /// The chunk came from a subagent's file or thread; its requests are the
    /// parent's prompt to it, not the operator's.
    pub subagent: bool,
    /// Claude Code: the parent pointer of the file's first record, which a
    /// session that continues another (compaction, resume) points into the
    /// previous file's last record with.
    pub first_parent_uuid: Option<String>,
    /// Claude Code: the last record's id.
    pub last_uuid: Option<String>,
    /// Codex: the session this one was forked from; a fork is its own line.
    pub forked_from: Option<String>,
    pub spans: Vec<BranchSpan>,
    pub prs: Vec<PrMark>,
    pub first_at_unix_ms: Option<u64>,
    pub last_at_unix_ms: Option<u64>,
    /// The last operator request in the chunk with its time.
    pub last_request: Option<(u64, String)>,
}

impl LinkFacts {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Collects [`LinkFacts`] while the line parser walks a chunk.
#[derive(Debug, Default)]
pub(crate) struct LinkAccumulator {
    facts: LinkFacts,
    requests: std::collections::VecDeque<RequestMark>,
    /// The branch the chunk's records are on so far.
    current: Option<Option<String>>,
    /// The record being read came from a subagent.
    sidechain_line: bool,
    meta_seen: bool,
}

impl LinkAccumulator {
    pub(crate) fn finish(mut self) -> LinkFacts {
        self.facts.requests = self.requests.into_iter().collect();
        self.facts
    }

    fn activity(&mut self, at: u64) {
        let facts = &mut self.facts;
        facts.first_at_unix_ms = Some(facts.first_at_unix_ms.map_or(at, |first| first.min(at)));
        facts.last_at_unix_ms = Some(facts.last_at_unix_ms.map_or(at, |last| last.max(at)));
        let inherited = self.current.is_none();
        let branch = self.current.clone().unwrap_or(None);
        match self.facts.spans.last_mut() {
            Some(span) if span.branch == branch && span.inherited == inherited => {
                span.first_at_unix_ms = span.first_at_unix_ms.min(at);
                span.last_at_unix_ms = span.last_at_unix_ms.max(at);
            }
            _ => self.facts.spans.push(BranchSpan {
                branch,
                inherited,
                first_at_unix_ms: at,
                last_at_unix_ms: at,
                last_request: None,
            }),
        }
    }

    fn set_branch(&mut self, branch: Option<&str>) {
        let branch = branch
            .map(str::trim)
            .filter(|branch| is_branch(branch))
            .map(str::to_owned);
        // A record that names no branch (detached `HEAD`, an empty value)
        // is no link, and it ends the span it interrupts.
        self.current = Some(branch);
    }

    /// The parser judged one record an event; an operator request is kept
    /// for the span it was made in.
    pub(crate) fn event(&mut self, kind: EventKind, at: u64, text: &str, offset: u64) {
        // A Hide letter is another agent's message, not the operator's.
        if kind != EventKind::Human || self.sidechain_line || text.trim().is_empty() {
            return;
        }
        let envelope = crate::envelope_header(text);
        if envelope.as_ref().is_some_and(|(_, kind)| kind != "request") {
            return;
        }
        let request = cut(text);
        if self.requests.len() == REQUESTS_PER_READ {
            self.requests.pop_front();
            self.facts.requests_capped += 1;
        }
        self.requests.push_back(RequestMark {
            offset,
            at_unix_ms: at,
            text: request.clone(),
            sender: envelope.as_ref().map(|(sender, _)| sender.clone()),
            letter_kind: envelope.as_ref().map(|(_, kind)| kind.clone()),
        });
        if envelope.is_some() {
            return;
        }
        if let Some(span) = self.facts.spans.last_mut() {
            span.last_request = Some(request.clone());
        }
        self.facts.last_request = Some((at, request));
    }

    pub(crate) fn sightings(&mut self, sightings: &[PrSighting]) {
        for sighting in sightings {
            self.pr(&sighting.repository, sighting.number, sighting.at_unix_ms);
        }
    }

    pub(crate) fn issue_sightings(&mut self, sightings: &[PrSighting]) {
        for sighting in sightings {
            if self.facts.issues.len() >= crate::MAX_SIGHTINGS_PER_OUTPUT {
                break;
            }
            if !self.facts.issues.iter().any(|seen| {
                seen.number == sighting.number
                    && seen.repository.eq_ignore_ascii_case(&sighting.repository)
            }) {
                self.facts.issues.push(PrMark {
                    repository: sighting.repository.clone(),
                    number: sighting.number,
                    at_unix_ms: sighting.at_unix_ms,
                    request: self
                        .facts
                        .last_request
                        .as_ref()
                        .map(|(_, text)| text.clone()),
                });
            }
        }
    }

    fn pr(&mut self, repository: &str, number: u64, at: u64) {
        let request = self
            .facts
            .last_request
            .as_ref()
            .map(|(_, text)| text.clone());
        let duplicate =
            self.facts.prs.iter().any(|mark| {
                mark.number == number && mark.repository.eq_ignore_ascii_case(repository)
            });
        if !duplicate {
            self.facts.prs.push(PrMark {
                repository: repository.to_owned(),
                number,
                at_unix_ms: at,
                request,
            });
        }
    }
}

/// A value names a branch Hide can link: not empty and not a detached head.
pub fn is_branch(branch: &str) -> bool {
    !branch.is_empty() && branch != "HEAD"
}

/// A request cut to [`REQUEST_CHARS`] characters on a character boundary.
pub fn cut(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(REQUEST_CHARS) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

/// Claude Code: what one record says about links.
pub(crate) fn claude_line(item: &Value, offset: u64, links: &mut LinkAccumulator) {
    links.sidechain_line = item
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if links.sidechain_line {
        links.facts.subagent = true;
    }
    if links.facts.session_id.is_none()
        && let Some(id) = item.get("sessionId").and_then(Value::as_str)
    {
        links.facts.session_id = Some(id.to_owned());
    }
    if links.facts.cwd.is_none()
        && let Some(cwd) = item.get("cwd").and_then(Value::as_str)
    {
        links.facts.cwd = Some(cwd.to_owned());
    }
    if let Some(entrypoint) = item.get("entrypoint").and_then(Value::as_str) {
        // `claude -p` and the SDKs record `sdk-cli`, `sdk-ts`, `sdk-py`.
        let interactive = !entrypoint.starts_with("sdk");
        links.facts.interactive = Some(links.facts.interactive.unwrap_or(true) && interactive);
    }
    if offset == 0 && links.facts.first_parent_uuid.is_none() {
        links.facts.first_parent_uuid = item
            .get("logicalParentUuid")
            .or_else(|| item.get("parentUuid"))
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(uuid) = item.get("uuid").and_then(Value::as_str) {
        links.facts.last_uuid = Some(uuid.to_owned());
    }
    let at = crate::timestamp_ms(item.get("timestamp")).ok();
    if item.get("type").and_then(Value::as_str) == Some("pr-link") {
        let repository = item.get("prRepository").and_then(Value::as_str);
        let number = item.get("prNumber").and_then(Value::as_u64);
        let from_url = item
            .get("prUrl")
            .and_then(Value::as_str)
            .and_then(|url| pull_request_addresses(url).into_iter().next());
        let address = match (repository, number) {
            (Some(repository), Some(number)) => Some((repository.to_owned(), number)),
            _ => from_url,
        };
        if let (Some((repository, number)), Some(at)) = (address, at) {
            links.pr(&repository, number, at);
        }
        return;
    }
    if !matches!(
        item.get("type").and_then(Value::as_str),
        Some("user" | "assistant" | "system")
    ) {
        return;
    }
    if item.get("gitBranch").is_some() {
        links.set_branch(item.get("gitBranch").and_then(Value::as_str));
    }
    if let Some(at) = at {
        links.activity(at);
    }
}

/// Pi records a native owner and cwd, but no branch, interactive marker or
/// subagent identity. parentSession is a file pointer, not a native id.
pub(crate) fn pi_line(item: &Value, offset: u64, links: &mut LinkAccumulator) {
    if offset == 0 && item["type"] == "session" && item["version"] == 3 {
        links.facts.session_id = item["id"].as_str().map(str::to_owned);
        links.facts.cwd = item["cwd"].as_str().map(str::to_owned);
    }
    if let Ok(at) = crate::timestamp_ms(item.get("timestamp")) {
        links.activity(at);
    }
}

/// Grok keeps its owner and cwd in `summary.json`, which the link reader
/// proves beside the file; a record says only when the session was active.
pub(crate) fn grok_line(item: &Value, _offset: u64, links: &mut LinkAccumulator) {
    let at = item
        .pointer("/params/_meta/agentTimestampMs")
        .and_then(Value::as_u64)
        .map_or_else(|| crate::timestamp_ms(item.get("timestamp")), Ok);
    if let Ok(at) = at {
        links.activity(at);
    }
}

/// agy keeps its owner and cwd in its conversation index, which the link
/// reader proves beside the file; a step says only when it was written.
pub(crate) fn antigravity_line(item: &Value, _offset: u64, links: &mut LinkAccumulator) {
    if let Ok(at) = crate::timestamp_ms(item.get("created_at")) {
        links.activity(at);
    }
}

/// Codex: what one record says about links.
pub(crate) fn codex_line(item: &Value, _offset: u64, links: &mut LinkAccumulator) {
    links.sidechain_line = links.facts.subagent;
    let at = crate::timestamp_ms(item.get("timestamp")).ok();
    if item.get("type").and_then(Value::as_str) == Some("session_meta") {
        // A forked rollout repeats its parent's `session_meta` after its own;
        // only the first one describes this file.
        if links.meta_seen {
            return;
        }
        links.meta_seen = true;
        let Some(payload) = item.get("payload") else {
            return;
        };
        let parent_thread = payload
            .pointer("/source/subagent/thread_spawn/parent_thread_id")
            .and_then(Value::as_str);
        let subagent = parent_thread.is_some()
            || payload.get("thread_source").and_then(Value::as_str) == Some("subagent");
        links.facts.subagent = subagent;
        links.sidechain_line = subagent;
        links.facts.session_id = parent_thread
            .or_else(|| payload.get("parent_thread_id").and_then(Value::as_str))
            .filter(|_| subagent)
            .or_else(|| payload.get("id").and_then(Value::as_str))
            .map(str::to_owned);
        links.facts.cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let exec = payload.get("source").and_then(Value::as_str) == Some("exec")
            || payload.get("originator").and_then(Value::as_str) == Some("codex_exec");
        links.facts.interactive = Some(!exec);
        links.facts.forked_from = payload
            .get("forked_from_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        links.set_branch(payload.pointer("/git/branch").and_then(Value::as_str));
        if let Some(at) = at {
            links.activity(at);
        }
        return;
    }
    if !matches!(
        item.get("type").and_then(Value::as_str),
        Some("response_item" | "event_msg" | "turn_context")
    ) {
        return;
    }
    if let Some(at) = at {
        links.activity(at);
    }
}

/// The line-level link reader for a provider, paired with its event parser.
pub(crate) type LinkLine = fn(&Value, u64, &mut LinkAccumulator);

/// The most files one listing names, newest first; older ones wait for a
/// later listing with an earlier `since`.
pub const CANDIDATE_LIMIT: usize = 2_000;
/// The most files one read request may name.
pub const READ_FILE_LIMIT: usize = 8;

/// A session file (or OpenCode session) changed since a time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub agent: crate::Agent,
    /// The file's path in this machine's spelling, or `opencode/<id>` for an
    /// OpenCode session, which lives in OpenCode's database.
    pub path: String,
    /// Length and modification time: a changed stamp is a file to read.
    pub stamp: String,
    pub modified_unix_ms: u64,
}

/// One file to read from where the last read stopped.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReadRequest {
    pub agent: crate::Agent,
    pub path: String,
    #[serde(default)]
    pub checkpoint: Option<crate::ConversationCheckpoint>,
}

/// What one file's read found.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReadAnswer {
    pub agent: crate::Agent,
    pub path: String,
    /// Where the next read resumes.
    #[serde(default)]
    pub checkpoint: Option<crate::ConversationCheckpoint>,
    pub has_more: bool,
    /// The read started over (the file was replaced or truncated); the
    /// caller drops what it had from this file before applying `facts`.
    pub rescanned: bool,
    #[serde(default)]
    pub facts: LinkFacts,
    /// A code naming why the file could not be read; never a path.
    #[serde(default)]
    pub error: Option<String>,
}

/// The session files under `home` modified at or after `since_unix_ms` and,
/// with `until_unix_ms`, at or before it, newest first, at most
/// [`CANDIDATE_LIMIT`]. A full page may hide older files: the caller asks
/// again with `until_unix_ms` at the page's oldest time.
pub fn candidates(
    home: &std::path::Path,
    since_unix_ms: u64,
    until_unix_ms: Option<u64>,
) -> Result<Vec<Candidate>, String> {
    let window = Window {
        since: since_unix_ms,
        until: until_unix_ms.unwrap_or(u64::MAX),
    };
    let mut found = Vec::new();
    let mut visited = 0_usize;
    walk(
        &home.join(crate::CLAUDE_SESSIONS),
        crate::Agent::Claude,
        3,
        window,
        &mut found,
        &mut visited,
    )?;
    walk(
        &home.join(crate::CODEX_SESSIONS),
        crate::Agent::Codex,
        4,
        window,
        &mut found,
        &mut visited,
    )?;
    opencode_candidates(home, window, &mut found)?;
    for agent in [crate::Agent::Pi, crate::Agent::Omp] {
        if let Ok(root) = crate::native_file::root(home, agent) {
            walk(&root, agent, 1, window, &mut found, &mut visited)?;
        }
    }
    if let Ok(root) = crate::native_file::root(home, crate::Agent::Grok) {
        walk(
            &root,
            crate::Agent::Grok,
            2,
            window,
            &mut found,
            &mut visited,
        )?;
    }
    found.sort_by(|left, right| {
        right
            .modified_unix_ms
            .cmp(&left.modified_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    found.truncate(CANDIDATE_LIMIT);
    Ok(found)
}

/// The modification times one listing takes, both ends included.
#[derive(Clone, Copy)]
struct Window {
    since: u64,
    until: u64,
}

/// The most directory entries one listing visits.
const VISIT_LIMIT: usize = 100_000;

fn walk(
    root: &std::path::Path,
    agent: crate::Agent,
    depth: usize,
    window: Window,
    found: &mut Vec<Candidate>,
    visited: &mut usize,
) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("links_root_unreadable".to_owned()),
    };
    for entry in entries {
        *visited += 1;
        if *visited > VISIT_LIMIT {
            return Err("links_listing_capacity".to_owned());
        }
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth > 0 {
                walk(&path, agent, depth - 1, window, found, visited)?;
            }
            continue;
        }
        if !kind.is_file()
            || path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            || (agent == crate::Agent::Grok && crate::grok::group_of(&path).is_none())
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = modified_ms(&metadata);
        if modified < window.since || modified > window.until {
            continue;
        }
        found.push(Candidate {
            agent,
            path: path.to_string_lossy().into_owned(),
            stamp: format!("{}:{}", metadata.len(), modified_ns(&metadata)),
            modified_unix_ms: modified,
        });
    }
    Ok(())
}

fn modified_ns(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}

fn modified_ms(metadata: &std::fs::Metadata) -> u64 {
    (modified_ns(metadata) / 1_000_000) as u64
}

/// How an OpenCode session is named where a file path would be.
pub const OPENCODE_PREFIX: &str = "opencode/";

fn opencode_candidates(
    home: &std::path::Path,
    window: Window,
    found: &mut Vec<Candidate>,
) -> Result<(), String> {
    let path = crate::opencode::database_path(home)?;
    if !path.is_file() {
        return Ok(());
    }
    let rows = crate::opencode::with_read(home, |connection, schema| {
        connection
        .prepare(&format!(
            "SELECT id, time_updated, (SELECT count(*) FROM {messages} WHERE session_id = {sessions}.id) \
             FROM {sessions} WHERE time_updated >= ?1 AND time_updated <= ?2 \
             ORDER BY time_updated DESC LIMIT ?3",
            sessions = schema.sessions(),
            messages = schema.messages()
        ))
        .and_then(|mut statement| {
            statement
                .query_map(
                    rusqlite::params![
                        i64::try_from(window.since).unwrap_or(i64::MAX),
                        i64::try_from(window.until).unwrap_or(i64::MAX),
                        CANDIDATE_LIMIT as i64
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()
        })
    })?;
    let Ok(rows) = rows else {
        return Err("opencode_db_unreadable".to_owned());
    };
    for (id, updated, messages) in rows {
        let updated = u64::try_from(updated).unwrap_or(0);
        found.push(Candidate {
            agent: crate::Agent::OpenCode,
            path: format!("{OPENCODE_PREFIX}{id}"),
            stamp: format!("{messages}:{updated}"),
            modified_unix_ms: updated,
        });
    }
    Ok(())
}

/// Read each file from its checkpoint, at most [`READ_FILE_LIMIT`] files
/// and one read's budget each. A path outside the agent's own transcript
/// root under `home` is refused, so a request can never make the reader open
/// a file elsewhere.
pub fn read(home: &std::path::Path, requests: &[ReadRequest]) -> Vec<ReadAnswer> {
    requests
        .iter()
        .take(READ_FILE_LIMIT)
        .map(|request| read_one(home, request))
        .collect()
}

fn read_one(home: &std::path::Path, request: &ReadRequest) -> ReadAnswer {
    let mut answer = ReadAnswer {
        agent: request.agent,
        path: request.path.clone(),
        checkpoint: request.checkpoint.clone(),
        has_more: false,
        rescanned: false,
        facts: LinkFacts::default(),
        error: None,
    };
    if request.agent == crate::Agent::OpenCode {
        read_opencode(home, request, &mut answer);
        return answer;
    }
    let path = match inside_root(home, request.agent, std::path::Path::new(&request.path)) {
        Ok(path) => path,
        Err(code) => {
            answer.error = Some(code);
            return answer;
        }
    };
    let mut cursor = request.checkpoint.clone().map_or_else(
        crate::ConversationCursor::new,
        crate::ConversationCursor::restore,
    );
    let pi_before = if request.agent.requires_native_file_proof() {
        match crate::native_file::header(request.agent, &path).and_then(|header| {
            crate::confirm_session_file(
                home,
                request.agent,
                &path,
                Some(&header.id),
                header.cwd.to_str(),
            )
            .map(|proof| (header, proof))
        }) {
            Ok(before) => Some(before),
            Err(error) => {
                answer.error = Some(error.to_string());
                return answer;
            }
        }
    } else {
        None
    };
    let stamp = crate::stamp::stamp_at(&path);
    match cursor.read(request.agent, &path) {
        Ok(parsed) => {
            let mut facts = parsed.links;
            if let Some((header, before)) = pi_before {
                let after = crate::confirm_session_file(
                    home,
                    request.agent,
                    &path,
                    Some(&header.id),
                    header.cwd.to_str(),
                );
                if !after.is_ok_and(|after| {
                    crate::label_owner::same_read(&path, &before, &after, stamp.as_deref())
                }) {
                    answer.error = Some("label_session_read_changed".to_owned());
                    return answer;
                }
                facts.session_id = Some(header.id);
                facts.cwd = header.cwd.to_str().map(str::to_owned);
            }
            answer.rescanned = parsed.rescan_reason.is_some();
            answer.facts = facts;
            answer.checkpoint = Some(cursor.checkpoint());
            answer.has_more = cursor.has_more();
        }
        // Only a file that is not there is gone; a read the system refused
        // is a failure, never a disappearance (D-35).
        Err(crate::SessionError::Io { source, .. }) => {
            let missing = source
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound);
            answer.error = Some(
                if missing {
                    "session_file_missing"
                } else {
                    "session_unreadable"
                }
                .to_owned(),
            );
        }
        Err(crate::SessionError::SessionFileMissing) => {
            answer.error = Some("session_file_missing".to_owned());
        }
        Err(crate::SessionError::Capacity { .. }) => {
            answer.error = Some("session_capacity".to_owned());
        }
        Err(_) => answer.error = Some("session_unreadable".to_owned()),
    }
    answer
}

/// [`crate::inside_session_root`] for one agent, as a read answer's error.
fn inside_root(
    home: &std::path::Path,
    agent: crate::Agent,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    crate::inside_session_root(home, &[agent], path).map_err(|refusal| {
        match refusal {
            crate::RootRefusal::Unsupported => "session_kind_unsupported",
            crate::RootRefusal::Outside => "links_session_outside_roots",
            crate::RootRefusal::Missing => "session_file_missing",
            crate::RootRefusal::Unreadable => "session_unreadable",
        }
        .to_owned()
    })
}

fn read_opencode(home: &std::path::Path, request: &ReadRequest, answer: &mut ReadAnswer) {
    let Some(id) = request.path.strip_prefix(OPENCODE_PREFIX) else {
        answer.error = Some("links_session_outside_roots".to_owned());
        return;
    };
    let label = crate::label_transcript::LabelTranscriptRequest {
        agent: crate::Agent::OpenCode,
        reference_kind: "id".to_owned(),
        reference_value: id.to_owned(),
        cwd: None,
        checkpoint: request.checkpoint.clone(),
        subagents: Default::default(),
        turns: None,
    };
    // A link line is each session as its own row records it, a subagent's
    // child session included; it never joins the root's conversation.
    let (transcript, proven) =
        match crate::opencode::read_as(home, &label, crate::opencode::Owner::Recorded) {
            Ok(read) => read,
            Err(code) => {
                answer.error = Some(code);
                return;
            }
        };
    let (directory, parent) = (proven.directory, proven.parent);
    let mut links = LinkAccumulator::default();
    links.facts.session_id = Some(parent.clone().unwrap_or_else(|| id.to_owned()));
    links.facts.subagent = parent.is_some();
    links.sidechain_line = parent.is_some();
    links.facts.cwd = directory;
    links.facts.interactive = Some(true);
    // OpenCode records no branch; its sessions link by a Hide pane's place
    // and the addresses they print.
    links.current = Some(None);
    let mut sightings = transcript.pr_sightings.iter().peekable();
    for event in &transcript.events {
        while let Some(sighting) = sightings.next_if(|s| s.at_unix_ms < event.at_unix_ms) {
            links.activity(sighting.at_unix_ms);
            links.pr(&sighting.repository, sighting.number, sighting.at_unix_ms);
        }
        links.activity(event.at_unix_ms);
        let kind = match event.kind {
            crate::label_transcript::LabelEventKind::Human => EventKind::Human,
            crate::label_transcript::LabelEventKind::Assistant => EventKind::Assistant,
            crate::label_transcript::LabelEventKind::Interrupted => EventKind::Interrupted,
        };
        links.event(kind, event.at_unix_ms, &event.text, event.offset);
    }
    for sighting in sightings {
        links.activity(sighting.at_unix_ms);
        links.pr(&sighting.repository, sighting.number, sighting.at_unix_ms);
    }
    links.issue_sightings(&transcript.issue_sightings);
    answer.rescanned = transcript.rescanned.is_some();
    answer.has_more = transcript.has_more;
    answer.checkpoint = Some(transcript.checkpoint);
    answer.facts = links.finish();
}
