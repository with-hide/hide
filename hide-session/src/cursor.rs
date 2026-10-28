//! Cursor's ordinary local chats: a read-only SQLite content-addressed graph.
//! The native sidecar supplies cwd; neither a display name, derived transcript
//! nor a previous-workspace URI is ownership evidence.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use prost::Message;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ConversationEvent, EventKind, ParsedSession, RescanReason, Result, SessionError};

pub const SESSIONS: &str = ".cursor/chats";
pub const ARCHIVE_PAGE_LIMIT: usize = 256;
const META_BYTES: usize = 64 * 1024;
const BLOB_BYTES: usize = crate::SESSION_LINE_LIMIT_BYTES;
const TURN_LIMIT: usize = crate::SESSION_DISCOVERY_LIMIT;
const STEP_LIMIT: usize = 256;
const QUERY_LIMIT: usize = 512;
const EVENT_LIMIT: usize = 200;
const SQL_STEPS: usize = 1_000_000;
const ANCESTOR_LIMIT: usize = 64;

fn invalid(reason: &'static str) -> SessionError {
    SessionError::Checkpoint(reason.to_owned())
}

fn database_error(reason: &'static str, error: rusqlite::Error) -> SessionError {
    match error {
        rusqlite::Error::SqliteFailure(code, _) => {
            SessionError::Checkpoint(format!("{reason}:sqlite:{}", code.extended_code))
        }
        _ => invalid(reason),
    }
}

fn capacity(resource: &'static str, limit: usize) -> SessionError {
    SessionError::Capacity {
        resource,
        limit: limit as u64,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str, limit: usize) -> Result<Vec<u8>> {
    if text.len() > limit.saturating_mul(2) {
        return Err(capacity("cursor_metadata_bytes", limit));
    }
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(invalid("cursor_hex_invalid"));
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            let high = digit(pair[0]).ok_or_else(|| invalid("cursor_hex_invalid"))?;
            let low = digit(pair[1]).ok_or_else(|| invalid("cursor_hex_invalid"))?;
            Ok(high * 16 + low)
        })
        .collect()
}

fn uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

pub(crate) fn directory(cwd: &Path) -> Result<String> {
    let spelling = cwd
        .to_str()
        .filter(|value| !value.chars().any(char::is_control))
        .filter(|_| cwd.is_absolute())
        .ok_or_else(|| invalid("cursor_cwd_unconfirmed"))?;
    Ok(format!("{:x}", md5::compute(spelling.as_bytes())))
}

