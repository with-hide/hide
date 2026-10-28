use crate::file_cache;
use crate::{
    Agent, ConversationCheckpoint, ConversationCursor, ConversationEvent, EventKind, ParsedSession,
    SESSION_LINE_LIMIT_BYTES, SESSION_READ_LIMIT_BYTES, SessionError, parse_events, read_bounded,
    read_bounded_line,
};
use hide_project::ProjectIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hard cap on files visited in one catalog refresh.
pub const SESSION_DISCOVERY_LIMIT: usize = 10_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFilter {
    #[default]
    All,
    Codex,
    Claude,
    Grok,
    Pi,
    Omp,
    Cursor,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl SessionFilter {
    pub fn includes(self, agent: Agent) -> bool {
        matches!(self, Self::All)
            || matches!(
                (self, agent),
                (Self::Codex, Agent::Codex)
                    | (Self::Claude, Agent::Claude)
                    | (Self::Grok, Agent::Grok)
                    | (Self::Pi, Agent::Pi)
                    | (Self::Omp, Agent::Omp)
                    | (Self::Cursor, Agent::Cursor)
                    | (Self::OpenCode, Agent::OpenCode)
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAvailability {
    Available,
    Unavailable { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectSession {
    pub id: String,
    pub agent: Agent,
    pub locator: PathBuf,
    pub checkout_path: PathBuf,
    pub first_human_request: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub updated_at_unix_ms: u64,
    pub title: Option<String>,
    pub event_count: usize,
    pub availability: SessionAvailability,
    /// The session file's stamp (`stamp::stamp_at`) when the listing read it,
    /// for a reader that compares two listings; `None` for a session kept in
    /// no file of its own (OpenCode), a file whose stamp could not be read,
    /// and every row of a node that predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<String>,
}

#[derive(Debug)]
pub enum SessionCatalogError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Capacity {
        limit: usize,
    },
}

impl Display for SessionCatalogError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(
                    formatter,
                    "session_catalog_{operation}:{}:{source}",
                    path.display()
                )
            }
            Self::Capacity { limit } => write!(formatter, "session_catalog_capacity:{limit}"),
        }
    }
}

impl Error for SessionCatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Capacity { .. } => None,
        }
    }
}

/// What one catalog read found: the project's sessions, and each agent
/// store that contributed none because it could not be read, with why. A
/// refusal never takes the other agents' sessions with it; it reaches the
/// core's log, since the operator cannot act on it from the archive.
#[derive(Debug, Default)]
pub struct ProjectSessions {
    pub sessions: Vec<ProjectSession>,
    pub refusals: Vec<SessionStoreRefusal>,
}

/// One agent store the catalog could not read. It rides the node's
/// session list as a row of its own (`{"agent", "store_refused"}`), which a
/// core that does not know the shape counts as one refused row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStoreRefusal {
    pub agent: Agent,
    #[serde(rename = "store_refused")]
    pub reason: String,
}

pub struct SessionCatalog {
    home: PathBuf,
    device_id: String,
}

impl SessionCatalog {
    pub fn new(home: &Path, device_id: impl Into<String>) -> Self {
        Self {
            home: home.to_path_buf(),
            device_id: device_id.into(),
        }
    }

