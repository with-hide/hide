//! Locate and read local Claude Code and Codex session JSONL files, and
//! OpenCode's session database.
//!
//! The crate has three deliberately separate responsibilities:
//!
//! - [`SessionLocator`] resolves a reported session identity or a working
//!   directory to a local file.
//! - [`ConversationCursor`] reads only bytes appended since its previous read,
//!   reports replacement and truncation explicitly, and reads past a record
//!   longer than the line cap without holding it.
//! - The conversation parsers turn provider-specific records into a small,
//!   common event stream.
//! - [`session_activity`] reuses native ownership proof and returns only
//!   modification time and size for inactivity watches.
//!
//! No provider response or session body is logged by this crate. Callers can
//! use the public raw-line helpers for records outside the conversation event
//! model, such as Codex usage data.

use anyhow::Error as AnyhowError;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub mod antigravity;
mod catalog;
mod claude_attachment;
mod conversation_cursor;
pub mod cursor;
mod envelope;
pub mod environment;
mod grok;
mod label_owner;
pub mod label_transcript;
pub mod links;
mod native_file;
pub mod opencode;
mod refusal;
pub mod session_activity;
mod sightings;
mod sqlite;
pub mod stamp;
pub mod turns;
pub mod usage;

pub use envelope::{envelope_from_operator, envelope_header, envelope_sender};
pub use refusal::ReadRefusal;
pub use sightings::{
    MAX_SIGHTINGS_PER_OUTPUT, PrSighting, issue_addresses, pull_request_addresses,
};

pub use label_owner::{
    ConfirmedLabelSession, SessionReadScope, confirm_label_session, confirm_session_file,
    label_reference_token, read_session_file, valid_native_id,
};

pub use conversation_cursor::{
    ConversationCheckpoint, ConversationCursor, ConversationRead, read_conversation,
};

pub use catalog::{
    ProjectSession, ProjectSessions, SESSION_DISCOVERY_LIMIT, SessionAvailability, SessionCatalog,
    SessionCatalogError, SessionFilter, SessionStoreRefusal,
};

#[cfg(not(unix))]
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Number of recent day directories used when Herdr has not reported a Codex
/// session identity yet.
pub const CODEX_FALLBACK_DAYS: usize = 7;
/// Number of newest files considered by the usage fallback.
pub const CODEX_CANDIDATE_LIMIT: usize = 32;
/// Total directory visits and retained path bytes in one session lookup.
const SESSION_LOOKUP_DIRECTORY_LIMIT: usize = 64;
const SESSION_LOOKUP_PATH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct DiscoveryBudget {
    entries: usize,
    directories: usize,
    path_bytes: usize,
}

impl DiscoveryBudget {
    fn directory(&mut self) -> Result<()> {
        if self.directories >= SESSION_LOOKUP_DIRECTORY_LIMIT {
            return Err(SessionError::Capacity {
                resource: "discovery_directories",
                limit: SESSION_LOOKUP_DIRECTORY_LIMIT as u64,
            });
        }
        self.directories += 1;
        Ok(())
    }

    fn entry(&mut self) -> Result<()> {
        if self.entries >= SESSION_DISCOVERY_LIMIT {
            return Err(SessionError::Capacity {
                resource: "discovery_entries",
                limit: SESSION_DISCOVERY_LIMIT as u64,
            });
        }
        self.entries += 1;
        Ok(())
    }

    fn path(&mut self, path: &Path) -> Result<()> {
        let bytes = path.as_os_str().as_encoded_bytes().len();
        if bytes > SESSION_LOOKUP_PATH_BYTES.saturating_sub(self.path_bytes) {
            return Err(SessionError::Capacity {
                resource: "discovery_path_bytes",
                limit: SESSION_LOOKUP_PATH_BYTES as u64,
            });
        }
        self.path_bytes += bytes;
        Ok(())
    }
}
/// Largest complete session file parsed by archive and detail views.
pub const SESSION_READ_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
/// Largest append chunk consumed by one incremental projection poll.
pub const SESSION_INCREMENT_READ_LIMIT_BYTES: u64 = 1024 * 1024;
/// Largest individual JSONL record retained or parsed by a cursor.
pub const SESSION_LINE_LIMIT_BYTES: usize = 256 * 1024;

/// Claude Code's transcript folder, under the home folder.
pub const CLAUDE_SESSIONS: &str = ".claude/projects";
/// Codex CLI's transcript folder, under the home folder.
pub const CODEX_SESSIONS: &str = ".codex/sessions";
/// Pi's default native session root. Custom session directories grant no trust.
pub const PI_SESSIONS: &str = ".pi/agent/sessions";
/// Grok's default native root; a `GROK_HOME` elsewhere grants no trust.
pub const GROK_SESSIONS: &str = ".grok/sessions";
/// omp's default native root; profiles and custom session roots grant no trust.
pub const OMP_SESSIONS: &str = ".omp/agent/sessions";

/// The folder in `home` that holds `agent`'s session files, `None` for an
/// agent that keeps no file per session.
pub fn session_root(home: &Path, agent: Agent) -> Option<PathBuf> {
    match agent {
        Agent::Claude => Some(home.join(CLAUDE_SESSIONS)),
        Agent::Codex => Some(home.join(CODEX_SESSIONS)),
        Agent::Grok => Some(home.join(GROK_SESSIONS)),
        Agent::Pi => Some(home.join(PI_SESSIONS)),
        Agent::Omp => Some(home.join(OMP_SESSIONS)),
        Agent::Cursor => Some(home.join(cursor::SESSIONS)),
        Agent::Antigravity => Some(home.join(antigravity::SESSIONS)),
        Agent::OpenCode => None,
    }
}

/// Why [`inside_session_root`] refused a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootRefusal {
    /// None of the agents keeps a file per session.
    Unsupported,
    /// The path resolves outside every agent's session folder.
    Outside,
    /// Nothing is at the path.
    Missing,
    /// The path could not be resolved.
    Unreadable,
}

/// `path` with every link resolved, refused unless it lies under one of
/// `agents`' session folders in `home`: a path a caller names, or a link
/// planted inside a folder, never makes a reader open a file elsewhere.
pub fn inside_session_root(
    home: &Path,
    agents: &[Agent],
    path: &Path,
) -> std::result::Result<PathBuf, RootRefusal> {
    for &agent in agents
        .iter()
        .filter(|agent| agent.requires_native_file_proof())
    {
        let checked = native_file::inside_root(home, agent, path);
        if agents.len() == 1 || checked.is_ok() {
            return checked;
        }
    }
    let roots = agents
        .iter()
        .filter(|agent| !agent.requires_native_file_proof())
        .filter_map(|agent| session_root(home, *agent))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(
            if agents
                .iter()
                .any(|agent| agent.requires_native_file_proof())
            {
                RootRefusal::Outside
            } else {
                RootRefusal::Unsupported
            },
        );
    }
    let roots = roots
        .iter()
        .filter_map(|root| hide_platform::fs::identity::canonical(root).ok())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Err(RootRefusal::Outside);
    }
    let path = hide_platform::fs::identity::canonical(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            RootRefusal::Missing
        } else {
            RootRefusal::Unreadable
        }
    })?;
    if roots
        .iter()
        .any(|root| path.starts_with(root) && &path != root)
    {
        Ok(path)
    } else {
        Err(RootRefusal::Outside)
    }
}

/// Opens a session file for reading without waiting on it: a pipe or a
/// device named where a session file should be is refused at once instead of
/// blocking the reader, and so is a folder.
pub(crate) fn open_session_file(path: &Path) -> io::Result<File> {
    open_session_file_with_links(path, true)
}

pub(crate) fn open_session_file_nofollow(path: &Path) -> io::Result<File> {
    open_session_file_with_links(path, false)
}

fn open_session_file_with_links(path: &Path, follow: bool) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
    }
    let file = options.open(path)?;
    if !follow
        && hide_platform::fs::identity::file_id_nofollow(path)?
            != hide_platform::fs::identity::file_id_of(&file)?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session source is linked.",
        ));
    }
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session source is not a regular file.",
        ));
    }
    Ok(file)
}

/// The local agent session formats supported by Hide. Claude Code and Codex
/// keep a JSONL file per session; OpenCode keeps every session in one SQLite
/// database, which only the session adapter reads (`label_transcript`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Codex,
    Claude,
    Grok,
    Pi,
    Omp,
    Cursor,
    #[serde(rename = "opencode")]
    OpenCode,
    Antigravity,
}

impl Agent {
    /// The session formats actually declared by this build, independent of
    /// whether its launch, hooks or other features are enabled.
    pub fn supported() -> impl Iterator<Item = Self> {
        hide_agent_adapter::ADAPTERS
            .iter()
            .filter_map(|row| row.session.map(Self::from_format))
    }

    pub const fn format(self) -> hide_agent_adapter::SessionFormat {
        match self {
            Self::Claude => hide_agent_adapter::SessionFormat::Claude,
            Self::Codex => hide_agent_adapter::SessionFormat::Codex,
            Self::Grok => hide_agent_adapter::SessionFormat::Grok,
            Self::Pi => hide_agent_adapter::SessionFormat::Pi,
            Self::Omp => hide_agent_adapter::SessionFormat::Omp,
            Self::Cursor => hide_agent_adapter::SessionFormat::Cursor,
            Self::OpenCode => hide_agent_adapter::SessionFormat::OpenCode,
            Self::Antigravity => hide_agent_adapter::SessionFormat::Antigravity,
        }
    }

    pub const fn from_format(format: hide_agent_adapter::SessionFormat) -> Self {
        match format {
            hide_agent_adapter::SessionFormat::Claude => Self::Claude,
            hide_agent_adapter::SessionFormat::Codex => Self::Codex,
            hide_agent_adapter::SessionFormat::Grok => Self::Grok,
            hide_agent_adapter::SessionFormat::Pi => Self::Pi,
            hide_agent_adapter::SessionFormat::Omp => Self::Omp,
            hide_agent_adapter::SessionFormat::Cursor => Self::Cursor,
            hide_agent_adapter::SessionFormat::OpenCode => Self::OpenCode,
            hide_agent_adapter::SessionFormat::Antigravity => Self::Antigravity,
        }
    }