pub(crate) fn checked_path(home: &Path, path: &Path) -> Result<PathBuf> {
    let checked = crate::native_file::checked_path_under(home, SESSIONS, path)
        .map_err(|_| invalid("cursor_path_unconfirmed"))?;
    let root = hide_platform::fs::identity::canonical(&home.join(SESSIONS))
        .map_err(|_| invalid("cursor_path_unconfirmed"))?;
    let relative = checked
        .strip_prefix(root)
        .map_err(|_| invalid("cursor_path_unconfirmed"))?;
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 3
        || parts[2].as_os_str() != "store.db"
        || !parts[0].as_os_str().to_str().is_some_and(|name| {
            name.len() == 32
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        || !parts[1].as_os_str().to_str().is_some_and(uuid)
    {
        return Err(invalid("cursor_path_unconfirmed"));
    }
    Ok(checked)
}

// Only fields needed for ownership and the graph enter memory. In particular,
// native title and encryption-key fields are deliberately not deserialized.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    agent_id: String,
    latest_root_blob_id: String,
    created_at: u64,
    #[serde(default)]
    subagent_info: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sidecar {
    schema_version: u32,
    created_at_ms: u64,
    #[serde(default)]
    updated_at_ms: Option<u64>,
    has_conversation: bool,
    #[serde(default)]
    is_subagent: bool,
    cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileMark {
    id: String,
    stamp: String,
}

fn mark(path: &Path) -> Result<FileMark> {
    let file = crate::open_session_file_nofollow(path)
        .map_err(|_| invalid("cursor_source_unavailable"))?;
    if hide_platform::fs::identity::link_count(&file).ok() != Some(1) {
        return Err(invalid("cursor_source_linked"));
    }
    mark_handle(&file)
}

fn mark_handle(file: &std::fs::File) -> Result<FileMark> {
    let id = hide_platform::fs::identity::file_id_of(file)
        .map_err(|_| invalid("cursor_source_unavailable"))?;
    Ok(FileMark {
        id: format!("{}:{}", id.volume(), id.index()),
        stamp: format!(
            "{:?}",
            hide_platform::fs::identity::stamp_of(file)
                .map_err(|_| invalid("cursor_source_unavailable"))?
        ),
    })
}

fn marks(home: &Path, path: &Path) -> Result<Vec<Option<FileMark>>> {
    checked_path(home, path)?;
    let mut result = Vec::with_capacity(5);
    for candidate in [
        path.to_path_buf(),
        path.with_file_name("meta.json"),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
        PathBuf::from(format!("{}-journal", path.display())),
    ] {
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                crate::native_file::checked_path_under(home, SESSIONS, &candidate)
                    .map_err(|_| invalid("cursor_source_linked"))?;
                result.push(Some(mark(&candidate)?));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => result.push(None),
            Err(_) => return Err(invalid("cursor_source_unavailable")),
        }
    }
    if result[0].is_none() || result[1].is_none() {
        return Err(invalid("cursor_source_unavailable"));
    }
    // SQLite can create a shared-memory companion for a read-only WAL source.
    // Do not give the reader that work: only an already initialized WAL is read.
    if result[2].is_some() && result[3].is_none() {
        return Err(invalid("cursor_wal_uninitialized"));
    }
    if result[4].is_some() {
        return Err(invalid("cursor_recovery_required"));
    }
    // HOME is the caller's trusted local root. Fence that anchor and all
    // native descendants; unrelated changes in /tmp or /Users are not source
    // changes. A temporary rename of one of these directories changes its
    // own stamp even when every contained file remains unchanged.
    let anchor = hide_platform::fs::identity::canonical(home)
        .map_err(|_| invalid("cursor_source_unavailable"))?;
    for (index, parent) in path
        .ancestors()
        .skip(1)
        .take_while(|parent| parent.starts_with(&anchor))
        .enumerate()
    {
        if index >= ANCESTOR_LIMIT {
            return Err(capacity("cursor_path_components", ANCESTOR_LIMIT));
        }
        let folder = hide_platform::fs::open_dir_nofollow(parent)
            .map_err(|_| invalid("cursor_source_unavailable"))?;
        result.push(Some(mark_handle(&folder)?));
    }
    Ok(result)
}

struct Database {
    connection: Connection,
    source: std::fs::File,
    cwd_proof: (PathBuf, PathBuf, hide_platform::fs::identity::FileId),
    requested_cwd: Option<PathBuf>,
    home: PathBuf,
    path: PathBuf,
    before: Vec<Option<FileMark>>,
    header: crate::native_file::Header,
    root: Vec<u8>,
    created_at: u64,
    updated_at: Option<u64>,
    read_bytes: usize,
    allowance: usize,
    queries: usize,
}

impl Database {
    fn open(home: &Path, path: &Path, scope: Option<(&str, &str)>, budget: usize) -> Result<Self> {
        let path = checked_path(home, path)?;
        let before = marks(home, &path)?;
        let source = crate::open_session_file_nofollow(&path)
            .map_err(|_| invalid("cursor_source_unavailable"))?;
        if Some(mark_handle(&source)?) != before[0] {
            return Err(invalid("cursor_source_changed"));
        }
        for suffix in ["store.db", "store.db-wal", "store.db-shm"] {
            match std::fs::symlink_metadata(path.with_file_name(suffix)) {
                Ok(metadata) if metadata.len() > crate::SESSION_READ_LIMIT_BYTES => {
                    return Err(capacity(
                        "cursor_database_bytes",
                        crate::SESSION_READ_LIMIT_BYTES as usize,
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(invalid("cursor_source_unavailable")),
            }
        }
        let sidecar_file = crate::open_session_file_nofollow(&path.with_file_name("meta.json"))
            .map_err(|_| invalid("cursor_sidecar_unavailable"))?;
        let mut raw = Vec::new();
        sidecar_file
            .take(META_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| invalid("cursor_sidecar_unavailable"))?;
        if raw.len() > META_BYTES {
            return Err(capacity("cursor_sidecar_bytes", META_BYTES));
        }
        let sidecar: Sidecar =
            serde_json::from_slice(&raw).map_err(|_| invalid("cursor_sidecar_invalid"))?;
        if sidecar.schema_version != 1 || sidecar.is_subagent || sidecar.created_at_ms == 0 {
            return Err(invalid("cursor_root_session_unconfirmed"));
        }
        let bucket = directory(&sidecar.cwd)?;
        if path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            != Some(std::ffi::OsStr::new(&bucket))
        {
            return Err(invalid("cursor_cwd_bucket_mismatch"));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::from_bits_retain(rusqlite::ffi::SQLITE_OPEN_NOFOLLOW);
        // Bundled SQLite's Unix and Windows VFSes implement readonly_shm.
        // READ_ONLY alone still opens an existing wal-index for writes.
        // With no WAL, an immutable connection reads the checkpointed main
        // without creating WAL/SHM companions. The source marks must remain
        // identical before and after: a concurrent native writer is refused.
        let connection = Connection::open_with_flags(
            crate::sqlite::read_uri(&path, before[2].is_some()).map_err(invalid)?,
            flags,
        )
        .map_err(|_| invalid("cursor_database_unavailable"))?;
        // This applies inside SQLite, before a builtin or a row can allocate
        // its result. A VM-instruction limit alone cannot bound randomblob.
        use rusqlite::limits::Limit;
        connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, (BLOB_BYTES + 4096) as i32);
        connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 4096);
        connection.set_limit(Limit::SQLITE_LIMIT_COLUMN, 16);
        connection.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 32);
        connection.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0);
        connection
            .busy_timeout(Duration::from_millis(50))
            .map_err(|_| invalid("cursor_database_unavailable"))?;
        let work = Arc::new(AtomicUsize::new(0));
        connection.progress_handler(
            1000,
            Some(move || work.fetch_add(1000, Ordering::Relaxed) >= SQL_STEPS),
        );
        connection
            .execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-1024; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY; BEGIN")
            .map_err(|_| invalid("cursor_database_unavailable"))?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| database_error("cursor_schema_unconfirmed", error))?;
        if version != 1 {
            return Err(invalid("cursor_schema_unsupported"));
        }
        for (table, key, value, value_type) in [
            ("meta", "key", "value", "TEXT"),
            ("blobs", "id", "data", "BLOB"),
        ] {
            let ordinary: bool = connection.query_row(
                "SELECT count(*)=1 FROM pragma_table_list WHERE schema='main' AND name=?1 AND type='table' AND ncol=2",
                [table], |row| row.get(0),
            ).map_err(|_| invalid("cursor_schema_unconfirmed"))?;
            let columns: bool = connection.query_row(
                "SELECT count(*)=2 FROM pragma_table_xinfo(?1) WHERE hidden=0 AND ((cid=0 AND name=?2 AND type='TEXT' AND pk=1) OR (cid=1 AND name=?3 AND type=?4 AND pk=0))",
                [table, key, value, value_type], |row| row.get(0),
            ).map_err(|_| invalid("cursor_schema_unconfirmed"))?;
            if !ordinary || !columns {
                return Err(invalid("cursor_schema_unconfirmed"));
            }
        }
        let size: usize = connection
            .query_row(
                "SELECT octet_length(value) FROM meta WHERE key='0' AND typeof(value)='text'",
                [],
                |row| row.get(0),
            )
            .map_err(|_| invalid("cursor_metadata_unavailable"))?;
        if size > META_BYTES * 2 {
            return Err(capacity("cursor_metadata_bytes", META_BYTES));
        }
        if size + raw.len() > budget {
            return Err(capacity("cursor_read_bytes", budget));
        }
        let encoded: String = connection
            .query_row("SELECT value FROM meta WHERE key='0'", [], |row| row.get(0))
            .map_err(|_| invalid("cursor_metadata_unavailable"))?;
        let metadata: Metadata = serde_json::from_slice(&unhex(&encoded, META_BYTES)?)
            .map_err(|_| invalid("cursor_metadata_invalid"))?;
        let id = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|id| id.to_str())
            .ok_or_else(|| invalid("cursor_session_id_invalid"))?
            .to_owned();
        if metadata.agent_id != id
            || metadata.subagent_info.is_some()
            || metadata.created_at == 0
            || metadata.created_at != sidecar.created_at_ms
        {
            return Err(invalid("cursor_session_identity_mismatch"));
        }
        let actual = hide_platform::fs::identity::canonical(&sidecar.cwd)
            .map_err(|_| invalid("cursor_cwd_unconfirmed"))?;
        let cwd_identity = hide_platform::fs::identity::file_id(&actual)
            .map_err(|_| invalid("cursor_cwd_unconfirmed"))?;
        if let Some((expected_id, expected_cwd)) = scope {
            let expected = hide_platform::fs::identity::canonical(Path::new(expected_cwd))
                .map_err(|_| invalid("cursor_cwd_unconfirmed"))?;
            if expected_id != id || actual != expected {
                return Err(invalid("cursor_scope_mismatch"));
            }
        }
        let root = unhex(&metadata.latest_root_blob_id, 32)?;
        if (!root.is_empty() && root.len() != 32) || sidecar.has_conversation == root.is_empty() {
            return Err(invalid("cursor_root_unconfirmed"));
        }
        Ok(Self {
            connection,
            source,
            cwd_proof: (sidecar.cwd.clone(), actual, cwd_identity),
            requested_cwd: scope.map(|(_, cwd)| PathBuf::from(cwd)),
            home: home.to_path_buf(),
            path,
            before,
            header: crate::native_file::Header {
                id,
                cwd: sidecar.cwd,
                opened_at_unix_ms: None,
            },
            root,
            created_at: metadata.created_at,
            updated_at: sidecar.updated_at_ms,
            read_bytes: size + raw.len(),
            allowance: budget,
            queries: 4,
        })
    }

    fn blob(&mut self, id: &[u8]) -> Result<Option<Vec<u8>>> {
        if id.len() != 32 {
            return Err(invalid("cursor_blob_reference_invalid"));
        }
        if self.queries + 2 > QUERY_LIMIT {
            return Ok(None);
        }
        self.queries += 2;
        let key = hex(id);
        let size: Option<usize> = self
            .connection
            .query_row(
                "SELECT length(data) FROM blobs WHERE id=?1 AND typeof(data)='blob'",
                [&key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| invalid("cursor_blob_unavailable"))?;
        let size = size.ok_or_else(|| invalid("cursor_blob_missing"))?;
        if size > BLOB_BYTES {
            return Err(capacity("cursor_blob_bytes", BLOB_BYTES));
        }
        if size > self.allowance.saturating_sub(self.read_bytes) {
            return Ok(None);
        }
        let bytes: Vec<u8> = self
            .connection
            .query_row("SELECT data FROM blobs WHERE id=?1", [&key], |row| {
                row.get(0)
            })
            .map_err(|_| invalid("cursor_blob_unavailable"))?;
        self.read_bytes += bytes.len();
        if bytes.len() != size || Sha256::digest(&bytes).as_slice() != id {
            return Err(invalid("cursor_blob_hash_mismatch"));
        }
        Ok(Some(bytes))
    }

    fn required<T: Message + Default>(&mut self, id: &[u8]) -> Result<T> {
        let bytes = self
            .blob(id)?
            .ok_or_else(|| capacity("cursor_read_bytes", self.allowance))?;
        T::decode(bytes.as_slice()).map_err(|_| invalid("cursor_protobuf_invalid"))
    }

    fn finish(&self) -> Result<()> {
        self.connection
            .execute_batch("ROLLBACK")
            .map_err(|_| invalid("cursor_read_failed"))?;
        let (native_cwd, admitted_cwd, admitted_identity) = &self.cwd_proof;
        if hide_platform::fs::identity::canonical(native_cwd)
            .ok()
            .as_ref()
            != Some(admitted_cwd)
            || hide_platform::fs::identity::file_id(native_cwd)
                .ok()
                .as_ref()
                != Some(admitted_identity)
            || self.requested_cwd.as_ref().is_some_and(|cwd| {
                hide_platform::fs::identity::canonical(cwd).ok().as_ref() != Some(admitted_cwd)
            })
            || Some(mark_handle(&self.source)?) != self.before[0]
            || marks(&self.home, &self.path)? != self.before
        {
            return Err(invalid("cursor_source_changed"));
        }
        Ok(())
    }
}

/// A native graph position, boxed by the generic conversation checkpoint.
/// Closed turn refs and consumed current steps are hashed, never retained as
/// transcript content. The last turn stays open so its appended steps survive.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    owner: String,
    incarnation: Option<String>,
    turn: usize,
    closed: String,
    user: Vec<u8>,
    user_read: bool,
    steps: usize,
    step_prefix: String,
    offset: u64,
}

