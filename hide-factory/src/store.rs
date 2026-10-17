//! The Factory store: one SQLite file (`factory.sqlite3`, WAL) in the state
//! folder, the source of truth for every Factory on this machine (D-03), and
//! a private folder beside it for PRD copies and verify logs.
//!
//! The schema is versioned with `PRAGMA user_version`; a newer file than this
//! build knows is refused rather than read.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Attachment, Factory, Task, UnixMs};

/// 2 since the activity log replaced notices (D-39); a schema 1 store is
/// moved forward once, on first open, after a copy of it is kept beside it
/// as [`V1_COPY`].
pub const SCHEMA_VERSION: i64 = 2;
/// The schema 1 store as it was before the move, never read again.
pub const V1_COPY: &str = "factory.v1.sqlite3";
/// A PRD attachment larger than this is refused.
/// Events kept per Factory; the oldest are dropped past it.
pub const EVENT_LIMIT: i64 = 20_000;
/// A kept judgment or letter body is cut to its first this many bytes.
pub const RECORD_LIMIT: usize = 256 * 1024;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS factories (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL UNIQUE,
    data TEXT NOT NULL,
    closed INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    factory TEXT NOT NULL REFERENCES factories(id),
    id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    issue TEXT,
    state TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    data TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (factory, id)
);
CREATE INDEX IF NOT EXISTS tasks_issue ON tasks(factory, issue);
CREATE TABLE IF NOT EXISTS dependencies (
    factory TEXT NOT NULL,
    task TEXT NOT NULL,
    on_task TEXT NOT NULL,
    PRIMARY KEY (factory, task, on_task)
);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    factory TEXT NOT NULL,
    task TEXT,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_factory ON events(factory, seq);
CREATE TABLE IF NOT EXISTS records (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    factory TEXT NOT NULL,
    task TEXT,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    reference TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS records_task ON records(factory, task, seq);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self(format!("sqlite: {error}"))
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self(format!("encoding: {error}"))
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self(format!("io: {error}"))
    }
}

/// One recorded transition, failure or external call (B77): ids and stage
/// only, never card text or secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub factory: String,
    pub task: Option<String>,
    pub at: UnixMs,
    pub kind: String,
    pub detail: serde_json::Value,
}

/// A judgment's input or output, or a letter body, kept with its Task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub factory: String,
    pub task: Option<String>,
    pub at: UnixMs,
    /// `judgment.input`, `judgment.output`, `letter.in` or `letter.out`.
    pub kind: String,
    /// The judgment or letter id.
    pub reference: String,
    pub body: String,
}

pub struct Store {
    connection: Connection,
    files: PathBuf,
}

/// Everything the engine loads at start (B72).
#[derive(Default)]
pub struct Loaded {
    pub factories: Vec<Factory>,
    pub tasks: Vec<Task>,
    pub meta: Vec<(String, String)>,
}