    pub fn from_kind(kind: &str) -> Option<Self> {
        hide_agent_adapter::adapter(kind)?
            .session
            .map(Self::from_format)
    }

    pub const fn as_str(self) -> &'static str {
        self.format().adapter().herdr.name
    }

    pub const fn reports_turns(self) -> bool {
        self.format().reports_turns()
    }
    pub const fn waits_while_working(self) -> bool {
        self.format().waits_while_working()
    }
    pub const fn reports_wake_devices(self) -> bool {
        self.format().reports_wake_devices()
    }
    pub const fn has_session_file(self) -> bool {
        self.format().has_session_file()
    }
    pub const fn requires_native_proof(self) -> bool {
        self.format().requires_native_proof()
    }
    pub const fn proof_reference_kind(self) -> &'static str {
        self.format().proof_reference_kind()
    }
    pub const fn reports_activity(self) -> bool {
        self.format().reports_activity()
    }
    pub const fn is_jsonl(self) -> bool {
        self.format().is_jsonl()
    }
    pub const fn requires_native_file_proof(self) -> bool {
        self.format().requires_native_file_proof()
    }
}

/// A session identity reported by Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdentity {
    Id(String),
    Path(PathBuf),
}

impl SessionIdentity {
    pub fn id(value: impl Into<String>) -> Self {
        Self::Id(value.into())
    }

    pub fn path(value: impl Into<PathBuf>) -> Self {
        Self::Path(value.into())
    }
}

/// The normalized kind of a conversation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Human,
    Injected,
    Interrupted,
    Assistant,
}

impl EventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Injected => "injected",
            Self::Interrupted => "interrupted",
            Self::Assistant => "assistant",
        }
    }
}

/// A provider-neutral conversation event.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConversationEvent {
    pub role: &'static str,
    pub kind: EventKind,
    /// True only when provider-owned transcript metadata identifies injected
    /// context. Text prefixes can classify display semantics but never set
    /// this trust bit.
    provider_injected: bool,
    pub at_unix_ms: u64,
    pub text: String,
    /// Images attached to the message; their bytes never enter `text`, and
    /// a message of images alone has empty `text`.
    pub images: u32,
    /// The native message this record is a part of, for a format that
    /// writes one message as several records (Grok's prompt blocks and an
    /// answer's text runs); consecutive parts become one event.
    part: Option<String>,
}

#[derive(serde::Deserialize)]
struct OwnedConversationEvent {
    role: String,
    kind: EventKind,
    provider_injected: bool,
    at_unix_ms: u64,
    text: String,
    images: u32,
    part: Option<String>,
}

impl TryFrom<OwnedConversationEvent> for ConversationEvent {
    type Error = &'static str;
    fn try_from(event: OwnedConversationEvent) -> std::result::Result<Self, Self::Error> {
        let role = match event.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            "system" => "system",
            _ => return Err("conversation_role_invalid"),
        };
        Ok(Self {
            role,
            kind: event.kind,
            provider_injected: event.provider_injected,
            at_unix_ms: event.at_unix_ms,
            text: event.text,
            images: event.images,
            part: event.part,
        })
    }
}

impl<'de> serde::Deserialize<'de> for ConversationEvent {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        OwnedConversationEvent::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl ConversationEvent {
    pub fn new(
        role: &'static str,
        kind: EventKind,
        at_unix_ms: u64,
        text: impl Into<String>,
    ) -> Self {
        Self {
            role,
            kind,
            provider_injected: false,
            at_unix_ms,
            text: text.into(),
            images: 0,
            part: None,
        }
    }

    fn with_images(mut self, images: u32) -> Self {
        self.images = images;
        self
    }

    fn with_part(mut self, part: Option<String>) -> Self {
        self.part = part;
        self
    }

    /// The native message this record is a part of, when the format
    /// splits messages; records of one message share it.
    pub fn part(&self) -> Option<&str> {
        self.part.as_deref()
    }

    fn continued_by(&self, next: &Self) -> bool {
        self.part.is_some() && self.part == next.part && self.kind == next.kind
    }

    fn absorb(&mut self, next: Self) {
        if !next.text.is_empty() {
            if !self.text.is_empty() {
                self.text.push_str(if self.kind == EventKind::Assistant {
                    "\n\n"
                } else {
                    "\n"
                });
            }
            self.text.push_str(&next.text);
        }
        self.images += next.images;
        self.provider_injected |= next.provider_injected;
    }

    pub const fn is_provider_injected(&self) -> bool {
        self.provider_injected
    }

    fn with_provider_injected(mut self, provider_injected: bool) -> Self {
        self.provider_injected = provider_injected;
        self
    }
}

/// Why one otherwise relevant JSONL line was not turned into an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum SkipReason {
    MalformedJson,
    MissingTimestamp,
    InvalidTimestamp,
    NonConversationCapacity,
    /// A record past the line cap whose text, or whose structure, the
    /// bounded scan cannot keep; its text is lost.
    ConversationCapacity,
    /// A record longer than the line cap was read without its bodies.
    BodyCapacity,
    UserTurnCapacity,
    UserTurnInvalid,
}

impl SkipReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedJson => "malformed_json",
            Self::MissingTimestamp => "missing_timestamp",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::NonConversationCapacity => "non_conversation_capacity",
            Self::ConversationCapacity => "conversation_capacity",
            Self::BodyCapacity => "body_capacity",
            Self::UserTurnCapacity => "user_turn_capacity",
            Self::UserTurnInvalid => "user_turn_invalid",
        }
    }
}

/// Why a cursor had to reread a file from byte zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanReason {
    Truncated,
    Replaced,
}

impl RescanReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Truncated => "truncated",
            Self::Replaced => "replaced",
        }
    }
}

/// The result of parsing one complete JSONL chunk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSession {
    pub events: Vec<ConversationEvent>,
    /// Absolute byte offset of the JSONL record that produced each event.
    ///
    /// This stays parallel to `events` so archive callers that only need the
    /// normalized conversation do not have to carry source-location state.
    pub event_offsets: Vec<u64>,
    /// The name the agent gave its own session, when the chunk carried one.
    ///
    /// Claude Code writes an `ai-title` record once it has named the
    /// conversation and repeats it on later turns; the last one in the
    /// chunk wins. It is a property of the session rather than an event, so
    /// it never enters `events`. A native name with no manual/automatic
    /// provenance (Pi's `session_info.name`) also belongs here.
    /// Codex has no such rollout record and leaves `None`.
    pub title: Option<String>,
    /// The name the operator gave the session (Claude Code's `/rename`, a
    /// `custom-title` record), which wins over `title`.
    pub custom_title: Option<String>,
    /// Pull request addresses the session's tools printed, in record order
    /// (`sightings`).
    pub pr_sightings: Vec<PrSighting>,
    pub issue_sightings: Vec<PrSighting>,
    /// What the chunk says about links: branches, pull request addresses,
    /// the run kind and continuation pointers (`links`).
    pub links: links::LinkFacts,
    /// The turn records of an agent that reports them, each with its record's
    /// offset, in record order (`turns`); empty for every other agent.
    pub turn_marks: Vec<(u64, turns::TurnMark)>,
    /// A plan awaiting approval that the session's current state, not one
    /// of its records, says is pending (Grok's `plan_mode.json`).
    pub plan_hold: Option<turns::UserTurnContent>,
    pub skipped_lines: usize,
    pub skipped_reasons: BTreeMap<SkipReason, usize>,
    pub rescan_reason: Option<RescanReason>,
    /// The read stopped at a Claude command record whose reply is not in the
    /// file yet (`parse_events_followed`); the record and what follows it are
    /// not in this result.
    pub(crate) held_command: bool,
}

impl ParsedSession {
    fn skipped(&mut self, reason: SkipReason) {
        self.skipped_lines += 1;
        *self.skipped_reasons.entry(reason).or_default() += 1;
    }

    /// Join the consecutive parts of one native message, keeping the first
    /// part's offset and time. A reader that pages or polls by record
    /// offset joins its own boundaries instead (`ConversationEvent::part`).
    pub fn coalesce(&mut self) {
        let events = std::mem::take(&mut self.events);
        let offsets = std::mem::take(&mut self.event_offsets);
        for (event, offset) in events.into_iter().zip(offsets) {
            match self.events.last_mut() {
                Some(last) if last.continued_by(&event) => last.absorb(event),
                _ => {
                    self.events.push(event);
                    self.event_offsets.push(offset);
                }
            }
        }
    }
}

/// Errors that prevent a caller from obtaining or reading a session file.
///
/// A malformed conversation line is intentionally not an error: it is counted
/// in [`ParsedSession`], allowing the rest of a session to remain useful.
#[derive(Debug)]
pub enum SessionError {
    CwdUnavailable,
    SessionFileMissing,
    UnsupportedSessionKind,
    Checkpoint(String),
    /// A proof read met the agent writing its records
    /// ([`ReadRefusal::InMotion`]); reading again can answer.
    InMotion(&'static str),
    Capacity {
        resource: &'static str,
        limit: u64,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: AnyhowError,
    },
}

impl SessionError {
    /// A proof `error` refused: still in motion when it only met the agent
    /// writing, the reason it gave otherwise.
    pub(crate) fn unproven(error: &anyhow::Error) -> Self {
        match ReadRefusal::of(error) {
            ReadRefusal::InMotion(reason) => Self::InMotion(reason),
            ReadRefusal::Refused(_) => Self::Checkpoint(error.to_string()),
        }
    }

    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source: AnyhowError::new(source),
        }
    }
}

impl Display for SessionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CwdUnavailable => formatter.write_str("session_cwd_unavailable"),
            Self::SessionFileMissing => formatter.write_str("session_file_missing"),
            Self::UnsupportedSessionKind => formatter.write_str("session_kind_unsupported"),
            Self::Checkpoint(reason) => write!(formatter, "session_checkpoint_invalid:{reason}"),
            Self::InMotion(reason) => formatter.write_str(reason),
            Self::Capacity { resource, limit } => {
                write!(formatter, "session_capacity:{resource}:{limit}")
            }
            Self::Io {
                operation, source, ..
            } => write!(formatter, "session_{operation}: {source}"),
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, SessionError>;