impl Checkpoint {
    pub fn offset(&self) -> u64 {
        self.offset
    }

    fn validate(&self) -> Result<()> {
        let hash = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        if !hash(&self.owner)
            || self
                .incarnation
                .as_ref()
                .is_none_or(|value| value.is_empty() || value.len() > 64)
            || self.turn >= TURN_LIMIT
            || self.steps > STEP_LIMIT
            || self.offset >> 32 != self.turn as u64
            || self.offset & u32::MAX as u64 > (STEP_LIMIT + 1) as u64
            || !matches!(self.user.len(), 0 | 32)
            || (self.user_read
                && (self.user.len() != 32
                    || self.offset & u32::MAX as u64 != (self.steps + 1) as u64))
            || (!self.user_read
                && (!self.user.is_empty()
                    || self.steps != 0
                    || self.offset & u32::MAX as u64 != 0
                    || !self.step_prefix.is_empty()))
            || (self.turn > 0 && !hash(&self.closed))
            || (!self.closed.is_empty() && !hash(&self.closed))
            || (!self.step_prefix.is_empty() && !hash(&self.step_prefix))
        {
            return Err(invalid("cursor_checkpoint_invalid"));
        }
        Ok(())
    }
}

pub struct ReadResult {
    pub parsed: ParsedSession,
    pub checkpoint: Checkpoint,
    pub has_more: bool,
    pub read_bytes: u64,
    pub human_anchors: Vec<(u64, Checkpoint)>,
}