impl Store {
    /// Opens or creates the store at `path` with its private files folder
    /// (0700 on Unix); `hide_kit::layout` names both.
    pub fn open(path: &Path, files: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            hide_platform::fs::private::create_dir_all(parent)?;
        }
        let files = files.to_owned();
        hide_platform::fs::private::create_dir_all(&files)?;
        private_dir(&files)?;
        let connection = Connection::open(path)?;
        private_file(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError(format!(
                "store schema {version} is newer than this build ({SCHEMA_VERSION})"
            )));
        }
        let mut connection = connection;
        if version == 1 {
            migrate_v1(&mut connection, path)?;
        }
        connection.execute_batch(SCHEMA)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { connection, files })
    }

    pub fn files_dir(&self) -> &Path {
        &self.files
    }

    pub fn load(&self) -> Result<Loaded, StoreError> {
        let mut loaded = Loaded::default();
        let mut statement = self
            .connection
            .prepare("SELECT data FROM factories ORDER BY created_at, id")?;
        for row in statement.query_map([], |row| row.get::<_, String>(0))? {
            loaded.factories.push(serde_json::from_str(&row?)?);
        }
        let mut statement = self
            .connection
            .prepare("SELECT data FROM tasks ORDER BY factory, seq")?;
        for row in statement.query_map([], |row| row.get::<_, String>(0))? {
            loaded.tasks.push(serde_json::from_str(&row?)?);
        }
        let mut statement = self.connection.prepare("SELECT key, value FROM meta")?;
        for row in statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))? {
            loaded.meta.push(row?);
        }
        Ok(loaded)
    }

    pub fn put_factory(&self, factory: &Factory) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO factories (id, project, data, closed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET project = excluded.project, data = excluded.data, closed = excluded.closed",
            params![
                factory.id,
                factory.project,
                serde_json::to_string(factory)?,
                factory.closed,
                factory.created_at as i64
            ],
        )?;
        Ok(())
    }

    /// Saves a Task and its dependency rows in one transaction.
    pub fn put_task(&mut self, task: &Task) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO tasks (factory, id, seq, issue, state, priority, data, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(factory, id) DO UPDATE SET issue = excluded.issue, state = excluded.state,
               priority = excluded.priority, data = excluded.data, updated_at = excluded.updated_at",
            params![
                task.factory,
                task.id,
                task.seq,
                task.issue.as_ref().map(|issue| issue.display()),
                task.state.as_str(),
                task.human.priority,
                serde_json::to_string(task)?,
                task.created_at as i64,
                task.updated_at as i64
            ],
        )?;
        transaction.execute(
            "DELETE FROM dependencies WHERE factory = ?1 AND task = ?2",
            params![task.factory, task.id],
        )?;
        for on in &task.card.depends_on {
            transaction.execute(
                "INSERT OR IGNORE INTO dependencies (factory, task, on_task) VALUES (?1, ?2, ?3)",
                params![task.factory, task.id, on],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Renames the machines a core move changes (PRD core-host-node-move
    /// Q17): each Factory's and worker's node through `machine`, where `None`
    /// is the core's own, and each pane a Task names through `pane`, in one
    /// transaction. Answers whether anything changed.
    pub fn reown(
        &mut self,
        machine: &dyn Fn(Option<&str>) -> Option<String>,
        pane: &dyn Fn(&str) -> String,
    ) -> Result<bool, StoreError> {
        let loaded = self.load()?;
        let transaction = self.connection.transaction()?;
        let mut changed = false;
        for mut factory in loaded.factories {
            let node = machine(factory.node.as_deref());
            if node == factory.node {
                continue;
            }
            factory.node = node;
            transaction.execute(
                "UPDATE factories SET data = ?2 WHERE id = ?1",
                params![factory.id, serde_json::to_string(&factory)?],
            )?;
            changed = true;
        }
        for mut task in loaded.tasks {
            let before = (task.worker.clone(), task.producer_pane.clone());
            if let Some(worker) = &mut task.worker {
                worker.node = machine(worker.node.as_deref());
                worker.pane = worker.pane.as_deref().map(pane);
            }
            task.producer_pane = task.producer_pane.as_deref().map(pane);
            if (&task.worker, &task.producer_pane) == (&before.0, &before.1) {
                continue;
            }
            transaction.execute(
                "UPDATE tasks SET data = ?3 WHERE factory = ?1 AND id = ?2",
                params![task.factory, task.id, serde_json::to_string(&task)?],
            )?;
            changed = true;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn append_event(&self, event: &Event) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO events (factory, task, at, kind, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.factory,
                event.task,
                event.at as i64,
                event.kind,
                event.detail.to_string()
            ],
        )?;
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM events WHERE factory = ?1",
            params![event.factory],
            |row| row.get(0),
        )?;
        if count > EVENT_LIMIT {
            self.connection.execute(
                "DELETE FROM events WHERE seq IN (SELECT seq FROM events WHERE factory = ?1 ORDER BY seq LIMIT ?2)",
                params![event.factory, count - EVENT_LIMIT],
            )?;
        }
        Ok(())
    }

    pub fn events(
        &self,
        factory: &str,
        task: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Event>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT factory, task, at, kind, detail FROM events
             WHERE factory = ?1 AND (?2 IS NULL OR task = ?2) ORDER BY seq DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![factory, task, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (factory, task, at, kind, detail) = row?;
            events.push(Event {
                factory,
                task,
                at: at as u64,
                kind,
                detail: serde_json::from_str(&detail)?,
            });
        }
        events.reverse();
        Ok(events)
    }

    pub fn factory_for_project(&self, project: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM factories WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Keeps a PRD, read from `source` on the node, in the private folder
    /// under its hash (D-10, B16).
    pub fn attach(
        &self,
        factory: &str,
        task: &str,
        source: &Path,
        bytes: &[u8],
        version: u32,
    ) -> Result<Attachment, StoreError> {
        let digest = hex(&Sha256::digest(bytes));
        let folder = self.files.join(factory).join(task);
        hide_platform::fs::private::create_dir_all(&folder)?;
        private_dir(&folder)?;
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| value.len() <= 8 && value.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("md");
        let path = folder.join(format!("{digest}.{extension}"));
        if !path.exists() {
            write_private(&path, bytes)?;
            read_only(&path)?;
        }
        Ok(Attachment {
            path: path.to_string_lossy().into_owned(),
            sha256: digest,
            version,
            original: source.to_string_lossy().into_owned(),
        })
    }

    /// Keeps a judgment's input or output or a letter's body (D-58); none
    /// is deleted automatically.
    pub fn keep_record(&self, record: &Record) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO records (factory, task, at, kind, reference, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.factory,
                record.task,
                record.at as i64,
                record.kind,
                record.reference,
                crate::judgment::cut(&record.body, RECORD_LIMIT)
            ],
        )?;
        Ok(())
    }

    /// A Task's kept records, oldest first, at most `limit` of the newest.
    pub fn records(
        &self,
        factory: &str,
        task: &str,
        limit: usize,
    ) -> Result<Vec<Record>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT factory, task, at, kind, reference, body FROM records
             WHERE factory = ?1 AND task = ?2 ORDER BY seq DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![factory, task, limit as i64], |row| {
            Ok(Record {
                factory: row.get(0)?,
                task: row.get(1)?,
                at: row.get::<_, i64>(2)? as u64,
                kind: row.get(3)?,
                reference: row.get(4)?,
                body: row.get(5)?,
            })
        })?;
        let mut records = rows.collect::<Result<Vec<_>, _>>()?;
        records.reverse();
        Ok(records)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// An issue body's identity: line endings and surrounding blank space do