/// A file identity used to detect an atomic replacement at the same path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct FileIdentity {
    first: u64,
    second: u64,
}

/// Durable state for resuming one append-only session file after relaunch.
///
/// The file identity is intentionally retained. An offset without the inode
/// would silently skip the beginning of an atomic replacement at the same
/// path.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CursorCheckpoint {
    pub offset: u64,
    identity: Option<FileIdentity>,
    /// Accepted only to migrate checkpoints written before Project Memory
    /// stopped persisting torn transcript bytes. New checkpoints always keep
    /// this empty and rewind `offset` to the start of the torn line instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<u8>,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        {
            Self {
                first: metadata.dev(),
                second: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_nanos() as u64);
            Self {
                first: metadata.len(),
                second: modified,
            }
        }
    }
}

/// The byte position of one session file that [`ConversationCursor`] reads
/// from.
///
/// The cursor retains an unterminated final line instead of discarding it.
/// This makes a later append complete the same JSON object without rereading
/// the preceding file.
#[derive(Debug, Default)]
pub struct SessionCursor {
    offset: u64,
    identity: Option<FileIdentity>,
    pending: Vec<u8>,
}

struct AppendedBytes {
    contents: Vec<u8>,
    start_offset: u64,
    identity: FileIdentity,
    rescan_reason: Option<RescanReason>,
    has_more: bool,
}

impl SessionCursor {
    pub fn checkpoint(&self) -> CursorCheckpoint {
        CursorCheckpoint {
            offset: self.offset.saturating_sub(self.pending.len() as u64),
            identity: self.identity,
            pending: Vec::new(),
        }
    }

    pub fn restore(checkpoint: CursorCheckpoint) -> Self {
        Self {
            // Old checkpoints included raw torn-line bytes after the stored
            // offset. Rewind once, discard those bytes, and reread them from
            // the provider-owned session file. New checkpoints have no
            // `pending` field and therefore restore at their exact offset.
            offset: checkpoint
                .offset
                .saturating_sub(checkpoint.pending.len() as u64),
            identity: checkpoint.identity,
            pending: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        self.identity = None;
        self.pending.clear();
    }
}

/// Resolve Herdr's session identity and working directory to local files.
pub struct SessionLocator {
    home: PathBuf,
    resolved: HashMap<String, PathBuf>,
}

impl SessionLocator {
    /// Release a retired caller's cached discovery result.
    pub fn forget(&mut self, pane_id: &str) {
        self.resolved.remove(pane_id);
    }

    /// Keep discovery caches bounded by their live callers.
    pub fn retain_callers(&mut self, live: impl Fn(&str) -> bool) {
        self.resolved.retain(|id, _| live(id));
    }

    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            resolved: HashMap::new(),
        }
    }

    pub fn locate(
        &mut self,
        pane_id: &str,
        agent: Agent,
        identity: Option<&SessionIdentity>,
        cwd: Option<&str>,
    ) -> Result<PathBuf> {
        // Pi's lossy folder and file names are never native ownership proof.
        if agent.requires_native_file_proof() {
            return native_file::locate(
                &self.home,
                agent,
                identity,
                cwd,
                &mut DiscoveryBudget::default(),
            );
        }
        // A session Herdr reported for this pane is that pane's session, full
        // stop, and a reported session whose file does not exist yet is a
        // session that has not started, not a reason to read another one.
        // Both fallbacks to the newest file in the cwd were tried and both
        // handed a pane its neighbour's transcript: the first once the
        // reported file went quiet for 30 s, the second in the seconds
        // between Claude's launch and its first written turn, when the id is
        // already reported and the file is not there yet (2026-09-18/19). A
        // pane that starts a new session in place (`/clear`, `--resume`) is
        // reported again by the runtime hook, so a switch reaches here as a
        // new id. The cwd search below serves only a pane with no reported
        // session at all.
        let mut budget = DiscoveryBudget::default();
        if let Some(identity) = identity {
            let path = self.reported_path(agent, cwd, identity, &mut budget)?;
            self.resolved.insert(pane_id.to_owned(), path.clone());
            return Ok(path);
        }

        if let Some(path) = self.resolved.get(pane_id)
            && path.is_file()
        {
            return Ok(path.clone());
        }

        let cwd = cwd.ok_or(SessionError::CwdUnavailable)?;
        match self.newest_for_cwd(agent, cwd, &mut budget)? {
            Some(path) => {
                self.resolved.insert(pane_id.to_owned(), path.clone());
                Ok(path)
            }
            None => match self.resolved.get(pane_id) {
                Some(path) if path.is_file() => Ok(path.clone()),
                _ => Err(SessionError::SessionFileMissing),
            },
        }
    }

    fn reported_path(
        &self,
        agent: Agent,
        cwd: Option<&str>,
        identity: &SessionIdentity,
        budget: &mut DiscoveryBudget,
    ) -> Result<PathBuf> {
        match identity {
            SessionIdentity::Path(path) => path
                .is_file()
                .then_some(path.clone())
                .ok_or(SessionError::SessionFileMissing),
            SessionIdentity::Id(id) => match agent {
                Agent::Claude => self.claude_path_for_id(cwd, id, budget),
                Agent::Codex => self.codex_path_for_id(id, budget),
                Agent::Grok | Agent::Pi | Agent::Omp | Agent::Cursor | Agent::Antigravity => {
                    native_file::locate(&self.home, agent, Some(identity), cwd, budget)
                }
                Agent::OpenCode => Err(SessionError::UnsupportedSessionKind),
            },
        }
    }

    fn claude_root(&self) -> PathBuf {
        self.home.join(CLAUDE_SESSIONS)
    }

    fn codex_root(&self) -> PathBuf {
        self.home.join(CODEX_SESSIONS)
    }

    fn claude_path_for_id(
        &self,
        cwd: Option<&str>,
        id: &str,
        budget: &mut DiscoveryBudget,
    ) -> Result<PathBuf> {
        let name = format!("{id}.jsonl");
        if let Some(cwd) = cwd {
            let direct = self.claude_root().join(project_directory(cwd)).join(&name);
            if direct.is_file() {
                return Ok(direct);
            }
        }
        let root = self.claude_root();
        for entry in read_directory(&root, budget)? {
            let candidate = entry.join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(SessionError::SessionFileMissing)
    }

    fn codex_path_for_id(&self, id: &str, budget: &mut DiscoveryBudget) -> Result<PathBuf> {
        for directory in self.recent_codex_days(budget)? {
            for path in jsonl_files(&directory, budget)? {
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(id))
                {
                    return Ok(path);
                }
            }
        }
        Err(SessionError::SessionFileMissing)
    }

    fn newest_for_cwd(
        &self,
        agent: Agent,
        cwd: &str,
        budget: &mut DiscoveryBudget,
    ) -> Result<Option<PathBuf>> {
        match agent {
            Agent::Claude => Ok(newest_by_modified(jsonl_files(
                &self.claude_root().join(project_directory(cwd)),
                budget,
            )?)),
            Agent::Codex => self.newest_codex_session(cwd, budget),
            Agent::Grok | Agent::Pi | Agent::Omp | Agent::Cursor | Agent::Antigravity => {
                native_file::locate(&self.home, agent, None, Some(cwd), budget).map(Some)
            }
            Agent::OpenCode => Err(SessionError::UnsupportedSessionKind),
        }
    }

    fn newest_codex_session(
        &self,
        cwd: &str,
        budget: &mut DiscoveryBudget,
    ) -> Result<Option<PathBuf>> {
        let mut candidates = Vec::new();
        for directory in self.recent_codex_days(budget)? {
            for path in jsonl_files(&directory, budget)? {
                let Some(modified) = fs::metadata(&path).and_then(|data| data.modified()).ok()
                else {
                    continue;
                };
                candidates.push((modified, path));
            }
        }
        candidates.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        Ok(candidates
            .into_iter()
            .map(|(_, path)| path)
            .find(|path| codex_session_cwd(path).as_deref() == Some(cwd)))
    }

    fn recent_codex_days(&self, budget: &mut DiscoveryBudget) -> Result<Vec<PathBuf>> {
        let mut days = Vec::new();
        for year in newest_directories(&self.codex_root(), 2, budget)? {
            for month in newest_directories(&year, 2, budget)? {
                days.extend(newest_directories(&month, CODEX_FALLBACK_DAYS, budget)?);
            }
        }
        days.sort();
        days.reverse();
        days.truncate(CODEX_FALLBACK_DAYS);
        Ok(days)
    }
}

fn read_directory(path: &Path, budget: &mut DiscoveryBudget) -> Result<Vec<PathBuf>> {
    budget.directory()?;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(SessionError::io("read_directory", path, error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        // Count before filtering or retaining: junk extensions, unmatched
        // names and unreadable entries cannot bypass the common total cap.
        budget.entry()?;
        let entry = entry.map_err(|error| SessionError::io("read_directory", path, error))?;
        let path = entry.path();
        budget.path(&path)?;
        paths.push(path);
    }
    Ok(paths)
}

fn jsonl_files(directory: &Path, budget: &mut DiscoveryBudget) -> Result<Vec<PathBuf>> {
    Ok(read_directory(directory, budget)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|value| value == "jsonl"))
        .collect())
}

fn newest_directories(
    root: &Path,
    limit: usize,
    budget: &mut DiscoveryBudget,
) -> Result<Vec<PathBuf>> {
    let mut directories = read_directory(root, budget)?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    directories.reverse();
    directories.truncate(limit);
    Ok(directories)
}

fn newest_by_modified(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).and_then(|data| data.modified()).ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

fn project_directory(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Return the newest Codex JSONL files in the newest YYYY/MM/DD directory.
///
/// This is intentionally separate from conversation parsing because Codex
/// usage records are not conversation events.
pub fn newest_codex_session_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut budget = DiscoveryBudget::default();
    let mut level = root.to_path_buf();
    for _ in 0..3 {
        let children = newest_directories(&level, usize::MAX, &mut budget)?
            .into_iter()
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .chars()
                        .all(|character| character.is_ascii_digit())
                })
            })
            .collect::<Vec<_>>();
        let Some(next) = children.into_iter().max() else {
            return Err(SessionError::SessionFileMissing);
        };
        level = next;
    }
    let mut candidates = jsonl_files(&level, &mut budget)?
        .into_iter()
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    candidates.truncate(CODEX_CANDIDATE_LIMIT);
    Ok(candidates.into_iter().map(|(_, path)| path).collect())
}