fn prefix(ids: &[Vec<u8>]) -> Result<String> {
    let mut hasher = Sha256::new();
    for id in ids {
        if id.len() != 32 {
            return Err(invalid("cursor_blob_reference_invalid"));
        }
        hasher.update(id);
    }
    Ok(hex(&hasher.finalize()))
}

fn minute(value: Option<u64>) -> u64 {
    value.map(|value| value / 60_000 * 60_000).unwrap_or(0)
}

fn agent_turn(turn: Turn) -> Result<AgentTurn> {
    let agent = match turn.kind {
        Some(turn::Kind::Agent(agent)) if agent.user_message.len() == 32 => agent,
        Some(turn::Kind::Shell(_)) => AgentTurn::default(),
        _ => return Err(invalid("cursor_turn_invalid")),
    };
    if agent.steps.len() > STEP_LIMIT {
        return Err(capacity("cursor_turn_steps", STEP_LIMIT));
    }
    prefix(&agent.steps)?;
    Ok(agent)
}

// A page and an incremental read decode the same native conversation units.
// The outer Option means the bounded transaction needs another read; the
// inner Option means this unit contains no visible conversation text.
fn user_event(db: &mut Database, id: &[u8]) -> Result<Option<Option<ConversationEvent>>> {
    let Some(bytes) = db.blob(id)? else {
        return Ok(None);
    };
    let user = User::decode(bytes.as_slice()).map_err(|_| invalid("cursor_protobuf_invalid"))?;
    let text = if user.text_blob_id.is_empty() {
        user.text
    } else {
        let Some(bytes) = db.blob(&user.text_blob_id)? else {
            return Ok(None);
        };
        String::from_utf8(bytes).map_err(|_| invalid("cursor_text_invalid"))?
    };
    let injected = user.is_simulated_msg.unwrap_or(false) || user.sent_by_agent_id.is_some();
    Ok(Some((!text.is_empty()).then(|| {
        ConversationEvent::new(
            "user",
            if injected {
                EventKind::Injected
            } else {
                EventKind::Human
            },
            minute(user.started_at_ms),
            text,
        )
        .with_provider_injected(injected)
    })))
}

fn step_event(db: &mut Database, id: &[u8]) -> Result<Option<Option<ConversationEvent>>> {
    let Some(bytes) = db.blob(id)? else {
        return Ok(None);
    };
    let step = Step::decode(bytes.as_slice()).map_err(|_| invalid("cursor_protobuf_invalid"))?;
    let event = match step.kind {
        Some(step::Kind::Assistant(message)) if !message.text.is_empty() => {
            Some(ConversationEvent::new(
                "assistant",
                EventKind::Assistant,
                minute(message.started_at_ms),
                message.text,
            ))
        }
        Some(_) => None,
        None => return Err(invalid("cursor_step_invalid")),
    };
    Ok(Some(event))
}