/// not make an edit, so the body the Factory wrote matches what GitHub
/// answers for it.
pub fn body_hash(body: &str) -> String {
    sha256_hex(body.replace("\r\n", "\n").trim().as_bytes())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = path.with_extension("partial");
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

fn private_dir(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn private_file(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

fn read_only(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// Moves a schema 1 store to schema 2 (D-39): first a copy of it is kept
/// beside it as [`V1_COPY`], then every record moves in one transaction, so
/// a failure leaves the store as it was and refuses to open.
fn migrate_v1(connection: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let copy = path.with_file_name(V1_COPY);
    // A copy an earlier, failed move made is the store as it was, since that
    // move changed nothing.
    if !copy.exists() {
        connection
            .execute(
                "VACUUM INTO ?1",
                params![copy.to_string_lossy().into_owned()],
            )
            .map_err(|error| StoreError(format!("migration copy: {error}")))?;
        private_file(&copy)?;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as UnixMs);
    let transaction = connection.transaction()?;
    let tasks: Vec<(String, String, String)> = {
        let mut statement = transaction.prepare("SELECT factory, id, data FROM tasks")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut proposals: std::collections::BTreeMap<String, Vec<crate::migrate::Proposal>> =
        std::collections::BTreeMap::new();
    for (factory, id, data) in tasks {
        let (task, asked) = crate::migrate::task(serde_json::from_str(&data)?, now);
        // The moved record must load as this build's Task.
        let typed: Task = serde_json::from_value(task.clone())
            .map_err(|error| StoreError(format!("migration of {factory}/{id}: {error}")))?;
        transaction.execute(
            "UPDATE tasks SET data = ?3 WHERE factory = ?1 AND id = ?2",
            params![factory, id, serde_json::to_string(&typed)?],
        )?;
        proposals.entry(factory).or_default().extend(asked);
    }
    let factories: Vec<(String, String)> = {
        let mut statement = transaction.prepare("SELECT id, data FROM factories")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, data) in factories {
        let asked = proposals.remove(&id).unwrap_or_default();
        let factory = crate::migrate::factory(serde_json::from_str(&data)?, &asked);
        let typed: Factory = serde_json::from_value(factory)
            .map_err(|error| StoreError(format!("migration of {id}: {error}")))?;
        transaction.execute(
            "UPDATE factories SET data = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(&typed)?],
        )?;
        transaction.execute(
            "INSERT INTO events (factory, task, at, kind, detail) VALUES (?1, NULL, ?2, 'store.migrated', ?3)",
            params![id, now as i64, serde_json::json!({"from": 1, "to": SCHEMA_VERSION}).to_string()],
        )?;
    }
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_schema_is_refused_rather_than_read() {
        let folder = tempfile::tempdir().unwrap();
        drop(
            Store::open(
                &folder.path().join("factory.sqlite3"),
                &folder.path().join("factory-files"),
            )
            .unwrap(),
        );
        let connection = Connection::open(folder.path().join("factory.sqlite3")).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        let error = Store::open(
            &folder.path().join("factory.sqlite3"),
            &folder.path().join("factory-files"),
        )
        .err()
        .unwrap();
        assert!(error.0.contains("newer"), "{error}");
    }

    #[test]
    fn records_are_kept_per_task_and_a_long_body_is_cut() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(
            &folder.path().join("factory.sqlite3"),
            &folder.path().join("factory-files"),
        )
        .unwrap();
        let record = |task: &str, body: String| Record {
            factory: "f".into(),
            task: Some(task.into()),
            at: 1,
            kind: "letter.in".into(),
            reference: "letter-1".into(),
            body,
        };
        store.keep_record(&record("T-1", "first".into())).unwrap();
        store.keep_record(&record("T-2", "other".into())).unwrap();
        store
            .keep_record(&record("T-1", "x".repeat(RECORD_LIMIT + 10)))
            .unwrap();
        let kept = store.records("f", "T-1", 10).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].body, "first");
        assert!(kept[1].body.len() <= RECORD_LIMIT + 32);
        assert!(kept[1].body.ends_with("[cut 10 bytes]"));
    }

    #[test]
    fn an_attachment_is_a_private_read_only_copy_named_by_its_hash() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(
            &folder.path().join("state/factory.sqlite3"),
            &folder.path().join("state/factory-files"),
        )
        .unwrap();
        let source = folder.path().join("prd.md");
        let attachment = store.attach("f", "T-1", &source, b"# PRD\n", 1).unwrap();
        assert_eq!(attachment.sha256, sha256_hex(b"# PRD\n"));
        assert!(
            attachment
                .path
                .ends_with(&format!("{}.md", attachment.sha256))
        );
        let copy = Path::new(&attachment.path);
        assert!(fs::metadata(copy).unwrap().permissions().readonly());
        let second = store.attach("f", "T-1", &source, b"# PRD v2\n", 2).unwrap();
        assert_ne!(second.path, attachment.path);
        assert_eq!(fs::read_to_string(copy).unwrap(), "# PRD\n");
    }
}