/// Read a bounded tail while dropping a line cut by the byte boundary.
pub fn read_tail(path: &Path, maximum_bytes: u64) -> Result<String> {
    let mut file =
        open_session_file(path).map_err(|error| SessionError::io("open", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| SessionError::io("stat", path, error))?
        .len();
    let start = length.saturating_sub(maximum_bytes);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| SessionError::io("seek", path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| SessionError::io("read", path, error))?;
    let contents = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Ok(contents);
    }
    Ok(contents
        .split_once('\n')
        .map_or_else(String::new, |(_, rest)| rest.to_owned()))
}

/// One page of complete lines read backwards from a line boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBefore {
    pub contents: String,
    /// Where `contents` begins, and where the next older page ends; 0 is the
    /// start of the file, with nothing older.
    pub start_offset: u64,
    /// Where `contents` ends: the `end` asked for, or after the file's last
    /// newline, so a line still being written is left to the cursor.
    pub end_offset: u64,
}

/// The block a backwards scan for a line start reads at a time.
const SCAN_BLOCK_BYTES: u64 = 64 * 1024;

/// Read the complete lines that end at or before `end`, at most
/// `maximum_bytes` of them, dropping the line the window cuts.
///
/// `end` must be a line boundary: 0, a previous page's `start_offset`, or
/// `None` for the file's last newline. A line longer than the window is
/// passed over by scanning back to its start, without keeping its bytes.
pub fn read_page_before(path: &Path, end: Option<u64>, maximum_bytes: u64) -> Result<PageBefore> {
    let mut file =
        open_session_file(path).map_err(|error| SessionError::io("open", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| SessionError::io("stat", path, error))?
        .len();
    let end = match end {
        Some(end) => end.min(length),
        None => line_start_before(&mut file, path, length)?,
    };
    let start = end.saturating_sub(maximum_bytes);
    // One byte before the window, so a line that starts exactly at `start` is
    // known to be whole.
    let read_start = start.saturating_sub(1);
    let mut bytes = vec![0; (end - read_start) as usize];
    file.seek(SeekFrom::Start(read_start))
        .map_err(|error| SessionError::io("seek", path, error))?;
    file.read_exact(&mut bytes)
        .map_err(|error| SessionError::io("read", path, error))?;
    let (start_offset, bytes) = if start == 0 {
        (0, bytes.as_slice())
    } else {
        // The window's last byte ends the newest line; a line starts after an
        // earlier newline or not at all.
        let body = &bytes[..bytes.len().saturating_sub(1)];
        match body.iter().position(|byte| *byte == b'\n') {
            Some(newline) => (read_start + newline as u64 + 1, &bytes[newline + 1..]),
            None => (line_start_before(&mut file, path, start)?, &[][..]),
        }
    };
    Ok(PageBefore {
        contents: String::from_utf8_lossy(bytes).into_owned(),
        start_offset,
        end_offset: end,
    })
}

/// The offset just after the last newline before `end`, or 0.
fn line_start_before(file: &mut File, path: &Path, end: u64) -> Result<u64> {
    let mut block_end = end;
    let mut block = Vec::new();
    while block_end > 0 {
        let block_start = block_end.saturating_sub(SCAN_BLOCK_BYTES);
        block.resize((block_end - block_start) as usize, 0);
        file.seek(SeekFrom::Start(block_start))
            .map_err(|error| SessionError::io("seek", path, error))?;
        file.read_exact(&mut block)
            .map_err(|error| SessionError::io("read", path, error))?;
        if let Some(newline) = block.iter().rposition(|byte| *byte == b'\n') {
            return Ok(block_start + newline as u64 + 1);
        }
        block_end = block_start;
    }
    Ok(0)
}

/// Read a whole file while enforcing a hard byte cap before and during I/O.
pub fn read_bounded(path: &Path, maximum_bytes: u64) -> Result<String> {
    let file = open_session_file(path).map_err(|error| SessionError::io("open", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| SessionError::io("stat", path, error))?
        .len();
    if length > maximum_bytes {
        return Err(SessionError::Capacity {
            resource: "file_bytes",
            limit: maximum_bytes,
        });
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| SessionError::io("read", path, error))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(SessionError::Capacity {
            resource: "file_bytes",
            limit: maximum_bytes,
        });
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn read_bounded_line(
    reader: &mut impl BufRead,
    maximum_bytes: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::with_capacity(maximum_bytes.min(8 * 1024));
    let mut limited = std::io::Read::take(&mut *reader, maximum_bytes.saturating_add(1) as u64);
    let read = limited.read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > maximum_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session line exceeds its fixed limit",
        ));
    }
    while line
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        line.pop();
    }
    Ok(Some(line))
}

/// Read the cwd from Codex's first session_meta line.
pub fn codex_session_cwd(path: &Path) -> Option<String> {
    let mut reader = BufReader::new(open_session_file(path).ok()?);
    let first = read_bounded_line(&mut reader, SESSION_LINE_LIMIT_BYTES).ok()??;
    let value: Value = serde_json::from_slice(&first).ok()?;
    value
        .pointer("/payload/cwd")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

enum LineResult {
    Ignore,
    Skip(SkipReason),
    Event(ConversationEvent),
    /// A session-level record rather than a turn: the title the agent gave
    /// the conversation.
    Title(String),
    /// The title the operator gave the conversation.
    CustomTitle(String),
    /// A physical current-title snapshot. Empty strings explicitly revoke
    /// previously observed names rather than retaining stale manual state.
    TitleSnapshot {
        title: String,
        custom_title: String,
    },
    /// A tool's output, which is no turn, and the pull request addresses it
    /// printed.
    Sightings(GithubSightings),
    /// A command record whose reply Claude Code has not written yet; the
    /// record cannot be classified until it has (`CommandReply`).
    Held,
}

/// What Claude Code wrote after a command record, which is what says whether
/// the command was a request to the model (`parse_claude_line`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandReply {
    /// Claude Code answered itself, as `/model` does with a
    /// `<local-command-stdout>` record: the command is not a request.
    Local,
    /// Anything else follows, a skill's or a user command's expansion among
    /// it, or an answer of Claude Code's that starts the operator's turn
    /// (`/goal`): the command went to the model.
    Model,
    /// No conversation record follows yet.
    Pending,
}

impl CommandReply {
    /// The first conversation record at or after `rest`, then `following`;
    /// [`Self::Pending`] when a reader that holds a command (`following` is
    /// `Some`) has seen none, and [`Self::Model`] for one that does not.
    fn after(rest: &[u8], following: Option<&[u8]>) -> Self {
        for line in rest.split(|byte| *byte == b'\n').chain(
            following
                .into_iter()
                .flat_map(|bytes| bytes.split(|byte| *byte == b'\n')),
        ) {
            if line.len() > SESSION_LINE_LIMIT_BYTES {
                continue;
            }
            let Ok(item) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let kind = item.get("type").and_then(Value::as_str);
            if !matches!(kind, Some("user" | "assistant" | "system")) {
                continue;
            }
            let own_reply = match kind {
                Some("system") => {
                    item.get("subtype").and_then(Value::as_str) == Some("local_command")
                }
                _ => item
                    .pointer("/message/content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| {
                        LOCAL_COMMAND_REPLIES
                            .iter()
                            .any(|tag| text.trim_start().starts_with(tag))
                    }),
            };
            // A command Claude Code answers and then hands to the model, as
            // `/goal` does, marks its answer as the start of the operator's
            // turn (2.1.294-2.1.301); a command it only answers (`/model`)
            // leaves no mark. The mark, not the command's name, decides.
            let starts_turn = item.get("turnOrigin").and_then(Value::as_str) == Some("human");
            return if own_reply && !starts_turn {
                Self::Local
            } else {
                Self::Model
            };
        }
        if following.is_some() {
            Self::Pending
        } else {
            Self::Model
        }
    }
}

/// The tags Claude Code wraps its own answer to a command in.
const LOCAL_COMMAND_REPLIES: &[&str] = &["<local-command-stdout>", "<local-command-stderr>"];

#[derive(Default)]
struct GithubSightings {
    prs: Vec<PrSighting>,
    issues: Vec<PrSighting>,
}

/// Parse Claude Code JSONL records into normalized events.
pub fn parse_claude_events(contents: &str) -> ParsedSession {
    parse_events_at(Agent::Claude, contents, 0)
}

/// Parse Codex response_item JSONL records into normalized events.
pub fn parse_codex_events(contents: &str) -> ParsedSession {
    parse_events_at(Agent::Codex, contents, 0)
}

/// Parse a complete-line chunk for the selected provider.
pub fn parse_events(agent: Agent, contents: &str) -> ParsedSession {
    parse_events_at(agent, contents, 0)
}

/// Parse a complete-line chunk while preserving each record's absolute byte
/// offset. The caller supplies the byte offset where `contents` begins.
/// OpenCode keeps no session file, so a chunk of one parses to nothing.
pub fn parse_events_at(agent: Agent, contents: &str, base_offset: u64) -> ParsedSession {
    let mut found = links::LinkAccumulator::default();
    let mut parsed = parse_events_into(agent, contents, base_offset, &mut found);
    parsed.links = found.finish();
    parsed
}

/// [`parse_events_at`] with the link reader's state carried by the caller,
/// so a reader that parses one line at a time keeps what earlier lines said
/// (the branch the session is on, its last request). `links` of the result
/// is left empty; the caller finishes the accumulator.
pub(crate) fn parse_events_into(
    agent: Agent,
    contents: &str,
    base_offset: u64,
    found: &mut links::LinkAccumulator,
) -> ParsedSession {
    parse_events_followed(agent, contents, base_offset, found, None)
}