/// Read native conversation units under one transaction and byte allowance.
/// Missing timing is zero (the common unknown-time sentinel); recorded timing
/// is rounded down to a minute. Tools and thoughts never become conversation.
/// `home` is the node-owned stable account root, never a request-supplied path.
/// Callers keep that anchor stable for the read; its native descendants and
/// the still-current checkout scope are independently fenced here.
pub fn read(
    home: &Path,
    path: &Path,
    native_id: &str,
    cwd: &str,
    saved: Option<Checkpoint>,
    budget: u64,
) -> Result<ReadResult> {
    if let Some(checkpoint) = &saved {
        checkpoint.validate()?;
    }
    let budget =
        usize::try_from(budget.min(crate::SESSION_INCREMENT_READ_LIMIT_BYTES)).map_err(|_| {
            capacity(
                "cursor_read_bytes",
                crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
            )
        })?;
    let mut db = Database::open(home, path, Some((native_id, cwd)), budget)?;
    let incarnation = db.before[0].as_ref().map(|mark| mark.id.clone());
    let owner = hex(&Sha256::digest(native_id.as_bytes()));
    let mut parsed = ParsedSession::default();
    let mut human_anchors = Vec::new();
    let mut checkpoint = saved.unwrap_or_default();
    let previous_offset = checkpoint.offset;
    if !checkpoint.owner.is_empty()
        && (checkpoint.owner != owner || checkpoint.incarnation != incarnation)
    {
        parsed.rescan_reason = Some(RescanReason::Replaced);
        checkpoint = Checkpoint::default();
    }
    checkpoint.owner = owner;
    checkpoint.incarnation = incarnation.clone();
    let root_id = db.root.clone();
    let root = if root_id.is_empty() {
        Conversation::default()
    } else {
        db.required::<Conversation>(&root_id)?
    };
    if root.turns.len() > TURN_LIMIT {
        return Err(capacity("cursor_turns", TURN_LIMIT));
    }
    let consistent = checkpoint.turn < root.turns.len()
        && (checkpoint.closed.is_empty()
            || checkpoint.closed == prefix(&root.turns[..checkpoint.turn])?);
    if checkpoint.offset != 0 && !consistent {
        parsed.rescan_reason = Some(RescanReason::Truncated);
        checkpoint = Checkpoint {
            owner: checkpoint.owner,
            incarnation: incarnation.clone(),
            ..Checkpoint::default()
        };
    }
    let mut has_more = false;
    while checkpoint.turn < root.turns.len() {
        let turn_id = &root.turns[checkpoint.turn];
        let Some(bytes) = db.blob(turn_id)? else {
            if checkpoint.offset == 0 {
                return Err(capacity("cursor_read_bytes", budget));
            }
            has_more = true;
            break;
        };
        let turn =
            Turn::decode(bytes.as_slice()).map_err(|_| invalid("cursor_protobuf_invalid"))?;
        let agent = agent_turn(turn)?;
        if checkpoint.steps > agent.steps.len()
            || (checkpoint.user_read && checkpoint.user != agent.user_message)
            || (!checkpoint.step_prefix.is_empty()
                && checkpoint.step_prefix != prefix(&agent.steps[..checkpoint.steps])?)
        {
            // A mutable prior prefix cannot be appended to the existing view.
            parsed = ParsedSession {
                rescan_reason: Some(RescanReason::Truncated),
                ..ParsedSession::default()
            };
            human_anchors.clear();
            checkpoint = Checkpoint {
                owner: checkpoint.owner,
                incarnation: incarnation.clone(),
                ..Checkpoint::default()
            };
            continue;
        }
        if !checkpoint.user_read && !agent.user_message.is_empty() {
            let Some(event) = user_event(&mut db, &agent.user_message)? else {
                has_more = true;
                break;
            };
            if let Some(event) = event {
                if event.kind == EventKind::Human {
                    human_anchors.push((checkpoint.offset, checkpoint.clone()));
                }
                parsed.events.push(event);
                parsed.event_offsets.push(checkpoint.offset);
            }
            checkpoint.offset = checkpoint
                .offset
                .checked_add(1)
                .ok_or_else(|| invalid("cursor_checkpoint_invalid"))?;
            checkpoint.user = agent.user_message;
            checkpoint.user_read = true;
        }
        while checkpoint.steps < agent.steps.len() && parsed.events.len() < EVENT_LIMIT {
            let Some(event) = step_event(&mut db, &agent.steps[checkpoint.steps])? else {
                has_more = true;
                break;
            };
            if let Some(event) = event {
                parsed.events.push(event);
                parsed.event_offsets.push(checkpoint.offset);
            }
            checkpoint.offset = checkpoint
                .offset
                .checked_add(1)
                .ok_or_else(|| invalid("cursor_checkpoint_invalid"))?;
            checkpoint.steps += 1;
        }
        checkpoint.closed = prefix(&root.turns[..checkpoint.turn])?;
        checkpoint.step_prefix = if checkpoint.user_read {
            prefix(&agent.steps[..checkpoint.steps])?
        } else {
            String::new()
        };
        if checkpoint.steps < agent.steps.len() || parsed.events.len() >= EVENT_LIMIT {
            has_more =
                checkpoint.steps < agent.steps.len() || checkpoint.turn + 1 < root.turns.len();
            break;
        }
        if checkpoint.turn + 1 == root.turns.len() {
            break;
        }
        checkpoint.turn += 1;
        checkpoint.offset = (checkpoint.turn as u64) << 32;
        checkpoint.closed = prefix(&root.turns[..checkpoint.turn])?;
        checkpoint.user.clear();
        checkpoint.user_read = false;
        checkpoint.steps = 0;
        checkpoint.step_prefix.clear();
    }
    if has_more && checkpoint.offset == previous_offset && parsed.rescan_reason.is_none() {
        return Err(capacity("cursor_read_bytes", budget));
    }
    db.finish()?;
    Ok(ReadResult {
        parsed,
        checkpoint,
        has_more,
        read_bytes: db.read_bytes as u64,
        human_anchors,
    })
}

pub struct Header {
    pub id: String,
    pub cwd: PathBuf,
    pub created_at: u64,
    pub updated_at: Option<u64>,
}

pub const PAGE_READ_BYTES: u64 = 8 * 1024 * 1024;
pub const PAGE_MESSAGES: usize = 30;
pub const PAGE_TEXT_BYTES: usize = 256 * 1024;

pub struct PageLimits {
    pub messages: usize,
    pub text_bytes: usize,
    pub read_bytes: u64,
}

impl Default for PageLimits {
    fn default() -> Self {
        Self {
            messages: PAGE_MESSAGES,
            text_bytes: PAGE_TEXT_BYTES,
            read_bytes: PAGE_READ_BYTES,
        }
    }
}

pub struct Page {
    pub parsed: ParsedSession,
    pub before: Option<u64>,
    pub tail: Checkpoint,
    pub read_bytes: u64,
}