    pub fn project_sessions(
        &self,
        project: &ProjectIdentity,
    ) -> Result<ProjectSessions, SessionCatalogError> {
        let mut files = Vec::new();
        let mut visited = 0;
        collect_jsonl(
            &self.home.join(crate::CLAUDE_SESSIONS),
            Agent::Claude,
            2,
            &mut files,
            &mut visited,
            SESSION_DISCOVERY_LIMIT,
        )?;
        collect_jsonl(
            &self.home.join(crate::CODEX_SESSIONS),
            Agent::Codex,
            4,
            &mut files,
            &mut visited,
            SESSION_DISCOVERY_LIMIT,
        )?;
        for agent in [Agent::Pi, Agent::Omp] {
            if let Ok(root) = crate::native_file::root(&self.home, agent) {
                collect_jsonl(
                    &root,
                    agent,
                    1,
                    &mut files,
                    &mut visited,
                    SESSION_DISCOVERY_LIMIT,
                )?;
            }
        }
        if let Ok(root) = crate::native_file::root(&self.home, Agent::Grok) {
            collect_grok(&root, &mut files, &mut visited, SESSION_DISCOVERY_LIMIT)?;
        }
        if let Ok(root) = crate::native_file::root(&self.home, Agent::Cursor) {
            collect_jsonl(
                &root,
                Agent::Cursor,
                2,
                &mut files,
                &mut visited,
                SESSION_DISCOVERY_LIMIT,
            )?;
        }

        let mut refusals = Vec::new();
        let mut sessions = crate::opencode::catalog(
            &self.home,
            &self.device_id,
            project,
            &mut visited,
            SESSION_DISCOVERY_LIMIT,
        )?
        .unwrap_or_else(|reason| {
            refusals.push(SessionStoreRefusal {
                agent: Agent::OpenCode,
                reason,
            });
            Vec::new()
        });
        // Every listed file is one this call saw, so a kept fact for any
        // other file is for a file that left the listing.
        let listed = files
            .iter()
            .map(|(_, path)| path.clone())
            .collect::<HashSet<_>>();
        for (agent, path) in files {
            if agent.requires_native_file_proof()
                && crate::native_file::inside_root(&self.home, agent, &path).is_err()
            {
                continue;
            }
            // Taken before the bytes are read: a change during the read
            // leaves a stamp older than the bytes, so the next call reads again.
            let stamp = crate::stamp::stamp_at(&path);
            let kept_stamp = file_cache::keeps(agent).then(|| stamp.clone()).flatten();
            let Some(cwd) = (if agent == Agent::Cursor {
                crate::cursor::header(&self.home, &path)
                    .ok()
                    .map(|header| header.cwd)
            } else {
                file_cache::header_cwd(&path, kept_stamp.clone(), || session_cwd(agent, &path))
            }) else {
                continue;
            };
            let Ok(identity) = hide_project::resolve(&cwd, &self.device_id) else {
                continue;
            };
            if identity.id != project.id {
                continue;
            }
            let session = if file_cache::keeps(agent) {
                Some(file_cache::session(&path, kept_stamp, |earlier| {
                    list_transcript(agent, &path, &cwd, earlier.and_then(|(_, kept)| kept))
                }))
            } else {
                read_project_session(&self.home, agent, path.clone(), cwd.clone())
            }
            .map(|session| ProjectSession { stamp, ..session });
            if let Some(session) = session {
                sessions.push(session);
            }
        }
        file_cache::retain(&self.home, &listed);
        sessions.sort_by(|left, right| {
            right
                .updated_at_unix_ms
                .cmp(&left.updated_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        // A refusal is a row of the node's answer, under the same cap.
        if sessions.len() + refusals.len() > SESSION_DISCOVERY_LIMIT {
            return Err(SessionCatalogError::Capacity {
                limit: SESSION_DISCOVERY_LIMIT,
            });
        }
        Ok(ProjectSessions { sessions, refusals })
    }

    pub fn filtered(
        sessions: &[ProjectSession],
        filter: SessionFilter,
        query: &str,
    ) -> Vec<ProjectSession> {
        let query = query.trim().to_lowercase();
        sessions
            .iter()
            .filter(|session| filter.includes(session.agent))
            .filter(|session| {
                query.is_empty()
                    || session
                        .first_human_request
                        .as_ref()
                        .is_some_and(|value| value.to_lowercase().contains(&query))
                    || session
                        .title
                        .as_ref()
                        .is_some_and(|value| value.to_lowercase().contains(&query))
                    || session
                        .checkout_path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect()
    }
}

fn collect_jsonl(
    root: &Path,
    agent: Agent,
    depth: usize,
    output: &mut Vec<(Agent, PathBuf)>,
    visited: &mut usize,
    limit: usize,
) -> Result<(), SessionCatalogError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(SessionCatalogError::Io {
                operation: "read_directory",
                path: root.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        *visited = visited.saturating_add(1);
        if *visited > limit {
            return Err(SessionCatalogError::Capacity { limit });
        }
        let path = entry
            .map_err(|source| SessionCatalogError::Io {
                operation: "read_entry",
                path: root.to_path_buf(),
                source,
            })?
            .path();
        if agent.requires_native_file_proof()
            && fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink())
        {
            continue;
        }
        if path.is_dir() && depth > 0 {
            collect_jsonl(&path, agent, depth - 1, output, visited, limit)?;
        } else if (agent == Agent::Cursor
            && path.file_name().is_some_and(|name| name == "store.db"))
            || (agent != Agent::Cursor
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl"))
        {
            output.push((agent, path));
        }
    }
    Ok(())
}

/// Grok keeps a folder per session, grouped by cwd; only its conversation
/// file is a session, and a folder's other files are never visited.
fn collect_grok(
    root: &Path,
    output: &mut Vec<(Agent, PathBuf)>,
    visited: &mut usize,
    limit: usize,
) -> Result<(), SessionCatalogError> {
    // Grok's own sweep may remove a folder between two listings.
    let entries = |directory: &Path| match fs::read_dir(directory) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SessionCatalogError::Io {
            operation: "read_directory",
            path: directory.to_path_buf(),
            source,
        }),
    };
    let visit = |visited: &mut usize| {
        *visited = visited.saturating_add(1);
        (*visited <= limit)
            .then_some(())
            .ok_or(SessionCatalogError::Capacity { limit })
    };
    let folders = |entries: fs::ReadDir, visited: &mut usize| {
        let mut folders = Vec::new();
        for entry in entries {
            visit(visited)?;
            let entry = entry.map_err(|source| SessionCatalogError::Io {
                operation: "read_entry",
                path: root.to_path_buf(),
                source,
            })?;
            // A link or a dot entry is no native session folder.
            if entry.file_type().is_ok_and(|kind| kind.is_dir())
                && !entry.file_name().to_string_lossy().starts_with('.')
            {
                folders.push(entry.path());
            }
        }
        Ok::<_, SessionCatalogError>(folders)
    };
    let Some(groups) = entries(root)? else {
        return Ok(());
    };
    for group in folders(groups, visited)? {
        let Some(sessions) = entries(&group)? else {
            continue;
        };
        for session in folders(sessions, visited)? {
            output.push((Agent::Grok, session.join(crate::grok::UPDATES)));
        }
    }
    Ok(())
}

fn session_cwd(agent: Agent, path: &Path) -> Option<PathBuf> {
    match agent {
        Agent::Grok | Agent::Pi | Agent::Omp | Agent::Antigravity => {
            crate::native_file::header(agent, path)
                .ok()
                .map(|header| header.cwd)
        }
        Agent::Codex => crate::codex_session_cwd(path).map(PathBuf::from),
        Agent::Claude => {
            let file = File::open(path).ok()?;
            let mut reader = BufReader::new(file);
            for _ in 0..128 {
                let line = match read_bounded_line(&mut reader, SESSION_LINE_LIMIT_BYTES) {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(_) => return None,
                };
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                    return Some(PathBuf::from(cwd));
                }
            }
            None
        }
        Agent::OpenCode | Agent::Cursor => None,
    }
}

/// A session of an agent other than Claude Code and Codex
/// ([`list_transcript`]), read whole on every listing.
fn read_project_session(
    home: &Path,
    agent: Agent,
    path: PathBuf,
    cwd: PathBuf,
) -> Option<ProjectSession> {
    if agent == Agent::Cursor {
        let header = crate::cursor::header(home, &path).ok()?;
        let scope = crate::SessionReadScope {
            id: header.id.clone(),
            cwd: cwd.to_str()?.to_owned(),
        };
        let read = crate::cursor::read_all(home, &path, &scope);
        let (events, availability) = match read {
            Ok(parsed) => (parsed.events, SessionAvailability::Available),
            Err(error) => (
                Vec::new(),
                SessionAvailability::Unavailable {
                    reason: error.to_string(),
                },
            ),
        };
        return Some(ProjectSession {
            id: header.id,
            agent,
            locator: path,
            checkout_path: cwd,
            first_human_request: first_human(&events).map(compact_snippet),
            started_at_unix_ms: events
                .first()
                .and_then(|event| (event.at_unix_ms != 0).then_some(event.at_unix_ms)),
            updated_at_unix_ms: header.updated_at.unwrap_or(header.created_at),
            title: None,
            event_count: events.len(),
            availability,
            stamp: None,
        });
    }
    let metadata = fs::metadata(&path).ok();
    let updated_at_unix_ms = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .and_then(system_time_ms)
        .unwrap_or_default();
    let id = session_id(agent, &path);
    if agent.requires_native_file_proof() && id.is_empty() {
        return None;
    }
    let pi_before = (agent.requires_native_file_proof())
        .then(|| crate::confirm_session_file(home, agent, &path, Some(&id), cwd.to_str()));
    let stamp = crate::stamp::stamp_at(&path);
    // When the session itself was made, where its file says so apart from
    // its records: a Pi or Grok fork copies its parent's records with their
    // own times, so its first record tells the parent's start, not its own.
    let mut opened_at_unix_ms = None;
    let (mut parsed, mut unavailable) = match (pi_before.as_ref(), metadata) {
        (Some(Err(reason)), _) => (None, Some(reason.to_string())),
        (_, Some(metadata)) if metadata.len() > SESSION_READ_LIMIT_BYTES => {
            (None, Some("session_too_large".to_owned()))
        }
        (_, Some(_)) => match read_bounded(&path, SESSION_READ_LIMIT_BYTES) {
            Ok(contents) => {
                let mut parsed = parse_events(agent, &contents);
                parsed.coalesce();
                opened_at_unix_ms = pi_header_time(agent, &contents);
                if agent == Agent::Grok {
                    // Grok keeps its current title and when the session was
                    // made beside the conversation.
                    let summary = crate::grok::summary(&path, &mut 0).ok();
                    opened_at_unix_ms = summary
                        .as_ref()
                        .and_then(|summary| summary.created_at_unix_ms);
                    (parsed.title, parsed.custom_title) =
                        summary.map(|summary| summary.title).unzip();
                }
                let unavailable = (parsed.events.is_empty() && parsed.skipped_lines > 0)
                    .then(|| "session_malformed".to_owned());
                (Some(parsed), unavailable)
            }
            Err(error) => (
                None,
                Some(if agent.requires_native_file_proof() {
                    "session_unreadable".to_owned()
                } else {
                    format!("session_unreadable:{error}")
                }),
            ),
        },
        (_, None) => (None, Some("session_missing".to_owned())),
    };
    if let Some(Ok(before)) = pi_before {
        let after = crate::confirm_session_file(home, agent, &path, Some(&id), cwd.to_str());
        if !after
            .map(|after| crate::label_owner::same_read(&path, &before, &after, stamp.as_deref()))
            .unwrap_or(false)
        {
            parsed = None;
            unavailable = Some("label_session_read_changed".to_owned());
        }
    }
    let events = parsed
        .as_ref()
        .map(|value| value.events.as_slice())
        .unwrap_or_default();
    Some(ProjectSession {
        id,
        agent,
        locator: path,
        checkout_path: cwd,
        first_human_request: first_human(events).map(compact_snippet),
        started_at_unix_ms: opened_at_unix_ms
            .filter(|_| parsed.is_some())
            .or_else(|| events.first().map(|event| event.at_unix_ms)),
        updated_at_unix_ms: events
            .last()
            .map(|event| event.at_unix_ms)
            .unwrap_or(updated_at_unix_ms),
        title: parsed.as_ref().and_then(|value| {
            value
                .custom_title
                .as_ref()
                .filter(|title| !title.trim().is_empty())
                .cloned()
                .or_else(|| value.title.clone().filter(|title| !title.trim().is_empty()))
        }),
        event_count: events.len(),
        availability: unavailable.map_or(SessionAvailability::Available, |reason| {
            SessionAvailability::Unavailable { reason }
        }),
        stamp: None,
    })
}

/// What a Claude or Codex transcript's records said so far, and where the
/// read that said it stopped. The read is the label and Memory readers' own
/// [`ConversationCursor`], so an appended file is read on from its
/// checkpoint, a file that got shorter or was replaced is read from its start
/// (the cursor's rule), and a torn last record is read again whole once its
/// rest is written. A listing that starts with nothing kept reads the same
/// way from byte zero, so a continued read lists what a cold one lists.
#[derive(Clone, Debug, Default)]
pub(crate) struct Transcript {
    checkpoint: ConversationCheckpoint,
    first_human_request: Option<String>,
    started_at_unix_ms: Option<u64>,
    last_event_at_unix_ms: Option<u64>,
    events: usize,
    skipped_lines: usize,
    title: Option<String>,
    custom_title: Option<String>,
}

impl Transcript {
    /// Adds one read's records. Claude Code and Codex write each message as
    /// one record, so no event of this read continues one of an earlier read.
    fn add(&mut self, parsed: ParsedSession) {
        if parsed.rescan_reason.is_some() {
            *self = Self::default();
        }
        debug_assert!(parsed.events.iter().all(|event| event.part().is_none()));
        if self.first_human_request.is_none() {
            self.first_human_request = first_human(&parsed.events).map(compact_snippet);
        }
        if let Some(first) = parsed.events.first() {
            self.started_at_unix_ms.get_or_insert(first.at_unix_ms);
        }
        if let Some(last) = parsed.events.last() {
            self.last_event_at_unix_ms = Some(last.at_unix_ms);
        }
        self.events += parsed.events.len();
        self.skipped_lines += parsed.skipped_lines;
        if parsed.title.is_some() {
            self.title = parsed.title;
        }
        if parsed.custom_title.is_some() {
            self.custom_title = parsed.custom_title;
        }
    }

    /// Reads `path` on from this transcript's checkpoint up to at least
    /// `length`, the size its stamp saw, one cursor window at a time; bytes
    /// written after that wait for the next listing, whose stamp moved.
    fn read_on(mut self, agent: Agent, path: &Path, length: u64) -> Result<Self, SessionError> {
        let mut cursor = ConversationCursor::restore(std::mem::take(&mut self.checkpoint));
        loop {
            let before = cursor.checkpoint().offset();
            self.add(cursor.read(agent, path)?);
            let after = cursor.checkpoint().offset();
            if !cursor.has_more() || after >= length || after == before {
                break;
            }
        }
        self.checkpoint = cursor.checkpoint();
        Ok(self)
    }
}

/// The time a Pi or omp session's header line says the session was made.
fn pi_header_time(agent: Agent, contents: &str) -> Option<u64> {
    if !matches!(agent, Agent::Pi | Agent::Omp) {
        return None;
    }
    let header: serde_json::Value = serde_json::from_str(contents.lines().next()?).ok()?;
    (header["type"] == "session")
        .then(|| crate::timestamp_ms(header.get("timestamp")).ok())
        .flatten()
}

/// The listed facts of a Claude or Codex transcript, read on from `kept`,
/// what its last listing read, and where this read stopped. A file past the
/// size cap is answered from its size alone, so nothing of it is kept.
fn list_transcript(
    agent: Agent,
    path: &Path,
    cwd: &Path,
    kept: Option<Transcript>,
) -> file_cache::Listed {
    let metadata = fs::metadata(path).ok();
    let modified_unix_ms = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .and_then(system_time_ms)
        .unwrap_or_default();
    let read = match &metadata {
        None => Err("session_missing".to_owned()),
        Some(metadata) if metadata.len() > SESSION_READ_LIMIT_BYTES => {
            Err("session_too_large".to_owned())
        }
        Some(metadata) => kept
            .unwrap_or_default()
            .read_on(agent, path, metadata.len())
            .map_err(|error| format!("session_unreadable:{error}")),
    };
    let transcript = read.as_ref().ok();
    let session = ProjectSession {
        id: session_id(agent, path),
        agent,
        locator: path.to_path_buf(),
        checkout_path: cwd.to_path_buf(),
        first_human_request: transcript.and_then(|read| read.first_human_request.clone()),
        started_at_unix_ms: transcript.and_then(|read| read.started_at_unix_ms),
        updated_at_unix_ms: transcript
            .and_then(|read| read.last_event_at_unix_ms)
            .unwrap_or(modified_unix_ms),
        title: transcript.and_then(|read| {
            read.custom_title
                .as_ref()
                .filter(|title| !title.trim().is_empty())
                .or(read.title.as_ref().filter(|title| !title.trim().is_empty()))
                .cloned()
        }),
        event_count: transcript.map_or(0, |read| read.events),
        availability: match &read {
            Ok(read) if read.events == 0 && read.skipped_lines > 0 => {
                SessionAvailability::Unavailable {
                    reason: "session_malformed".to_owned(),
                }
            }
            Ok(_) => SessionAvailability::Available,
            Err(reason) => SessionAvailability::Unavailable {
                reason: reason.clone(),
            },
        },
        stamp: None,
    };
    (session, read.ok())
}

fn first_human(events: &[ConversationEvent]) -> Option<&str> {
    events
        .iter()
        .find(|event| event.kind == EventKind::Human)
        .map(|event| event.text.as_str())
}

pub(crate) fn compact_snippet(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(160).collect()
}

fn session_id(agent: Agent, path: &Path) -> String {
    if agent.requires_native_file_proof() {
        // project_sessions already rejected invalid metadata, never synthesize
        // an identity from Pi's lossy pathname.
        return crate::native_file::header(agent, path)
            .map(|header| header.id)
            .unwrap_or_default();
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    if agent == Agent::Claude {
        return stem.to_owned();
    }
    let explicit = File::open(path).ok().and_then(|file| {
        let mut reader = BufReader::new(file);
        let line = read_bounded_line(&mut reader, SESSION_LINE_LIMIT_BYTES).ok()??;
        let value: Value = serde_json::from_slice(&line).ok()?;
        value
            .pointer("/payload/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    explicit.unwrap_or_else(|| {
        let digest = Sha256::digest(path.to_string_lossy().as_bytes());
        format!("codex:{:x}", digest)
    })
}

fn system_time_ms(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn mixed_provider_catalog_folds_linked_worktrees_and_keeps_order_through_filters() {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        fs::create_dir_all(home.join(".claude/projects/p")).unwrap();
        fs::create_dir_all(home.join(".codex/sessions/2026/09/21")).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        fs::write(
            home.join(".claude/projects/p/claude-1.jsonl"),
            format!("{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-09-21T01:00:00Z\",\"userType\":\"external\",\"promptId\":\"prompt-1\",\"message\":{{\"role\":\"user\",\"content\":\"alpha request\"}}}}\n", serde_json::to_string(&project_root).unwrap()),
        ).unwrap();
        fs::write(
            home.join(".codex/sessions/2026/09/21/rollout.jsonl"),
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-1\",\"cwd\":{}}}}}\n{{\"type\":\"response_item\",\"timestamp\":\"2026-09-21T02:00:00Z\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"beta request\"}}]}}}}\n", serde_json::to_string(&project_root).unwrap()),
        ).unwrap();
        let catalog = SessionCatalog::new(&home, "local");
        let sessions = catalog.project_sessions(&project).unwrap().sessions;
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex-1", "claude-1"]
        );
        assert_eq!(
            SessionCatalog::filtered(&sessions, SessionFilter::Claude, "alpha").len(),
            1
        );
        assert_eq!(
            sessions[1].first_human_request.as_deref(),
            Some("alpha request")
        );
        assert!(SessionCatalog::filtered(&sessions, SessionFilter::Codex, "alpha").is_empty());
    }

    #[test]
    fn an_unreadable_opencode_database_is_refused_once_and_leaves_the_other_agents_sessions_listed()
    {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        let database = home.join(".local/share/opencode/opencode.db");
        fs::create_dir_all(home.join(".claude/projects/p")).unwrap();
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        fs::write(
            home.join(".claude/projects/p/claude-1.jsonl"),
            format!("{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-09-21T01:00:00Z\",\"origin\":{{\"kind\":\"human\"}},\"message\":{{\"role\":\"user\",\"content\":\"alpha request\"}}}}\n", serde_json::to_string(&project_root).unwrap()),
        ).unwrap();
        let catalog = SessionCatalog::new(&home, "local");
        let read = |catalog: &SessionCatalog| {
            let read = catalog.project_sessions(&project).unwrap();
            let ids = read
                .sessions
                .iter()
                .map(|session| session.id.clone())
                .collect::<Vec<_>>();
            (ids, read.refusals)
        };

        // No OpenCode database is no OpenCode sessions, and nothing refused.
        assert_eq!(read(&catalog), (vec!["claude-1".to_owned()], Vec::new()));

        let refused = |reason: &str| {
            vec![SessionStoreRefusal {
                agent: Agent::OpenCode,
                reason: reason.to_owned(),
            }]
        };
        fs::write(&database, "not a database").unwrap();
        assert_eq!(
            read(&catalog),
            (
                vec!["claude-1".to_owned()],
                refused("opencode_db_unreadable")
            )
        );

        #[cfg(unix)]
        {
            fs::remove_file(&database).unwrap();
            let elsewhere = root.path().join("elsewhere.db");
            fs::write(&elsewhere, "").unwrap();
            std::os::unix::fs::symlink(&elsewhere, &database).unwrap();
            assert_eq!(
                read(&catalog),
                (vec!["claude-1".to_owned()], refused("label_session_linked"))
            );

            // A folder Hide may not look into is no proof the database is gone.
            use std::os::unix::fs::PermissionsExt;
            let folder = database.parent().unwrap();
            fs::set_permissions(folder, fs::Permissions::from_mode(0o000)).unwrap();
            let denied = read(&catalog);
            fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                denied,
                (
                    vec!["claude-1".to_owned()],
                    refused("opencode_db_unreadable")
                )
            );
        }
    }

    #[test]
    fn claude_catalog_skips_a_malformed_line_before_the_cwd_record() {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        let sessions_root = home.join(".claude/projects/p");
        fs::create_dir_all(&sessions_root).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        fs::write(
            sessions_root.join("claude-malformed.jsonl"),
            format!(
                "not-json\n{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-09-21T01:00:00Z\",\"origin\":{{\"kind\":\"human\"}},\"message\":{{\"content\":\"recover me\"}}}}\n",
                serde_json::to_string(&project_root).unwrap()
            ),
        )
        .unwrap();

        let sessions = SessionCatalog::new(&home, "local")
            .project_sessions(&project)
            .unwrap()
            .sessions;

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].first_human_request.as_deref(),
            Some("recover me")
        );
    }

    #[test]
    fn discovery_capacity_counts_every_directory_entry_not_only_sessions() {
        let root = tempdir().unwrap();
        for index in 0..4 {
            fs::write(
                root.path().join(format!("unrelated-{index}.txt")),
                "fixture",
            )
            .unwrap();
        }
        let mut output = Vec::new();
        let mut visited = 0;

        assert!(matches!(
            collect_jsonl(root.path(), Agent::Claude, 0, &mut output, &mut visited, 3,),
            Err(SessionCatalogError::Capacity { limit: 3 })
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn bounded_line_reader_refuses_before_allocating_past_the_limit() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"123456789\nnext\n"));
        let error = read_bounded_line(&mut reader, 8).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_claude_transcript_keeps_its_facts_until_its_stamp_moves() {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        let folder = home.join(".claude/projects/p");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        let file = folder.join("claude-cache.jsonl");
        let record = |content: &str| {
            format!(
                "{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-09-21T01:00:00Z\",\"userType\":\"external\",\"promptId\":\"prompt-1\",\"message\":{{\"role\":\"user\",\"content\":\"{content}\"}}}}\n",
                serde_json::to_string(&project_root).unwrap()
            )
        };
        fs::write(&file, record("alpha request")).unwrap();
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        let catalog = SessionCatalog::new(&home, "local");
        let first = catalog.project_sessions(&project).unwrap().sessions;
        assert_eq!(
            first[0].first_human_request.as_deref(),
            Some("alpha request")
        );

        // The same length and the same modification time keep the stamp, so
        // the transcript is not read again and its kept facts still answer.
        fs::write(&file, record("omega request")).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let kept = catalog.project_sessions(&project).unwrap().sessions;
        assert_eq!(
            kept[0].first_human_request.as_deref(),
            Some("alpha request")
        );

        // An appended record moves the length, so the file is read again and
        // the new record is counted.
        let mut appended = fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut appended, record("beta request").as_bytes()).unwrap();
        let grown = catalog.project_sessions(&project).unwrap().sessions;
        assert!(grown[0].event_count > first[0].event_count);
    }

    #[cfg(unix)]
    #[test]
    fn a_transcript_that_could_not_be_read_is_listed_once_it_can_be() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        fs::create_dir_all(home.join(".claude/projects/p")).unwrap();
        fs::create_dir_all(home.join(".codex/sessions/2026/09/21")).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        let cwd = serde_json::to_string(&project_root).unwrap();
        let claude = home.join(".claude/projects/p/claude-unreadable.jsonl");
        let codex = home.join(".codex/sessions/2026/09/21/rollout-unreadable.jsonl");
        fs::write(
            &claude,
            format!("{{\"type\":\"user\",\"cwd\":{cwd},\"timestamp\":\"2026-09-21T01:00:00Z\",\"userType\":\"external\",\"promptId\":\"prompt-1\",\"message\":{{\"role\":\"user\",\"content\":\"alpha request\"}}}}\n"),
        )
        .unwrap();
        fs::write(
            &codex,
            format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-unreadable\",\"cwd\":{cwd}}}}}\n{{\"type\":\"response_item\",\"timestamp\":\"2026-09-21T02:00:00Z\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"beta request\"}}]}}}}\n"),
        )
        .unwrap();
        let catalog = SessionCatalog::new(&home, "local");
        let set_mode = |mode: u32| {
            for path in [&claude, &codex] {
                fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            }
        };

        // Unreadable on this listing: nothing is listed, and nothing is kept.
        set_mode(0o000);
        let unreadable = catalog.project_sessions(&project).unwrap().sessions;
        assert!(unreadable.is_empty());
        assert_eq!(crate::file_cache::kept(&home), (0, 0));

        // Readable on the next: both transcripts are listed with their requests,
        // as a cold read answers them.
        set_mode(0o644);
        let readable = catalog.project_sessions(&project).unwrap().sessions;
        let mut requests = readable
            .iter()
            .map(|session| session.first_human_request.clone())
            .collect::<Vec<_>>();
        requests.sort();
        assert_eq!(
            requests,
            vec![
                Some("alpha request".to_owned()),
                Some("beta request".to_owned())
            ]
        );
    }

    #[test]
    fn a_listing_forgets_the_facts_of_a_file_it_no_longer_finds() {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let project_root = root.path().join("project");
        let folder = home.join(".claude/projects/p");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&project_root).unwrap();
        let project = hide_project::resolve(&project_root, "local").unwrap();
        let record = |content: &str| {
            format!(
                "{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-09-21T01:00:00Z\",\"userType\":\"external\",\"promptId\":\"prompt-1\",\"message\":{{\"role\":\"user\",\"content\":\"{content}\"}}}}\n",
                serde_json::to_string(&project_root).unwrap()
            )
        };
        let kept_file = folder.join("claude-kept.jsonl");
        let removed_file = folder.join("claude-removed.jsonl");
        fs::write(&kept_file, record("alpha request")).unwrap();
        fs::write(&removed_file, record("beta request")).unwrap();
        let catalog = SessionCatalog::new(&home, "local");
        assert_eq!(
            catalog.project_sessions(&project).unwrap().sessions.len(),
            2
        );
        assert_eq!(crate::file_cache::kept(&home), (2, 2));

        fs::remove_file(&removed_file).unwrap();
        let listed = catalog.project_sessions(&project).unwrap().sessions;
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].first_human_request.as_deref(),
            Some("alpha request")
        );
        assert_eq!(crate::file_cache::kept(&home), (1, 1));
    }

    /// A home with one project, whose Claude and Codex transcripts a test
    /// writes and lists; a cold listing forgets what earlier ones kept.
    struct Transcripts {
        _root: tempfile::TempDir,
        home: PathBuf,
        cwd: String,
        project: ProjectIdentity,
    }

    const ONE_AM: u64 = 1_789_952_400_000;
    const HOUR: u64 = 3_600_000;

    impl Transcripts {
        fn new() -> Self {
            let root = tempdir().unwrap();
            let home = root.path().join("home");
            let project_root = root.path().join("project");
            fs::create_dir_all(home.join(".claude/projects/p")).unwrap();
            fs::create_dir_all(home.join(".codex/sessions/2026/09/21")).unwrap();
            fs::create_dir_all(&project_root).unwrap();
            let project = hide_project::resolve(&project_root, "local").unwrap();
            Self {
                cwd: serde_json::to_string(&project_root).unwrap(),
                _root: root,
                home,
                project,
            }
        }

        fn claude_file(&self) -> PathBuf {
            self.home.join(".claude/projects/p/claude-grows.jsonl")
        }

        fn codex_file(&self) -> PathBuf {
            self.home
                .join(".codex/sessions/2026/09/21/rollout-grows.jsonl")
        }

        /// A Claude Code record of the operator's (`user`) or the agent's,
        /// written at `hour` o'clock.
        fn claude(&self, role: &str, hour: u64, text: &str) -> String {
            format!(
                "{{\"type\":\"{role}\",\"cwd\":{},\"timestamp\":\"2026-09-21T0{hour}:00:00Z\",\"userType\":\"external\",\"promptId\":\"prompt-{hour}\",\"message\":{{\"role\":\"{role}\",\"content\":{}}}}}\n",
                self.cwd,
                serde_json::to_string(text).unwrap()
            )
        }

        fn codex_meta(&self) -> String {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-grows\",\"cwd\":{}}}}}\n",
                self.cwd
            )
        }

        fn codex(&self, role: &str, hour: u64, text: &str) -> String {
            let kind = if role == "user" {
                "input_text"
            } else {
                "output_text"
            };
            format!(
                "{{\"type\":\"response_item\",\"timestamp\":\"2026-09-21T0{hour}:00:00Z\",\"payload\":{{\"type\":\"message\",\"role\":\"{role}\",\"content\":[{{\"type\":\"{kind}\",\"text\":{}}}]}}}}\n",
                serde_json::to_string(text).unwrap()
            )
        }

        fn append(path: &Path, text: &str) {
            let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
            std::io::Write::write_all(&mut file, text.as_bytes()).unwrap();
        }

        fn list(&self) -> Vec<ProjectSession> {
            SessionCatalog::new(&self.home, "local")
                .project_sessions(&self.project)
                .unwrap()
                .sessions
        }

        /// The listing a process that never listed this home would make.
        fn cold(&self) -> Vec<ProjectSession> {
            file_cache::retain(&self.home, &HashSet::new());
            assert_eq!(file_cache::kept(&self.home), (0, 0));
            self.list()
        }

        /// The listing after a change, checked against a cold one.
        fn list_as_cold(&self) -> Vec<ProjectSession> {
            let warm = self.list();
            assert_eq!(warm, self.cold());
            warm
        }
    }

    /// What the Sessions list shows of one session: its request, how many
    /// events, its last activity and its title.
    type Shown<'a> = (Agent, Option<&'a str>, usize, u64, Option<&'a str>);

    fn shown(sessions: &[ProjectSession]) -> Vec<Shown<'_>> {
        sessions
            .iter()
            .map(|session| {
                (
                    session.agent,
                    session.first_human_request.as_deref(),
                    session.event_count,
                    session.updated_at_unix_ms,
                    session.title.as_deref(),
                )
            })
            .collect()
    }

    #[test]
    fn an_appended_transcript_lists_what_a_cold_read_lists() {
        let home = Transcripts::new();
        let (claude, codex) = (home.claude_file(), home.codex_file());
        fs::write(&claude, home.claude("user", 1, "alpha request")).unwrap();
        fs::write(
            &codex,
            home.codex_meta() + &home.codex("user", 2, "beta request"),
        )
        .unwrap();
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![
                (Agent::Codex, Some("beta request"), 1, ONE_AM + HOUR, None),
                (Agent::Claude, Some("alpha request"), 1, ONE_AM, None),
            ]
        );

        Transcripts::append(
            &claude,
            &(home.claude("assistant", 3, "done")
                + "{\"type\":\"ai-title\",\"aiTitle\":\"Alpha work\"}\n"
                + &home.claude("user", 4, "gamma request")),
        );
        Transcripts::append(&codex, &home.codex("assistant", 3, "answered"));
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![
                (
                    Agent::Claude,
                    Some("alpha request"),
                    3,
                    ONE_AM + 3 * HOUR,
                    Some("Alpha work")
                ),
                (
                    Agent::Codex,
                    Some("beta request"),
                    2,
                    ONE_AM + 2 * HOUR,
                    None
                ),
            ]
        );
    }

    #[test]
    fn a_shorter_or_replaced_transcript_is_listed_from_its_start() {
        let home = Transcripts::new();
        let claude = home.claude_file();
        fs::write(
            &claude,
            home.claude("user", 1, "alpha request") + &home.claude("user", 2, "beta request"),
        )
        .unwrap();
        assert_eq!(shown(&home.list())[0].2, 2);

        // Cut short in place: the same file, fewer bytes than were read.
        fs::write(&claude, home.claude("user", 3, "gamma")).unwrap();
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(Agent::Claude, Some("gamma"), 1, ONE_AM + 2 * HOUR, None)]
        );

        // Replaced by another file at the same path, longer than the first.
        let next = claude.with_extension("next");
        fs::write(
            &next,
            home.claude("user", 4, "delta request") + &home.claude("assistant", 4, "ok"),
        )
        .unwrap();
        fs::rename(&next, &claude).unwrap();
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(
                Agent::Claude,
                Some("delta request"),
                2,
                ONE_AM + 3 * HOUR,
                None
            )]
        );
    }

    #[test]
    fn a_transcript_replaced_by_one_of_the_same_size_is_listed_from_its_start() {
        let home = Transcripts::new();
        let claude = home.claude_file();
        fs::write(&claude, home.claude("user", 1, "alpha request")).unwrap();
        assert_eq!(shown(&home.list())[0].1, Some("alpha request"));

        // Another file renamed over it, byte for byte as long: only its
        // identity says it is not the file that was read.
        let next = claude.with_extension("next");
        fs::write(&next, home.claude("user", 1, "gamma request")).unwrap();
        assert_eq!(
            fs::metadata(&next).unwrap().len(),
            fs::metadata(&claude).unwrap().len()
        );
        fs::rename(&next, &claude).unwrap();
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(Agent::Claude, Some("gamma request"), 1, ONE_AM, None)]
        );
    }

    #[test]
    fn a_record_longer_than_one_read_window_is_read_past() {
        let home = Transcripts::new();
        let claude = home.claude_file();
        let long = home.claude(
            "assistant",
            2,
            &"x".repeat(crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize + 4096),
        );
        assert!(long.len() as u64 > crate::SESSION_INCREMENT_READ_LIMIT_BYTES);
        fs::write(&claude, home.claude("user", 1, "alpha request") + &long).unwrap();
        let listed = home.list_as_cold();
        assert_eq!(shown(&listed)[0].1, Some("alpha request"));

        // The record after it is read on from where the long one ended.
        Transcripts::append(&claude, &home.claude("user", 3, "beta request"));
        let grown = home.list_as_cold();
        assert_eq!(grown[0].event_count, listed[0].event_count + 1);
        assert_eq!(grown[0].updated_at_unix_ms, ONE_AM + 2 * HOUR);
    }

    #[test]
    fn a_record_appended_in_two_writes_is_counted_once_it_is_whole() {
        let home = Transcripts::new();
        let codex = home.codex_file();
        let record = home.codex("user", 2, "second request");
        let (head, tail) = record.split_at(record.len() / 2);
        fs::write(
            &codex,
            home.codex_meta() + &home.codex("user", 1, "first request") + head,
        )
        .unwrap();
        // The torn record is not a session event yet.
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(Agent::Codex, Some("first request"), 1, ONE_AM, None)]
        );

        Transcripts::append(&codex, tail);
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(Agent::Codex, Some("first request"), 2, ONE_AM + HOUR, None)]
        );
    }

    #[test]
    fn a_transcript_longer_than_one_read_window_is_listed_whole() {
        let home = Transcripts::new();
        let claude = home.claude_file();
        let filler = "x".repeat(1000);
        let records = (0..1500)
            .map(|_| home.claude("assistant", 2, &filler))
            .collect::<String>();
        assert!(records.len() as u64 > crate::SESSION_INCREMENT_READ_LIMIT_BYTES);
        fs::write(&claude, home.claude("user", 1, "alpha request") + &records).unwrap();
        assert_eq!(
            shown(&home.list_as_cold()),
            vec![(
                Agent::Claude,
                Some("alpha request"),
                1501,
                ONE_AM + HOUR,
                None
            )]
        );
        Transcripts::append(&claude, &records);
        assert_eq!(shown(&home.list_as_cold())[0].2, 3001);
    }

    #[test]
    fn an_appended_transcript_is_read_on_from_where_its_last_read_stopped() {
        let home = Transcripts::new();
        let claude = home.claude_file();
        let first = home.claude("user", 1, "alpha request");
        fs::write(&claude, &first).unwrap();
        assert_eq!(shown(&home.list())[0].1, Some("alpha request"));

        // Bytes already read are not read again: overwriting them in place,
        // which no agent does, changes nothing a continued listing shows,
        // while the record appended after them is counted.
        let mut file = fs::OpenOptions::new().write(true).open(&claude).unwrap();
        std::io::Write::write_all(&mut file, &vec![b'x'; first.len() - 1]).unwrap();
        drop(file);
        Transcripts::append(&claude, &home.claude("user", 2, "beta request"));
        assert_eq!(
            shown(&home.list()),
            vec![(Agent::Claude, Some("alpha request"), 2, ONE_AM + HOUR, None)]
        );
    }
}