/// [`parse_events_into`] for a reader that reads a file in polls. `following`
/// is the rest of the poll after `contents`, and a Claude command record that
/// nothing in `contents` or `following` answers is not classified: the result
/// says `held_command` and the caller reads the record again with the poll
/// that holds its reply, so a command and its reply may fall in different polls.
pub(crate) fn parse_events_followed(
    agent: Agent,
    contents: &str,
    base_offset: u64,
    found: &mut links::LinkAccumulator,
    following: Option<&[u8]>,
) -> ParsedSession {
    match agent {
        Agent::Claude => parse_lines_at(
            contents,
            base_offset,
            parse_claude_line,
            links::claude_line,
            Turns {
                parser: Some(turns::native::claude),
                wake: agent
                    .reports_wake_devices()
                    .then_some(turns::wake::claude as turns::wake::Parser),
                human_starts_turn: true,
                large: None,
            },
            found,
            following,
        ),
        Agent::Codex => parse_lines_at(
            contents,
            base_offset,
            |item, _| parse_codex_line(item),
            links::codex_line,
            Turns {
                parser: Some(turns::native::codex),
                wake: None,
                human_starts_turn: false,
                large: None,
            },
            found,
            following,
        ),
        Agent::Grok => parse_lines_at(
            contents,
            base_offset,
            |item, _| grok::parse_line(item),
            links::grok_line,
            Turns {
                parser: Some(grok::turn),
                wake: None,
                human_starts_turn: true,
                large: Some(grok::reduced_line),
            },
            found,
            following,
        ),
        Agent::Pi => parse_lines_at(
            contents,
            base_offset,
            |item, _| native_file::parse_line(Agent::Pi, item),
            links::pi_line,
            Turns {
                parser: None,
                wake: None,
                human_starts_turn: false,
                large: None,
            },
            found,
            following,
        ),
        Agent::Omp => parse_lines_at(
            contents,
            base_offset,
            |item, _| native_file::parse_line(Agent::Omp, item),
            links::pi_line,
            Turns {
                parser: Some(turns::native::omp),
                wake: None,
                human_starts_turn: true,
                large: None,
            },
            found,
            following,
        ),
        Agent::Antigravity => parse_lines_at(
            contents,
            base_offset,
            |item, _| antigravity::parse_line(item),
            links::antigravity_line,
            Turns {
                parser: None,
                wake: None,
                human_starts_turn: false,
                large: None,
            },
            found,
            following,
        ),
        Agent::OpenCode | Agent::Cursor => ParsedSession::default(),
    }
}

/// How a format's records mark turns, and how it reads a record longer
/// than the line cap.
struct Turns {
    parser: Option<turns::native::Parser>,
    /// Reads the records that start or end background work the agent waits
    /// on; only a format that declares it can prove one
    /// ([`Agent::reports_wake_devices`]).
    wake: Option<turns::wake::Parser>,
    human_starts_turn: bool,
    large: Option<fn(&str) -> Option<String>>,
}

/// Each record is parsed once and read by both the conversation parser and
/// the link reader (`links`), so a chunk yields its events and its link facts
/// from the same pass.
fn parse_lines_at(
    contents: &str,
    base_offset: u64,
    mut extract: impl FnMut(&Value, &dyn Fn() -> CommandReply) -> LineResult,
    link: links::LinkLine,
    turns: Turns,
    found: &mut links::LinkAccumulator,
    following: Option<&[u8]>,
) -> ParsedSession {
    let Turns {
        parser: turn,
        wake,
        human_starts_turn,
        large,
    } = turns;
    let mut parsed = ParsedSession::default();
    let mut relative_offset = 0_u64;
    for raw_line in contents.split_inclusive('\n') {
        let line_offset = base_offset.saturating_add(relative_offset);
        relative_offset = relative_offset.saturating_add(raw_line.len() as u64);
        let reduced;
        let raw_line = if raw_line.len() > SESSION_LINE_LIMIT_BYTES {
            match large.and_then(|reduce| reduce(raw_line)) {
                Some(line) => {
                    parsed.skipped(SkipReason::BodyCapacity);
                    reduced = line;
                    reduced.as_str()
                }
                None => {
                    parsed.skipped(SkipReason::NonConversationCapacity);
                    continue;
                }
            }
        } else {
            raw_line
        };
        let line = raw_line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(_) => {
                parsed.skipped(SkipReason::MalformedJson);
                continue;
            }
        };
        link(&value, line_offset, found);
        if let Some(turn) = turn {
            match turn(&value) {
                Ok(Some(mark)) => parsed.turn_marks.push((line_offset, mark)),
                Err(reason) => parsed.skipped(reason),
                Ok(None) => {}
            }
        }
        if let Some(wake) = wake {
            match wake(&value) {
                Ok(Some(mark)) => parsed.turn_marks.push((line_offset, mark)),
                Err(reason) => parsed.skipped(reason),
                Ok(None) => {}
            }
        }
        let after = &contents.as_bytes()[usize::try_from(relative_offset)
            .unwrap_or(usize::MAX)
            .min(contents.len())..];
        match extract(&value, &|| CommandReply::after(after, following)) {
            LineResult::Ignore => {}
            LineResult::Held => {
                parsed.held_command = true;
                break;
            }
            LineResult::Skip(reason) => parsed.skipped(reason),
            LineResult::Event(event) => {
                if turn.is_some() {
                    let mark = match event.kind {
                        EventKind::Human if human_starts_turn => Some(turns::TurnMark::HumanTurn),
                        EventKind::Human => Some(turns::TurnMark::Human),
                        EventKind::Interrupted => Some(turns::TurnMark::Interrupted),
                        _ => None,
                    };
                    if let Some(mark) = mark {
                        parsed.turn_marks.push((line_offset, mark));
                    }
                }
                found.event(event.kind, event.at_unix_ms, &event.text, line_offset);
                parsed.events.push(event);
                parsed.event_offsets.push(line_offset);
            }
            LineResult::Title(title) => parsed.title = Some(title),
            LineResult::CustomTitle(title) => parsed.custom_title = Some(title),
            LineResult::TitleSnapshot {
                title,
                custom_title,
            } if line_offset == 0 => {
                parsed.title = Some(title);
                parsed.custom_title = Some(custom_title);
            }
            LineResult::TitleSnapshot { .. } => {}
            LineResult::Sightings(sightings) => {
                found.sightings(&sightings.prs);
                found.issue_sightings(&sightings.issues);
                parsed.pr_sightings.extend(sightings.prs);
                parsed.issue_sightings.extend(sightings.issues);
            }
        }
    }
    parsed
}

fn parse_claude_line(item: &Value, reply: &dyn Fn() -> CommandReply) -> LineResult {
    let title = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_owned)
    };
    let role = match item.get("type").and_then(Value::as_str) {
        Some("user") => "user",
        Some("assistant") => "assistant",
        Some("ai-title") => return title("aiTitle").map_or(LineResult::Ignore, LineResult::Title),
        Some("custom-title") => {
            return title("customTitle").map_or(LineResult::Ignore, LineResult::CustomTitle);
        }
        Some("attachment") => return claude_hook_context(item),
        _ => return LineResult::Ignore,
    };
    let content = item.pointer("/message/content");
    if role == "user"
        && let Some(outputs) = claude_tool_outputs(content)
    {
        return match timestamp_ms(item.get("timestamp")) {
            Ok(at) => LineResult::Sightings(sightings_in(&outputs, at)),
            Err(_) => LineResult::Ignore,
        };
    }
    let images = image_blocks(content, "image");
    let text = match session_text(content) {
        Some(text) => text,
        None if role == "user" && images > 0 => String::new(),
        None => return LineResult::Ignore,
    };
    let timestamp = match timestamp_ms(item.get("timestamp")) {
        Ok(timestamp) => timestamp,
        Err(reason) => return LineResult::Skip(reason),
    };
    if role == "assistant" {
        return LineResult::Event(ConversationEvent::new(
            role,
            EventKind::Assistant,
            timestamp,
            text,
        ));
    }
    let origin_kind = item.pointer("/origin/kind").and_then(Value::as_str);
    let origin_is_human = origin_kind == Some("human");
    let origin_is_injected = matches!(origin_kind, Some("hook" | "system"));
    let external_human = item.get("userType").and_then(Value::as_str) == Some("external")
        && item.get("promptId").and_then(Value::as_str).is_some()
        && item
            .pointer("/message/content")
            .is_some_and(Value::is_string);
    let is_meta = item.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
    let is_system_prompt = item.get("promptSource").and_then(Value::as_str) == Some("system");
    // A compaction summary is Claude Code's own record, written as an external
    // user prompt with a `promptId`; only these flags tell it from the operator.
    let is_compact_summary = item
        .get("isCompactSummary")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || item
            .get("isVisibleInTranscriptOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let provider_injected = origin_is_injected || is_meta || is_system_prompt || is_compact_summary;
    let interrupted = is_interruption(&text);
    let command = slash_command_text(&text);
    let mut kind = if interrupted {
        EventKind::Interrupted
    } else if (origin_is_human || external_human)
        && !provider_injected
        && !has_injected_prefix(&text)
    {
        EventKind::Human
    } else {
        EventKind::Injected
    };
    if kind == EventKind::Human && command.is_some() {
        // A command is the operator's words only when it went to the model;
        // what Claude Code answered itself (`/model`) is its own record.
        match reply() {
            CommandReply::Local => kind = EventKind::Injected,
            CommandReply::Pending => return LineResult::Held,
            CommandReply::Model => {}
        }
    }
    let text = if kind == EventKind::Human {
        command.unwrap_or(text)
    } else {
        text
    };
    LineResult::Event(
        ConversationEvent::new(role, kind, timestamp, text)
            .with_provider_injected(provider_injected)
            .with_images(images),
    )
}

/// The context a hook returned, which Claude Code writes as an `attachment`
/// record of its own rather than as a message. It is provider-injected, so a
/// Memory receipt in it is the hook's; the operator's words are never in an
/// attachment. Any other kind of attachment is not conversation.
fn claude_hook_context(item: &Value) -> LineResult {
    let Some(claude_attachment::Attachment::HookContext { content }) =
        claude_attachment::Attachment::of(item)
    else {
        return LineResult::Ignore;
    };
    let Some(text) = claude_attachment::hook_context_text(content) else {
        return LineResult::Ignore;
    };
    match timestamp_ms(item.get("timestamp")) {
        Ok(timestamp) => LineResult::Event(
            ConversationEvent::new("user", EventKind::Injected, timestamp, text)
                .with_provider_injected(true),
        ),
        Err(reason) => LineResult::Skip(reason),
    }
}