/// Reads older native units directly from their turn and step references.
/// Ordinals are `(turn << 32) | unit`, where the user is unit zero.
/// The current graph prefix is retained for an incremental tail poll; a
/// rewritten prefix resets that view instead of appending a different past.
pub fn page_before(
    home: &Path,
    path: &Path,
    native_id: &str,
    cwd: &str,
    before: Option<u64>,
    limits: PageLimits,
) -> Result<Page> {
    let budget = usize::try_from(limits.read_bytes.min(PAGE_READ_BYTES))
        .map_err(|_| capacity("cursor_read_bytes", PAGE_READ_BYTES as usize))?;
    let messages = limits.messages.min(PAGE_MESSAGES);
    let text_limit = limits.text_bytes.min(PAGE_TEXT_BYTES);
    if messages == 0 || text_limit == 0 {
        return Err(invalid("cursor_page_limit_invalid"));
    }
    let mut db = Database::open(home, path, Some((native_id, cwd)), budget)?;
    let root_id = db.root.clone();
    let root = if root_id.is_empty() {
        Conversation::default()
    } else {
        db.required::<Conversation>(&root_id)?
    };
    if root.turns.len() > TURN_LIMIT {
        return Err(capacity("cursor_turns", TURN_LIMIT));
    }
    prefix(&root.turns)?;
    let last = if let Some(id) = root.turns.last() {
        agent_turn(db.required::<Turn>(id)?)?
    } else {
        AgentTurn::default()
    };
    let last_turn = root.turns.len().saturating_sub(1);
    let tail = Checkpoint {
        owner: hex(&Sha256::digest(native_id.as_bytes())),
        incarnation: db.before[0].as_ref().map(|mark| mark.id.clone()),
        turn: last_turn,
        closed: prefix(&root.turns[..last_turn])?,
        user: last.user_message.clone(),
        user_read: !last.user_message.is_empty(),
        steps: last.steps.len(),
        step_prefix: if last.user_message.is_empty() {
            String::new()
        } else {
            prefix(&last.steps)?
        },
        offset: ((last_turn as u64) << 32)
            | (last.steps.len() + usize::from(!last.user_message.is_empty())) as u64,
    };
    tail.validate()?;
    let mut parsed = ParsedSession::default();
    let mut text_bytes = 0usize;
    let mut next = None;
    let mut inspected = false;
    if !root.turns.is_empty() {
        let first_turn = before.map(|id| (id >> 32) as usize).unwrap_or(last_turn);
        if first_turn > last_turn {
            return Err(invalid("cursor_page_cursor_invalid"));
        }
        'turns: for index in (0..=first_turn).rev() {
            let agent = if index == last_turn {
                last.clone()
            } else {
                let Some(bytes) = db.blob(&root.turns[index])? else {
                    if !inspected {
                        return Err(capacity("cursor_read_bytes", budget));
                    }
                    next = Some(((index as u64) << 32) | (STEP_LIMIT + 1) as u64);
                    break;
                };
                agent_turn(
                    Turn::decode(bytes.as_slice())
                        .map_err(|_| invalid("cursor_protobuf_invalid"))?,
                )?
            };
            let units = if agent.user_message.is_empty() {
                0
            } else {
                agent.steps.len() + 1
            };
            let upper = if index == first_turn {
                before
                    .map(|id| (id & u32::MAX as u64) as usize)
                    .unwrap_or(units)
            } else {
                units
            };
            let upper = if upper == STEP_LIMIT + 1 {
                units
            } else {
                upper
            };
            if upper > units {
                return Err(invalid("cursor_page_cursor_invalid"));
            }
            for unit in (0..upper).rev() {
                let ordinal = ((index as u64) << 32) | unit as u64;
                let decoded = if unit == 0 {
                    user_event(&mut db, &agent.user_message)?
                } else {
                    step_event(&mut db, &agent.steps[unit - 1])?
                };
                let Some(event) = decoded else {
                    if !inspected {
                        return Err(capacity("cursor_read_bytes", budget));
                    }
                    next = Some(ordinal + 1);
                    break 'turns;
                };
                inspected = true;
                if let Some(event) = event.filter(|event| {
                    event.kind != EventKind::Injected && !event.text.trim().is_empty()
                }) {
                    if !parsed.events.is_empty()
                        && text_bytes.saturating_add(event.text.len()) > text_limit
                    {
                        next = Some(ordinal + 1);
                        break 'turns;
                    }
                    text_bytes += event.text.len();
                    parsed.events.push(event);
                    parsed.event_offsets.push(ordinal);
                    if parsed.events.len() == messages {
                        next = (ordinal > 0).then_some(ordinal);
                        break 'turns;
                    }
                }
            }
        }
    } else if before.is_some_and(|id| id != 0) {
        return Err(invalid("cursor_page_cursor_invalid"));
    }
    parsed.events.reverse();
    parsed.event_offsets.reverse();
    db.finish()?;
    Ok(Page {
        parsed,
        before: next,
        tail,
        read_bytes: db.read_bytes as u64,
    })
}

pub fn header(home: &Path, path: &Path) -> Result<Header> {
    let db = Database::open(
        home,
        path,
        None,
        crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
    )?;
    db.finish()?;
    Ok(Header {
        id: db.header.id,
        cwd: db.header.cwd,
        created_at: minute(Some(db.created_at)),
        updated_at: db.updated_at.map(|time| minute(Some(time))),
    })
}

pub(crate) fn activity(
    home: &Path,
    path: &Path,
    native_id: &str,
    cwd: &str,
) -> Result<crate::session_activity::SessionActivity> {
    let db = Database::open(
        home,
        path,
        Some((native_id, cwd)),
        crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
    )?;
    let mut bytes = 0u64;
    let mut modified_at_unix_ms = 0u64;
    for name in ["store.db", "meta.json", "store.db-wal", "store.db-shm"] {
        let metadata = match std::fs::symlink_metadata(path.with_file_name(name)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(invalid("session_activity_stat_failed")),
        };
        bytes = bytes
            .checked_add(metadata.len())
            .ok_or_else(|| invalid("session_activity_size_invalid"))?;
        let time: u64 = metadata
            .modified()
            .map_err(|_| invalid("session_activity_mtime_unavailable"))?
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_err(|_| invalid("session_activity_mtime_invalid"))?
            .as_millis()
            .try_into()
            .map_err(|_| invalid("session_activity_mtime_invalid"))?;
        modified_at_unix_ms = modified_at_unix_ms.max(time);
    }
    db.finish()?;
    Ok(crate::session_activity::SessionActivity {
        modified_at_unix_ms,
        bytes,
    })
}

pub(crate) fn proof(
    home: &Path,
    path: &Path,
    reported_id: Option<&str>,
    cwd: &str,
) -> Result<crate::ConfirmedLabelSession> {
    let id = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .filter(|value| uuid(value))
        .ok_or_else(|| invalid("cursor_session_id_invalid"))?;
    if reported_id.is_some_and(|reported| reported != id) {
        return Err(invalid("cursor_session_identity_mismatch"));
    }
    let db = Database::open(
        home,
        path,
        Some((id, cwd)),
        crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
    )?;
    let result = crate::ConfirmedLabelSession {
        owner: crate::label_owner::reference_token(crate::Agent::Cursor, "id", id)
            .ok_or_else(|| invalid("cursor_session_id_invalid"))?,
        native_session_id: Some(id.to_owned()),
        source_path: Some(
            db.path
                .to_str()
                .filter(|value| value.len() <= 4096)
                .ok_or_else(|| invalid("cursor_path_unconfirmed"))?
                .to_owned(),
        ),
        incarnation: db.before[0]
            .as_ref()
            .ok_or_else(|| invalid("cursor_source_unavailable"))?
            .id
            .clone(),
        bytes: db
            .source
            .metadata()
            .map_err(|_| invalid("cursor_source_unavailable"))?
            .len(),
    };
    db.finish()?;
    Ok(result)
}

pub(crate) fn confirm_route(home: &Path, path: &Path, id: &str, cwd: &Path) -> Result<()> {
    if !uuid(id) {
        return Err(invalid("cursor_session_id_invalid"));
    }
    let route = home
        .join(SESSIONS)
        .join(directory(cwd)?)
        .join(id)
        .join("store.db");
    if checked_path(home, &route)? != checked_path(home, path)? {
        return Err(invalid("cursor_native_route_mismatch"));
    }
    proof(
        home,
        &route,
        Some(id),
        cwd.to_str()
            .ok_or_else(|| invalid("cursor_cwd_unconfirmed"))?,
    )?;
    confirm_known_roots(home, id)?;
    Ok(())
}

// This is an action-only ambiguity audit, not an execution-environment
// certificate. Never open another root's SQLite store or admit it as a source.
fn confirm_known_roots(home: &Path, id: &str) -> Result<()> {
    let roots = crate::environment::cursor_roots().map_err(invalid)?;
    let default = home.join(".cursor");
    let default_real = hide_platform::fs::identity::canonical(&default)
        .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?;
    let selected = roots
        .config
        .as_ref()
        .or(roots.xdg.as_ref())
        .unwrap_or(&default);
    if hide_platform::fs::identity::canonical(selected)
        .ok()
        .as_ref()
        != Some(&default_real)
    {
        return Err(invalid("cursor_launch_root_unsupported"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut budget = crate::DiscoveryBudget::default();
    let mut owners = 0;
    for root in [Some(&default), roots.config.as_ref(), roots.xdg.as_ref()]
        .into_iter()
        .flatten()
    {
        let real = match hide_platform::fs::identity::canonical(root) {
            Ok(real) => real,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(invalid("cursor_launch_root_unconfirmed")),
        };
        if seen.insert(real.clone()) && root_contains(&real, id, &mut budget)? {
            owners += 1;
            if owners > 1 {
                return Err(invalid("cursor_launch_root_ambiguous"));
            }
        }
    }
    if owners != 1 {
        return Err(invalid("cursor_launch_root_unconfirmed"));
    }
    Ok(())
}

fn root_contains(root: &Path, id: &str, budget: &mut crate::DiscoveryBudget) -> Result<bool> {
    let chats = root.join("chats");
    if !chats
        .try_exists()
        .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?
    {
        return Ok(false);
    }
    let mut opened = Vec::new();
    let fence = |path: &Path,
                 opened: &mut Vec<(PathBuf, std::fs::File, FileMark)>,
                 budget: &mut crate::DiscoveryBudget|
     -> Result<()> {
        budget.directory()?;
        crate::native_file::checked_path_under(root, "", path)
            .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?;
        let folder = hide_platform::fs::open_dir_nofollow(path)
            .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?;
        let before = mark_handle(&folder)?;
        opened.push((path.to_owned(), folder, before));
        Ok(())
    };
    fence(root, &mut opened, budget)?;
    fence(&chats, &mut opened, budget)?;
    let mut found = false;
    for bucket in crate::read_directory(&chats, budget).map_err(|error| match error {
        SessionError::Capacity { .. } => error,
        _ => invalid("cursor_launch_root_unconfirmed"),
    })? {
        if !bucket
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.len() == 32
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            continue;
        }
        fence(&bucket, &mut opened, budget)?;
        let candidate = bucket.join(id);
        if candidate
            .try_exists()
            .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?
        {
            fence(&candidate, &mut opened, budget)?;
            found = true;
            break;
        }
    }
    for (path, handle, before) in opened {
        let current = hide_platform::fs::open_dir_nofollow(&path)
            .map_err(|_| invalid("cursor_launch_root_unconfirmed"))?;
        if mark_handle(&handle)? != before || mark_handle(&current)? != before {
            return Err(invalid("cursor_launch_root_changed"));
        }
    }
    Ok(found)
}

pub(crate) fn locate(
    home: &Path,
    identity: Option<&crate::SessionIdentity>,
    cwd: Option<&str>,
    budget: &mut crate::DiscoveryBudget,
) -> Result<PathBuf> {
    let cwd = cwd.ok_or(crate::SessionError::CwdUnavailable)?;
    if let Some(crate::SessionIdentity::Path(path)) = identity {
        proof(home, path, None, cwd)?;
        return checked_path(home, path);
    }
    let directory = home.join(SESSIONS).join(directory(Path::new(cwd))?);
    let candidates = match identity {
        Some(crate::SessionIdentity::Id(id)) if uuid(id) => {
            vec![directory.join(id).join("store.db")]
        }
        Some(_) => return Err(invalid("cursor_session_id_invalid")),
        None => crate::read_directory(&directory, budget)?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(uuid)
            })
            .map(|path| path.join("store.db"))
            .collect(),
    };
    let mut remaining = crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize;
    let mut latest = None;
    for path in candidates {
        let db = match Database::open(home, &path, None, remaining) {
            Ok(db) => db,
            Err(SessionError::Capacity { .. }) => {
                return Err(capacity(
                    "discovery_read_bytes",
                    crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
                ));
            }
            Err(_) if identity.is_none() => continue,
            Err(error) => return Err(error),
        };
        remaining = remaining.saturating_sub(db.read_bytes);
        if hide_platform::fs::identity::canonical(&db.header.cwd).ok()
            != hide_platform::fs::identity::canonical(Path::new(cwd)).ok()
        {
            return Err(invalid("cursor_scope_mismatch"));
        }
        db.finish()?;
        let time = db.updated_at.unwrap_or(db.created_at);
        if latest.as_ref().is_none_or(|(previous, _)| time > *previous) {
            latest = Some((time, db.path.clone()));
        }
    }
    latest
        .map(|(_, path)| path)
        .ok_or(crate::SessionError::SessionFileMissing)
}