/// The text of every `tool_result` block in a Claude user record, or `None`
/// when the record holds none (a person's message).
fn claude_tool_outputs(content: Option<&Value>) -> Option<Vec<&str>> {
    let blocks = content?.as_array()?;
    let mut outputs = Vec::new();
    let mut any = false;
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        any = true;
        collect_text(block.get("content"), &mut outputs);
    }
    any.then_some(outputs)
}

/// A tool output's text, whether a string or blocks carrying `text`.
fn collect_text<'a>(value: Option<&'a Value>, into: &mut Vec<&'a str>) {
    match value {
        Some(Value::String(text)) => into.push(text),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block {
                    Value::String(text) => into.push(text),
                    other => {
                        if let Some(text) = other.get("text").and_then(Value::as_str) {
                            into.push(text);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn sightings_in(outputs: &[&str], at_unix_ms: u64) -> GithubSightings {
    GithubSightings {
        prs: address_sightings(outputs, at_unix_ms, pull_request_addresses),
        issues: address_sightings(outputs, at_unix_ms, issue_addresses),
    }
}

fn address_sightings(
    outputs: &[&str],
    at_unix_ms: u64,
    addresses: fn(&str) -> Vec<(String, u64)>,
) -> Vec<PrSighting> {
    let mut found: Vec<PrSighting> = Vec::new();
    for output in outputs {
        for (repository, number) in addresses(output) {
            if found.len() == MAX_SIGHTINGS_PER_OUTPUT {
                return found;
            }
            if !found
                .iter()
                .any(|seen| seen.number == number && seen.repository == repository)
            {
                found.push(PrSighting {
                    repository,
                    number,
                    at_unix_ms,
                });
            }
        }
    }
    found
}

fn image_blocks(content: Option<&Value>, kind: &str) -> u32 {
    content.and_then(Value::as_array).map_or(0, |blocks| {
        blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some(kind))
            .count() as u32
    })
}

fn parse_codex_line(item: &Value) -> LineResult {
    if item.get("type").and_then(Value::as_str) != Some("response_item") {
        return LineResult::Ignore;
    }
    let Some(payload) = item.get("payload") else {
        return LineResult::Ignore;
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {}
        // Codex records a tool's output after the tool ran, so the record's
        // time is when the output was printed.
        Some("function_call_output" | "custom_tool_call_output") => {
            let mut outputs = Vec::new();
            match payload.get("output") {
                Some(Value::Object(object)) => {
                    collect_text(object.get("content").or(object.get("output")), &mut outputs);
                }
                other => collect_text(other, &mut outputs),
            }
            return match timestamp_ms(item.get("timestamp")) {
                Ok(at) => LineResult::Sightings(sightings_in(&outputs, at)),
                Err(_) => LineResult::Ignore,
            };
        }
        _ => return LineResult::Ignore,
    }
    // Codex writes the instructions it adds to a turn as `developer`
    // messages; a conversation names that role `system` on every
    // provider, which is the closed set a page crosses the node link with.
    let role = match payload.get("role").and_then(Value::as_str) {
        Some("user") => "user",
        Some("developer") => "system",
        Some("assistant") => "assistant",
        _ => return LineResult::Ignore,
    };
    let images = image_blocks(payload.get("content"), "input_image");
    let text = match session_text(codex_own_text(role, payload.get("content")).as_ref()) {
        Some(text) => text,
        None if role == "user" && images > 0 => String::new(),
        None => return LineResult::Ignore,
    };
    let timestamp = match timestamp_ms(item.get("timestamp")) {
        Ok(timestamp) => timestamp,
        Err(reason) => return LineResult::Skip(reason),
    };
    if role == "assistant" {
        return LineResult::Event(ConversationEvent::new(
            role,
            EventKind::Assistant,
            timestamp,
            text,
        ));
    }
    if role == "system" {
        return LineResult::Event(
            ConversationEvent::new(role, EventKind::Injected, timestamp, text)
                .with_provider_injected(true),
        );
    }
    match codex_internal_context(&text) {
        Some(CodexInternal::Own) => {
            return LineResult::Event(
                ConversationEvent::new(role, EventKind::Injected, timestamp, text)
                    .with_provider_injected(true),
            );
        }
        Some(CodexInternal::UserGoal(objective)) => {
            return LineResult::Event(ConversationEvent::new(
                role,
                EventKind::Human,
                timestamp,
                objective.unwrap_or(text),
            ));
        }
        None => {}
    }
    let kind = if is_interruption(&text) {
        EventKind::Interrupted
    } else if has_injected_prefix(&text) {
        EventKind::Injected
    } else {
        EventKind::Human
    };
    let text = if kind == EventKind::Human {
        slash_command_text(&text).unwrap_or(text)
    } else {
        text
    };
    LineResult::Event(ConversationEvent::new(role, kind, timestamp, text).with_images(images))
}

/// What a Codex user message wrapped in `<codex_internal_context>` is.
#[derive(Debug, PartialEq, Eq)]
enum CodexInternal {
    /// `source="user_goal"` setting a goal: the operator's `/goal`. The
    /// objective is their words; `None` when Codex wrote none to read (it
    /// omits a long one), and the message is then kept as written.
    UserGoal(Option<String>),
    /// Any other source, or a `user_goal` that sets no goal (one cleared):
    /// Codex's own text, such as the `source="goal"` reminder it adds to
    /// every turn while a goal is active.
    Own,
}

/// Codex writes what it adds to a turn on its own behalf as a user message
/// wrapped in `<codex_internal_context source="...">` (codex-cli 0.163.0).
/// The wrapper is Codex's format, not ours, so this one function reads it:
/// the source names whose words the message carries, and only `user_goal`
/// carries the operator's, as `User set the goal: "<JSON string>"`.
fn codex_internal_context(text: &str) -> Option<CodexInternal> {
    let rest = text.trim_start().strip_prefix("<codex_internal_context")?;
    let (attributes, body) = rest.split_once('>')?;
    if attributes
        .split_whitespace()
        .all(|attribute| attribute != r#"source="user_goal""#)
    {
        return Some(CodexInternal::Own);
    }
    // Only a goal being set carries the operator's words; a goal cleared or
    // paused (`User cleared the goal.`) is Codex reporting a state.
    let Some(quoted) = body.trim_start().strip_prefix("User set the goal: ") else {
        return Some(CodexInternal::Own);
    };
    let objective = serde_json::Deserializer::from_str(quoted)
        .into_iter::<String>()
        .next()
        .and_then(std::result::Result::ok)
        .map(|objective| objective.trim().to_owned())
        .filter(|objective| !objective.is_empty());
    Some(CodexInternal::UserGoal(objective))
}

/// All prefixes that identify prompt scaffolding live here so both providers
/// use the same fixed classification table.
pub const INJECTED_PREFIXES: &[&str] = &[
    "<task-notification>",
    "<system-reminder>",
    "Another Claude session",
    "# AGENTS.md instructions",
    "<environment_context>",
    "<user_instructions>",
    // Codex's record of the operator's answer to an async question: it
    // answers that question and is not a request.
    turns::native::ASYNC_REPLY_OPEN,
    // Claude Code's compaction summary, for a version that does not flag it.
    "This session is being continued from a previous conversation",
];

/// The tags a Project Memory delivery begins with: a prompt's cards follow
/// Hide's instruction for reporting a stale one, the index stands alone.
const MEMORY_TAGS: &[&str] = &["<hide-memory-context", "<hide-memory-instruction"];

fn has_injected_prefix(text: &str) -> bool {
    let text = text.trim_start();
    let memory_context = MEMORY_TAGS.iter().any(|tag| {
        text.strip_prefix(tag)
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|boundary| boundary == '>' || boundary.is_ascii_whitespace())
    });
    memory_context
        || INJECTED_PREFIXES
            .iter()
            .chain(LOCAL_COMMAND_REPLIES)
            .any(|prefix| text.starts_with(prefix))
}

fn is_interruption(text: &str) -> bool {
    text.trim_start().starts_with("[Request interrupted")
}

fn slash_command_text(text: &str) -> Option<String> {
    let name = text
        .split_once("<command-name>")?
        .1
        .split_once("</command-name>")?
        .0
        .trim();
    if name.is_empty() {
        return None;
    }
    let args = text
        .split_once("<command-args>")
        .and_then(|(_, rest)| {
            rest.split_once("</command-args>")
                .map(|(args, _)| args.trim())
        })
        .unwrap_or("");
    Some(if args.is_empty() {
        // Claude Code's `<command-name>` already carries the slash.
        format!("/{}", name.trim_start_matches('/'))
    } else {
        args.to_owned()
    })
}

/// A Codex user message without what Codex wrapped around its attachments:
/// each image sits between `<image name=[Image #N] path="...">` and
/// `</image>` blocks, and attached files are listed in a
/// `# Files mentioned by the user:` block (codex-cli 0.160). None of it is
/// the operator's words, and the paths name the operator's folders.
fn codex_own_text(role: &str, content: Option<&Value>) -> Option<Value> {
    let content = content?;
    let (true, Value::Array(blocks)) = (role == "user", content) else {
        return Some(content.clone());
    };
    let wrapper = |block: &&Value| {
        block
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim_start)
            .is_some_and(|text| {
                text.starts_with("<image name=")
                    || text == "</image>"
                    || text.starts_with("# Files mentioned by the user:")
            })
    };
    Some(Value::Array(
        blocks
            .iter()
            .filter(|block| !wrapper(block))
            .cloned()
            .collect(),
    ))
}

fn session_text(content: Option<&Value>) -> Option<String> {
    let text = match content? {
        Value::String(text) => text.to_owned(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("text" | "input_text" | "output_text")
                )
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    (!text.trim().is_empty()).then_some(text)
}

fn timestamp_ms(value: Option<&Value>) -> std::result::Result<u64, SkipReason> {
    let Some(value) = value else {
        return Err(SkipReason::MissingTimestamp);
    };
    if let Some(value) = value.as_u64() {
        return if value >= 10_000_000_000 {
            Ok(value)
        } else {
            value.checked_mul(1_000).ok_or(SkipReason::InvalidTimestamp)
        };
    }
    let Some(value) = value.as_str() else {
        return Err(SkipReason::InvalidTimestamp);
    };
    parse_rfc3339_unix_ms(value).ok_or(SkipReason::InvalidTimestamp)
}

/// Parse an RFC3339 timestamp into Unix seconds.
pub fn parse_rfc3339(value: &str) -> Option<u64> {
    parse_rfc3339_parts(value).map(|(seconds, _)| seconds)
}

fn parse_rfc3339_unix_ms(value: &str) -> Option<u64> {
    let (seconds, millis) = parse_rfc3339_parts(value)?;
    seconds.checked_mul(1_000)?.checked_add(millis)
}

fn parse_rfc3339_parts(value: &str) -> Option<(u64, u64)> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let number = |start: usize, end: usize| value.get(start..end)?.parse::<i64>().ok();
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let mut cursor = 19;
    let millis = if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            return None;
        }
        let fraction = value.get(start..cursor)?;
        let mut millis = 0_u64;
        for digit in fraction.bytes().take(3) {
            millis = millis * 10 + u64::from(digit - b'0');
        }
        if fraction.len() == 1 {
            millis *= 100;
        } else if fraction.len() == 2 {
            millis *= 10;
        }
        millis
    } else {
        0
    };
    let offset = match bytes.get(cursor)? {
        b'Z' | b'z' if cursor + 1 == bytes.len() => 0,
        sign @ (b'+' | b'-')
            if cursor + 6 == bytes.len() && bytes.get(cursor + 3) == Some(&b':') =>
        {
            let offset_hour = number(cursor + 1, cursor + 3)?;
            let offset_minute = number(cursor + 4, cursor + 6)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let seconds = offset_hour * 3_600 + offset_minute * 60;
            if *sign == b'+' { seconds } else { -seconds }
        }
        _ => return None,
    };
    let timestamp = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?
        .checked_sub(offset)?;
    Some((u64::try_from(timestamp).ok()?, millis))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_rfc3339_with_fraction_and_offset() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01Z"), Some(1));
        assert_eq!(
            parse_rfc3339_unix_ms("1970-01-01T09:00:01.234+09:00"),
            Some(1_234)
        );
        assert_eq!(parse_rfc3339("not-a-date"), None);
    }

    #[test]
    fn pages_walk_back_over_complete_lines_to_the_start() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, b"one\ntwo\nthree\npartial").unwrap();

        let newest = read_page_before(&path, None, 6).unwrap();
        assert_eq!(newest.contents, "three\n");
        assert_eq!(newest.end_offset, 14);
        let older = read_page_before(&path, Some(newest.start_offset), 6).unwrap();
        assert_eq!(older.contents, "two\n");
        let oldest = read_page_before(&path, Some(older.start_offset), 6).unwrap();
        assert_eq!(
            (oldest.contents.as_str(), oldest.start_offset),
            ("one\n", 0)
        );
    }

    #[test]
    fn a_page_passes_over_a_line_longer_than_its_window() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        let long = "x".repeat(200_000);
        fs::write(&path, format!("one\n{long}\nthree\n")).unwrap();

        let newest = read_page_before(&path, None, 8).unwrap();
        assert_eq!(newest.contents, "three\n");
        let over = read_page_before(&path, Some(newest.start_offset), 8).unwrap();
        assert_eq!((over.contents.as_str(), over.start_offset), ("", 4));
        assert_eq!(
            read_page_before(&path, Some(4), 8).unwrap().contents,
            "one\n"
        );
    }

    #[test]
    fn bounded_reader_rejects_a_file_larger_than_its_limit() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, b"12345").unwrap();
        assert_eq!(read_bounded(&path, 5).unwrap(), "12345");
        assert!(matches!(
            read_bounded(&path, 4),
            Err(SessionError::Capacity {
                resource: "file_bytes",
                limit: 4
            })
        ));
    }

    #[test]
    fn codex_cwd_reader_rejects_an_oversized_first_record() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, vec![b'x'; SESSION_LINE_LIMIT_BYTES + 1]).unwrap();

        assert_eq!(codex_session_cwd(&path), None);
    }

    #[test]
    fn parser_counts_only_relevant_lines_and_preserves_reason_categories() {
        let input = concat!(
            "{\"type\":\"user\",\"timestamp\":\"1970-01-01T00:00:01Z\",\"origin\":{\"kind\":\"human\"},\"message\":{\"content\":\"요청\"}}\n",
            "{\"type\":\"user\",\"timestamp\":\"bad\",\"origin\":{\"kind\":\"human\"},\"message\":{\"content\":\"건너뜀\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"시간 없음\"}}\n",
            "not-json\n",
            "{\"type\":\"session_meta\",\"payload\":{}}\n",
        );
        let parsed = parse_claude_events(input);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].kind, EventKind::Human);
        assert_eq!(parsed.events[0].at_unix_ms, 1_000);
        assert_eq!(parsed.skipped_lines, 3);
        assert_eq!(parsed.skipped_reasons[&SkipReason::InvalidTimestamp], 1);
        assert_eq!(parsed.skipped_reasons[&SkipReason::MissingTimestamp], 1);
        assert_eq!(parsed.skipped_reasons[&SkipReason::MalformedJson], 1);
    }

    #[test]
    fn parser_preserves_absolute_jsonl_offsets_for_emitted_events() {
        let ignored = "{\"type\":\"session_meta\",\"payload\":{}}\n";
        let first = "{\"type\":\"user\",\"timestamp\":\"1970-01-01T00:00:01Z\",\"origin\":{\"kind\":\"human\"},\"message\":{\"content\":\"one\"}}\n";
        let second = "{\"type\":\"assistant\",\"timestamp\":\"1970-01-01T00:00:02Z\",\"message\":{\"content\":\"two\"}}\n";
        let parsed = parse_events_at(Agent::Claude, &format!("{ignored}{first}{second}"), 700);
        assert_eq!(
            parsed.event_offsets,
            vec![
                700 + ignored.len() as u64,
                700 + ignored.len() as u64 + first.len() as u64
            ]
        );
    }

    #[test]
    fn parses_claude_human_injected_interrupted_and_command_events() {
        let fixture = include_str!("../tests/fixtures/claude.jsonl");
        let parsed = parse_claude_events(fixture);
        assert_eq!(
            parsed
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                EventKind::Human,
                EventKind::Injected,
                EventKind::Injected,
                EventKind::Injected,
                EventKind::Human,
                EventKind::Interrupted,
                EventKind::Assistant,
            ]
        );
        assert_eq!(parsed.events[0].text, "첫 번째 요청");
        assert_eq!(parsed.events[4].text, "--quick");
        assert_eq!(parsed.events[5].text, "[Request interrupted by user]");
        assert_eq!(parsed.skipped_lines, 0);
        assert_eq!(parsed.title, None, "the fixture carries no ai-title record");
    }

    /// Claude's `ai-title` record is the session's own name: the last one
    /// wins, it is not an event, and an empty one clears a stored title.
    #[test]
    fn claude_ai_title_is_the_session_title_and_not_an_event() {
        let chunk = concat!(
            r#"{"type":"ai-title","aiTitle":"  Hook 버그 확인 ","sessionId":"s1"}"#,
            "\n",
            r#"{"type":"user","timestamp":"2026-09-18T00:00:00.000Z","origin":{"kind":"human"},"message":{"role":"user","content":"hook 고쳐줘"}}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"","sessionId":"s1"}"#,
            "\n",
            r#"{"type":"ai-title","aiTitle":"Hook 보고 경로 교체","sessionId":"s1"}"#,
            "\n",
        );
        let parsed = parse_claude_events(chunk);
        assert_eq!(parsed.title.as_deref(), Some("Hook 보고 경로 교체"));
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.skipped_lines, 0);
        let codex = parse_codex_events(chunk);
        assert_eq!(codex.title, None);
    }

    #[test]
    fn parses_codex_human_and_injected_prefixes() {
        let fixture = include_str!("../tests/fixtures/codex.jsonl");
        let parsed = parse_codex_events(fixture);
        assert_eq!(
            parsed
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                EventKind::Injected,
                EventKind::Injected,
                EventKind::Injected,
                EventKind::Human,
                EventKind::Assistant,
            ]
        );
        assert_eq!(parsed.events[3].text, "실제 요청");
        assert_eq!(parsed.events[4].at_unix_ms, 1_789_516_805_000);
    }

    #[test]
    fn codex_developer_message_is_provider_authenticated_injected_context() {
        let line = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-09-18T00:00:00Z",
            "payload": {
                "type": "message",
                "role": "developer",
                "content": [{"type": "input_text", "text": "<hide-memory-context />"}],
            },
        })
        .to_string();
        let parsed = parse_codex_events(&line);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].role, "system");
        assert_eq!(parsed.events[0].kind, EventKind::Injected);
        assert!(parsed.events[0].is_provider_injected());
    }

    /// Hide's extension message, as Pi 1.0.4 and omp 18.7.0 store it, carries
    /// the Memory receipt as provider-injected context; another extension's
    /// custom message stays out, and an operator's own text quoting a receipt,
    /// even inside reminder tags, never asserts one (PRD pi-omp-extension
    /// D-09).
    #[test]
    fn hides_pi_and_omp_extension_message_is_provider_injected_context() {
        let receipt =
            "<hide-memory-receipt event=\"SessionStart\" count=\"0\" items=\"\" auth=\"aa\" />";
        let hidden = format!("<system-reminder>\nguidance\n{receipt}\n</system-reminder>");
        let lines = [
            serde_json::json!({"type": "session", "version": 3, "id": "native-a", "timestamp": "2026-10-09T00:00:00.000Z", "cwd": "/work/app"}),
            serde_json::json!({"type": "message", "timestamp": "2026-10-09T00:00:01.000Z", "message": {"role": "user", "content": [{"type": "text", "text": receipt}]}}),
            serde_json::json!({"type": "message", "timestamp": "2026-10-09T00:00:01.200Z", "message": {"role": "user", "content": [{"type": "text", "text": format!("<system-reminder>\n{receipt}\n</system-reminder>")}]}}),
            serde_json::json!({"type": "custom_message", "customType": "hide", "content": hidden, "display": false, "details": {"hide": "0011"}, "timestamp": "2026-10-09T00:00:01.500Z"}),
            serde_json::json!({"type": "custom_message", "customType": "extension", "content": receipt, "display": true, "timestamp": "2026-10-09T00:00:02.000Z"}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        for agent in [Agent::Pi, Agent::Omp] {
            let parsed = parse_events(agent, &format!("{lines}\n"));
            let kinds: Vec<_> = parsed
                .events
                .iter()
                .map(|event| (event.kind, event.is_provider_injected()))
                .collect();
            assert_eq!(
                kinds,
                [
                    (EventKind::Human, false),
                    (EventKind::Injected, false),
                    (EventKind::Injected, true)
                ],
                "{agent:?}"
            );
            assert_eq!(parsed.events[2].text, hidden, "{agent:?}");
        }
    }

    #[test]
    fn a_codex_goal_without_a_readable_objective_keeps_its_message() {
        // Codex omits an objective past its evidence limit (codex-cli 0.163.0).
        let omitted = "<codex_internal_context source=\"user_goal\">\nUser set the goal: [objective omitted; exceeds the evidence limit].\n</codex_internal_context>";
        let line = serde_json::json!({
            "timestamp": "2026-10-17T00:00:00Z",
            "type": "response_item",
            "payload": {"type": "message", "role": "user",
                "content": [{"type": "input_text", "text": omitted}]},
        })
        .to_string();
        let parsed = parse_codex_events(&line);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].kind, EventKind::Human);
        assert_eq!(parsed.events[0].text, omitted);
        for own in [
            "<codex_internal_context source=\"budget\">\nBudget reached.",
            "<codex_internal_context source=\"user_goal\">\nUser cleared the goal.\n</codex_internal_context>",
        ] {
            assert_eq!(
                codex_internal_context(own),
                Some(CodexInternal::Own),
                "{own}"
            );
        }
    }

    #[test]
    fn claude_external_human_without_origin_remains_human_and_untrusted() {
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-09-18T00:00:00Z",
            "userType": "external",
            "entrypoint": "claude-desktop",
            "promptId": "prompt-1",
            "message": {
                "role": "user",
                "content": "<hide-memory-receipt event=\"UserPromptSubmit\" />",
            },
        })
        .to_string();
        let parsed = parse_claude_events(&line);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].kind, EventKind::Human);
        assert!(!parsed.events[0].is_provider_injected());
    }

    /// Claude Code 2.1.296 writes a hook's added context as an `attachment`
    /// record, one string per hook, and repeats it in the `stdout` of the
    /// `hook_success` record beside it; only the first is conversation.
    #[test]
    fn claude_hook_context_attachment_is_provider_injected_and_counted_once() {
        let receipt =
            "<hide-memory-receipt event=\"SessionStart\" count=\"0\" items=\"\" auth=\"aa\" />";
        let line = |attachment: serde_json::Value| {
            serde_json::json!({
                "type": "attachment",
                "timestamp": "2026-10-11T00:00:00.000Z",
                "attachment": attachment,
            })
            .to_string()
        };
        let session = [
            line(serde_json::json!({
                "type": "hook_success",
                "hookName": "SessionStart:startup",
                "hookEvent": "SessionStart",
                "stdout": serde_json::json!({
                    "hookSpecificOutput": {"additionalContext": receipt},
                })
                .to_string(),
            })),
            line(serde_json::json!({
                "type": "hook_additional_context",
                "hookName": "SessionStart:startup",
                "hookEvent": "SessionStart",
                "content": ["another hook's context", receipt],
            })),
            line(serde_json::json!({"type": "queued_command", "prompt": receipt})),
            line(serde_json::json!({"type": "diagnostics", "content": receipt})),
            line(serde_json::json!({"type": "hook_additional_context", "content": []})),
        ]
        .join("\n");

        let parsed = parse_claude_events(&session);

        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].kind, EventKind::Injected);
        assert!(parsed.events[0].is_provider_injected());
        assert_eq!(
            parsed.events[0].text,
            format!("another hook's context\n{receipt}")
        );
        assert_eq!(parsed.skipped_lines, 0);
    }

    #[test]
    fn claude_hook_context_attachment_without_a_timestamp_is_skipped() {
        let line = serde_json::json!({
            "type": "attachment",
            "attachment": {"type": "hook_additional_context", "content": ["context"]},
        })
        .to_string();

        let parsed = parse_claude_events(&line);

        assert!(parsed.events.is_empty());
        assert_eq!(parsed.skipped_lines, 1);
    }

    #[test]
    fn hook_memory_context_with_a_trust_attribute_is_injected_in_every_message_shape() {
        let context = concat!(
            "<hide-memory-context trust=\"untrusted-reference-data\">\n",
            "reference data\n",
            "</hide-memory-context>"
        );
        for context in [
            context.to_owned(),
            // A prompt's cards follow the line saying how to report one stale.
            format!(
                "<hide-memory-instruction>run hide memory stale</hide-memory-instruction>\n{context}"
            ),
        ] {
            memory_context_is_injected(&context);
        }
    }

    fn memory_context_is_injected(context: &str) {
        let codex = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-09-18T00:00:00Z",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": context}],
            },
        })
        .to_string();
        let claude = serde_json::json!({
            "type": "user",
            "timestamp": "2026-09-18T00:00:00Z",
            "origin": {"kind": "human"},
            "message": {"role": "user", "content": context},
        })
        .to_string();

        assert_eq!(
            parse_codex_events(&codex).events[0].kind,
            EventKind::Injected
        );
        assert_eq!(
            parse_claude_events(&claude).events[0].kind,
            EventKind::Injected
        );
        let claude_attachment = serde_json::json!({
            "type": "attachment",
            "timestamp": "2026-09-18T00:00:00Z",
            "attachment": {"type": "hook_additional_context", "content": [context]},
        })
        .to_string();
        assert_eq!(
            parse_claude_events(&claude_attachment).events[0].kind,
            EventKind::Injected
        );

        let malformed = serde_json::json!({
            "type": "response_item",
            "timestamp": "2026-09-18T00:00:00Z",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "<hide-memory-context-malformed>"}],
            },
        })
        .to_string();
        assert_eq!(
            parse_codex_events(&malformed).events[0].kind,
            EventKind::Human
        );
        assert!(!parse_codex_events(&codex).events[0].is_provider_injected());
    }

    #[test]
    fn locator_finds_claude_and_codex_files() {
        let home = tempdir().unwrap();
        let claude_dir = home.path().join(".claude/projects/-Users-example");
        let codex_dir = home.path().join(".codex/sessions/2026/09/16");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::create_dir_all(&codex_dir).unwrap();
        let claude = claude_dir.join("claude-id.jsonl");
        let codex = codex_dir.join("rollout-codex-id.jsonl");
        fs::write(&claude, b"").unwrap();
        fs::write(
            &codex,
            b"{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/Users/example\"}}\n",
        )
        .unwrap();
        let mut locator = SessionLocator::new(home.path());
        assert_eq!(
            locator
                .locate(
                    "pane-claude",
                    Agent::Claude,
                    Some(&SessionIdentity::id("claude-id")),
                    Some("/Users/example"),
                )
                .unwrap(),
            claude
        );
        assert_eq!(
            locator
                .locate(
                    "pane-codex",
                    Agent::Codex,
                    Some(&SessionIdentity::id("codex-id")),
                    Some("/Users/example"),
                )
                .unwrap(),
            codex
        );
    }

    #[test]
    fn a_reported_session_is_kept_when_a_neighbour_in_the_same_cwd_is_newer() {
        let home = tempdir().unwrap();
        let claude_dir = home.path().join(".claude/projects/-Users-example");
        fs::create_dir_all(&claude_dir).unwrap();
        let quiet = claude_dir.join("quiet-id.jsonl");
        let busy = claude_dir.join("busy-id.jsonl");
        fs::write(&quiet, b"").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
        fs::File::open(&quiet).unwrap().set_modified(old).unwrap();
        fs::write(&busy, b"").unwrap();
        let mut locator = SessionLocator::new(home.path());
        assert_eq!(
            locator
                .locate(
                    "pane-quiet",
                    Agent::Claude,
                    Some(&SessionIdentity::id("quiet-id")),
                    Some("/Users/example"),
                )
                .unwrap(),
            quiet
        );
        // Without a reported id the newest file in the cwd is still the answer.
        assert_eq!(
            locator
                .locate("pane-unknown", Agent::Claude, None, Some("/Users/example"))
                .unwrap(),
            busy
        );
    }

    /// A reported session whose file is not written yet (Claude before its
    /// first turn) is missing, never the neighbour's newest transcript.
    #[test]
    fn a_reported_session_without_a_file_yet_is_missing_not_the_neighbour() {
        let home = tempdir().unwrap();
        let claude_dir = home.path().join(".claude/projects/-Users-example");
        fs::create_dir_all(&claude_dir).unwrap();
        let busy = claude_dir.join("busy-id.jsonl");
        fs::write(&busy, b"").unwrap();
        let mut locator = SessionLocator::new(home.path());
        let unborn = SessionIdentity::id("unborn-id");
        assert!(matches!(
            locator.locate(
                "pane-new",
                Agent::Claude,
                Some(&unborn),
                Some("/Users/example")
            ),
            Err(SessionError::SessionFileMissing)
        ));
        // Once the file exists the same pane resolves to it, not to what
        // the cwd search would have cached.
        let own = claude_dir.join("unborn-id.jsonl");
        fs::write(&own, b"").unwrap();
        assert_eq!(
            locator
                .locate(
                    "pane-new",
                    Agent::Claude,
                    Some(&unborn),
                    Some("/Users/example")
                )
                .unwrap(),
            own
        );
    }
}