/// Metadata-only change token. This is an observation, never read authority.
pub(crate) fn source_stamp(path: &Path) -> Option<String> {
    let mut digest = Sha256::new();
    for name in [
        "store.db",
        "meta.json",
        "store.db-wal",
        "store.db-shm",
        "store.db-journal",
    ] {
        let candidate = path.with_file_name(name);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => digest.update(format!("{name}:{:?}", mark(&candidate).ok()?).as_bytes()),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && name != "store.db"
                    && name != "meta.json" =>
            {
                digest.update(format!("{name}:absent").as_bytes())
            }
            Err(_) => return None,
        }
    }
    Some(format!("cursor:{:x}", digest.finalize()))
}

pub fn read_all(
    home: &Path,
    path: &Path,
    scope: &crate::SessionReadScope,
) -> Result<ParsedSession> {
    let stamp = source_stamp(path).ok_or_else(|| invalid("cursor_source_unavailable"))?;
    let mut parsed = ParsedSession::default();
    let mut saved = None;
    let mut read_bytes = 0;
    let mut text_bytes = 0;
    let mut pages = 0;
    loop {
        pages += 1;
        if pages > ARCHIVE_PAGE_LIMIT {
            return Err(capacity("cursor_archive_pages", ARCHIVE_PAGE_LIMIT));
        }
        let read = read(
            home,
            path,
            &scope.id,
            &scope.cwd,
            saved,
            crate::SESSION_INCREMENT_READ_LIMIT_BYTES,
        )?;
        if read.parsed.rescan_reason.is_some() {
            return Err(invalid("cursor_source_changed"));
        }
        read_bytes += read.read_bytes;
        text_bytes += read
            .parsed
            .events
            .iter()
            .map(|event| event.text.len() as u64)
            .sum::<u64>();
        if read_bytes > crate::SESSION_READ_LIMIT_BYTES
            || text_bytes > crate::SESSION_READ_LIMIT_BYTES
        {
            return Err(capacity(
                "cursor_archive_bytes",
                crate::SESSION_READ_LIMIT_BYTES as usize,
            ));
        }
        if parsed.events.len() + read.parsed.events.len() > TURN_LIMIT {
            return Err(capacity("cursor_archive_events", TURN_LIMIT));
        }
        parsed.events.extend(read.parsed.events);
        parsed.event_offsets.extend(read.parsed.event_offsets);
        if !read.has_more {
            break;
        }
        saved = Some(read.checkpoint);
    }
    if source_stamp(path).as_deref() != Some(&stamp) {
        return Err(invalid("cursor_source_changed"));
    }
    Ok(parsed)
}

// Field numbers come from the installed native agent/v1 generated schema.
// Prost skips unowned fields, including subagent maps, with its recursion cap.
#[derive(Clone, PartialEq, Message)]
struct Conversation {
    #[prost(bytes = "vec", repeated, tag = "8")]
    turns: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct Turn {
    #[prost(oneof = "turn::Kind", tags = "1, 2")]
    kind: Option<turn::Kind>,
}
mod turn {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        Agent(super::AgentTurn),
        #[prost(message, tag = "2")]
        Shell(super::Ignored),
    }
}

#[derive(Clone, PartialEq, Message)]
struct AgentTurn {
    #[prost(bytes = "vec", tag = "1")]
    user_message: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "2")]
    steps: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, Message)]
struct User {
    #[prost(string, tag = "1")]
    text: String,
    #[prost(bool, optional, tag = "5")]
    is_simulated_msg: Option<bool>,
    #[prost(bytes = "vec", tag = "18")]
    text_blob_id: Vec<u8>,
    #[prost(uint64, optional, tag = "25")]
    started_at_ms: Option<u64>,
    #[prost(string, optional, tag = "27")]
    sent_by_agent_id: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct Step {
    #[prost(oneof = "step::Kind", tags = "1, 2, 3")]
    kind: Option<step::Kind>,
}
mod step {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        Assistant(super::Assistant),
        #[prost(message, tag = "2")]
        Tool(super::Ignored),
        #[prost(message, tag = "3")]
        Thinking(super::Ignored),
    }
}

#[derive(Clone, PartialEq, Message)]
struct Assistant {
    #[prost(string, tag = "1")]
    text: String,
    #[prost(uint64, optional, tag = "2")]
    started_at_ms: Option<u64>,
}

#[derive(Clone, PartialEq, Message)]
struct Ignored {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_ancestor_names_do_not_restore_read_authority() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        let id = "a1b2c3d4-0000-4000-8000-000000000001";
        let path = home
            .path()
            .join(SESSIONS)
            .join(directory(&cwd).unwrap())
            .join(id)
            .join("store.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=1; CREATE TABLE blobs(id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);").unwrap();
        let metadata = serde_json::json!({"agentId":id,"latestRootBlobId":"","createdAt":1});
        connection
            .execute(
                "INSERT INTO meta VALUES ('0',?1)",
                [hex(metadata.to_string().as_bytes())],
            )
            .unwrap();
        drop(connection);
        std::fs::write(path.with_file_name("meta.json"), serde_json::json!({"schemaVersion":1,"createdAtMs":1,"hasConversation":false,"cwd":cwd}).to_string()).unwrap();
        let database = Database::open(
            home.path(),
            &path,
            None,
            crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
        )
        .unwrap();
        let original_file = mark(&path).unwrap();
        let folder = path.parent().unwrap();
        let saved = folder.with_file_name("saved");
        std::fs::rename(folder, &saved).unwrap();
        std::fs::create_dir(folder).unwrap();
        std::fs::remove_dir(folder).unwrap();
        std::fs::rename(&saved, folder).unwrap();
        assert_eq!(mark(&path).unwrap(), original_file);
        assert!(database.finish().is_err());
        #[cfg(unix)]
        {
            let aliases = tempfile::tempdir().unwrap();
            let alias = aliases.path().join("checkout");
            std::os::unix::fs::symlink(&cwd, &alias).unwrap();
            let database = Database::open(
                home.path(),
                &path,
                Some((id, alias.to_str().unwrap())),
                crate::SESSION_INCREMENT_READ_LIMIT_BYTES as usize,
            )
            .unwrap();
            let other = aliases.path().join("other");
            std::fs::create_dir(&other).unwrap();
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&other, &alias).unwrap();
            assert!(database.finish().is_err());
        }
    }
}
