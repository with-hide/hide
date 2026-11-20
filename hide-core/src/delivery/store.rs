//! The delivery ledger at rest (PRD letters-missions-simplify D-22, D-31):
//! one SQLite file in the state folder, `delivery.sqlite3`
//! (`hide_kit::layout::delivery_store`), opened the way the link record is
//! (`links::store`): WAL, `secure_delete`, a page cap and a schema version.
//!
//! The core decides on the ledger in memory, as it did when the ledger was
//! one JSON file; this file only keeps it. Each letter, agent registration
//! and spawn record is a row of its own table, the record's JSON encoding
//! keyed by its number in the ledger's one id sequence, so rows read back in
//! the order they were made. The rest of the ledger (its next id, watches,
//! identity waits and declared turns, each bounded by the
//! records) is one row. A write is the rows that changed between the ledger
//! the file holds and the one to keep, in one transaction
//! ([`LedgerStore::write`]): a sent letter is one new row, not a new file.
//!
//! What the file may hold is decided in memory too: the ledger keeps within
//! [`FILE_LIMIT`] by dropping finished records oldest first
//! (`coordination::room::for_size`). The page cap ([`STORE_PAGE_LIMIT`]) sits
//! above everything that rule lets through, so it only stops a write the rule
//! did not bound; that write fails as any other failed write does.
//!
//! The first open of a folder an older build wrote imports its
//! `delivery-ledger.json` in the transaction that makes the tables (D-08,
//! D-20): a start cut short before it commits imports again, and the old file
//! is only ever read, so an older build started on the same folder finds the
//! ledger as it was before the update (B27). A file of the first version,
//! whose letters were addressed to agents rather than their registrations,
//! is brought to this one in one transaction (`upgrade`, PRD
//! letters-missions-simplify B25), as is the ledger an import reads. A file that fails its integrity
//! check, or that a newer build made, is refused and kept as it is, never
//! made again by guess: the core then refuses every letter until it is fixed,
//! as it did for a JSON file it could not read.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ledger::{Ledger, Letter, SaveError, StoreFailure};
use super::watch::Watch;
use super::{FILE_LIMIT, LETTER_LIMIT};
use crate::coordination::human_turn::HumanTurn;
use crate::coordination::identity_wait::IdentityWait;
use crate::coordination::{AGENT_LIMIT, AgentRecord, SPAWN_LIMIT, SpawnRecord};

/// The tables this build makes and reads. A file of a higher version was
/// made by a newer build and is refused; one of version 1 is upgraded
/// (`upgrade`).
pub const SCHEMA_VERSION: i64 = 2;

const PAGE_BYTES: usize = 4_096;

/// The pages the file may take: every byte the in-memory limit lets the
/// records encode to, plus one page for each record a row can hold at worst
/// (a row a little past a page leaves most of its last page empty), twice
/// over for the tables' inner pages and the pages a write frees for the next.
pub const STORE_PAGE_LIMIT: u32 =
    (2 * (FILE_LIMIT / PAGE_BYTES + LETTER_LIMIT + AGENT_LIMIT + SPAWN_LIMIT + 1)) as u32;

/// How much of the write-ahead log is kept after a checkpoint: the import of
/// a large ledger writes one long transaction, and its log would otherwise
/// stay that long.
const LOG_KEPT_BYTES: usize = 4 * 1_024 * 1_024;

const SCHEMA: &str = "\
    CREATE TABLE ledger (only INTEGER PRIMARY KEY CHECK (only = 1), body TEXT NOT NULL);\
    CREATE TABLE letters (seq INTEGER PRIMARY KEY, id TEXT NOT NULL, body TEXT NOT NULL);\
    CREATE TABLE agents (seq INTEGER PRIMARY KEY, id TEXT NOT NULL, body TEXT NOT NULL);\
    CREATE TABLE spawns (seq INTEGER PRIMARY KEY, id TEXT NOT NULL, body TEXT NOT NULL);";

/// How an open found the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// The file held this build's tables.
    Existing,
    /// The file was made, holding what `delivery-ledger.json` held.
    Imported,
    /// The file was made empty: there was no `delivery-ledger.json`.
    Fresh,
    /// The file held version 1's tables, brought to this build's.
    Upgraded,
}

/// The writer's connection to the ledger's file. One per open; the core's
/// store thread holds one for as long as it runs (`delivery::worker`).
pub struct LedgerStore {
    connection: Connection,
}

impl LedgerStore {
    /// Opens the file at `path`, making it, private, when it is not there,
    /// and answers the ledger it holds. A file without this build's tables
    /// gets them in one transaction with what `delivery-ledger.json` beside
    /// it holds, which is read and never written.
    pub fn open(path: &Path) -> Result<(Self, Ledger, Opened), String> {
        let (store, opened) = Self::open_file(path)?;
        let ledger = read_from(&store.connection).map_err(|code| refused(path, &code))?;
        Ok((store, ledger, opened))
    }

    /// The file at `path` with this build's tables, as [`Self::open`] makes
    /// it, without reading the ledger.
    fn open_file(path: &Path) -> Result<(Self, Opened), String> {
        let existed = path.exists();
        if existed && !hide_platform::fs::private::is_private(path).unwrap_or(false) {
            return Err(refused(path, "not_private"));
        }
        hide_platform::fs::private::open_or_create_file(path)
            .map_err(|_| refused(path, "create_failed"))?;
        let mut connection = Connection::open(path).map_err(|error| failed(path, &error))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(|error| failed(path, &error))?;
        // Every commit is on the disk before the core answers for it: a
        // letter answered as sent was kept (B29).
        connection
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; \
                 PRAGMA max_page_count={STORE_PAGE_LIMIT}; \
                 PRAGMA journal_size_limit={LOG_KEPT_BYTES};"
            ))
            .map_err(|error| failed(path, &error))?;
        let check: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|error| failed(path, &error))?;
        if check != "ok" {
            return Err(refused(path, "corrupt"));
        }
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| failed(path, &error))?;
        let opened = match version {
            SCHEMA_VERSION => Opened::Existing,
            0 => import(&mut connection, path)?,
            1 => upgrade(&mut connection, path)?,
            _ => return Err(refused(path, "newer")),
        };
        Ok((Self { connection }, opened))
    }

    /// Keeps `after` in place of `before`, the ledger the file holds: the
    /// rows that differ between them, in one transaction.
    pub fn write(&mut self, before: &Ledger, after: &Ledger) -> Result<(), SaveError> {
        self.apply(&Changes::between(before, after))
    }

    /// Writes `changes` in one transaction. A failure before the commit
    /// leaves the file as it was; a failed commit may or may not have kept
    /// them, and says so ([`SaveError::uncertain`]).
    pub fn apply(&mut self, changes: &Changes<'_>) -> Result<(), SaveError> {
        if changes.is_empty() {
            return Ok(());
        }
        let refused = |error: rusqlite::Error| SaveError::Refused(StoreFailure::of(&error));
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(refused)?;
        changes.write(&transaction)?;
        transaction
            .commit()
            .map_err(|error| SaveError::Uncertain(StoreFailure::of(&error)))
    }

    /// A consistent copy of the whole file at `destination`, which must not
    /// exist yet: the writer runs in WAL mode, so the main file alone may
    /// miss committed rows.
    pub fn copy_to(&self, destination: &Path) -> Result<(), String> {
        let destination = destination.to_str().ok_or("ledger_copy_path_not_utf8")?;
        self.connection
            .execute("VACUUM INTO ?1", [destination])
            .map(|_| ())
            .map_err(|error| code(&error).to_owned())
    }
}

/// Makes this build's tables in a file that has none, holding what
/// `delivery-ledger.json` beside `path` holds, in one transaction.
fn import(connection: &mut Connection, path: &Path) -> Result<Opened, String> {
    let legacy = legacy_path(path);
    let imported = match super::ledger::read_file(&legacy) {
        Ok(ledger) => ledger,
        Err(code) => {
            crate::diagnostic!(
                json!({"component":"delivery","kind":"ledger.import_refused",
                "code":code})
            );
            return Err(code);
        }
    };
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| failed(path, &error))?;
    transaction
        .execute_batch(SCHEMA)
        .map_err(|error| failed(path, &error))?;
    let mut ledger = imported.clone().unwrap_or_default();
    upgraded(&mut ledger, 0);
    Changes::all(&ledger)
        .write(&transaction)
        .map_err(|error| refused(path, error.code()))?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| failed(path, &error))?;
    transaction.commit().map_err(|error| failed(path, &error))?;
    crate::diagnostic!(json!({"component":"delivery","kind":"ledger.imported",
        "from_file":imported.is_some(),"letters":ledger.letters.len(),
        "agents":ledger.agents.len(),"spawns":ledger.spawns.len(),
        "watches":ledger.watches.len()}));
    Ok(if imported.is_some() {
        Opened::Imported
    } else {
        Opened::Fresh
    })
}

/// Brings a file of version 1 to this build's version in one transaction:
/// its ledger read as that version wrote it, its letters addressed to the
/// registrations they are between (`upgraded`), and every row written again.
/// A start cut short before it commits finds version 1 again.
fn upgrade(connection: &mut Connection, path: &Path) -> Result<Opened, String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| failed(path, &error))?;
    let mut ledger = read_records(&transaction).map_err(|code| refused(path, &code))?;
    upgraded(&mut ledger, 1);
    // The rest row of version 1 also kept the machines' clocks, which
    // nothing reads now; every row is written again from the ledger.
    transaction
        .execute_batch(
            "DELETE FROM ledger; DELETE FROM letters; DELETE FROM agents; DELETE FROM spawns;",
        )
        .map_err(|error| failed(path, &error))?;
    Changes::all(&ledger)
        .write(&transaction)
        .map_err(|error| refused(path, error.code()))?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| failed(path, &error))?;
    transaction.commit().map_err(|error| failed(path, &error))?;
    Ok(Opened::Upgraded)
}

/// Addresses the letters of a ledger version `from` kept (`0` for the JSON
/// file) as this build does (`Ledger::address_letters`), and says so.
fn upgraded(ledger: &mut Ledger, from: i64) {
    let addressed = ledger.address_letters(crate::runtime::hide_notice, super::worker::now());
    crate::diagnostic!(json!({"component":"delivery","kind":"ledger.upgraded",
        "from_version":from,"to_version":SCHEMA_VERSION,"letters":ledger.letters.len(),
        "no_recipient":addressed.no_recipient,"expired":addressed.expired}));
}

/// `ledger`, an earlier build's, as an import or an upgrade keeps it: its
/// letters addressed (`Ledger::address_letters`), each it ended finished at
/// the time `kept`, the store's, gives it, which the start's clock set.
#[cfg(test)]
pub(crate) fn as_upgraded(mut ledger: Ledger, kept: &Ledger) -> Ledger {
    ledger.address_letters(crate::runtime::hide_notice, 0);
    for (letter, kept) in ledger.letters.iter_mut().zip(&kept.letters) {
        if letter.finished_at_unix_ms == Some(0) {
            letter.finished_at_unix_ms = kept.finished_at_unix_ms;
        }
    }
    ledger
}

/// `delivery-ledger.json` beside the store at `path`: where builds before it
/// kept the ledger.
pub fn legacy_path(path: &Path) -> PathBuf {
    hide_kit::layout::delivery_ledger(path.parent().unwrap_or(Path::new(".")))
}

/// The ledger `connection` holds, under the rules the core starts with: this
/// build's tables, records within the size the core keeps, and a valid
/// ledger. A core move asks it of the copy it is about to place
/// (`node_migration::copy::check_loadable`).
pub fn read_from(connection: &Connection) -> Result<Ledger, String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| "ledger_unavailable".to_owned())?;
    if version != SCHEMA_VERSION {
        return Err("ledger_unavailable".into());
    }
    read_records(connection)
}

/// The ledger `connection`'s tables hold, whichever version wrote them, under
/// the rules [`read_from`] reads them by.
fn read_records(connection: &Connection) -> Result<Ledger, String> {
    let unavailable = |_| "ledger_unavailable".to_owned();
    // A file this store wrote holds no more than the limit the core keeps
    // its ledger within; one that does was not, and is not read into memory.
    // Counted in bytes, as the limit is: `length` of text counts characters.
    let stored: i64 = connection
        .query_row(
            "SELECT (SELECT coalesce(sum(length(CAST(body AS BLOB))), 0) FROM ledger) \
             + (SELECT coalesce(sum(length(CAST(body AS BLOB))), 0) FROM letters) \
             + (SELECT coalesce(sum(length(CAST(body AS BLOB))), 0) FROM agents) \
             + (SELECT coalesce(sum(length(CAST(body AS BLOB))), 0) FROM spawns)",
            [],
            |row| row.get(0),
        )
        .map_err(unavailable)?;
    if stored > FILE_LIMIT as i64 {
        return Err("ledger_unavailable".into());
    }
    let rest: String = connection
        .query_row("SELECT body FROM ledger WHERE only = 1", [], |row| {
            row.get(0)
        })
        .map_err(unavailable)?;
    let rest: Rest = serde_json::from_str(&rest).map_err(|_| "ledger_unavailable")?;
    let ledger = rest.with(rows(connection)?, rows(connection)?, rows(connection)?);
    ledger.validate().map_err(|_| "ledger_unavailable")?;
    Ok(ledger)
}

/// Every row of `T`'s table, in the order its records were made.
fn rows<T: Row>(connection: &Connection) -> Result<Vec<T>, String> {
    let unavailable = |_| "ledger_unavailable".to_owned();
    let mut statement = connection
        .prepare(&format!(
            "SELECT seq, id, body FROM {} ORDER BY seq",
            T::TABLE
        ))
        .map_err(unavailable)?;
    let mut read = statement.query([]).map_err(unavailable)?;
    let mut records = Vec::new();
    while let Some(row) = read.next().map_err(unavailable)? {
        let seq: i64 = row.get(0).map_err(unavailable)?;
        let id: String = row.get(1).map_err(unavailable)?;
        let body: String = row.get(2).map_err(unavailable)?;
        let record: T = serde_json::from_str(&body).map_err(|_| "ledger_unavailable")?;
        // A row is the record its key names.
        if record.id() != id || sequence(&id) != Some(seq) {
            return Err("ledger_unavailable".into());
        }
        records.push(record);
    }
    Ok(records)
}

/// The ledger the store at `path` holds, in the shape its JSON file had,
/// which a core move and a device's renaming rewrite through the paths
/// `node_migration::KEYS` names; `None` when the folder holds no ledger at
/// all. A folder an older build wrote is imported first, as a start would.
/// Each record is read as its row holds it, not as the core would load it:
/// a rewrite changes keys and keeps everything else, and a core move checks
/// that the core loads its copy before placing it (`copy::check_loadable`).
pub fn read_value(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() && !legacy_path(path).exists() {
        return Ok(None);
    }
    let (store, _) = LedgerStore::open_file(path)?;
    let unreadable = |_| "ledger_unavailable".to_owned();
    let rest: String = store
        .connection
        .query_row("SELECT body FROM ledger WHERE only = 1", [], |row| {
            row.get(0)
        })
        .map_err(unreadable)?;
    let mut value: Value = serde_json::from_str(&rest).map_err(|_| "ledger_unavailable")?;
    let object = value.as_object_mut().ok_or("ledger_unavailable")?;
    for table in TABLES {
        let rows = raw_rows(&store.connection, table)?;
        object.insert(table.into(), Value::Array(rows.into_values().collect()));
    }
    Ok(Some(value))
}

/// Keeps `value`, a ledger in its JSON file's shape that [`read_value`]
/// gave and a rewrite changed, in the store at `path`: the rows that
/// differ, in one transaction. Each record is keyed by its `id`.
pub fn write_value(path: &Path, value: &Value) -> Result<(), String> {
    let unavailable = |_| "ledger_unavailable".to_owned();
    let mut rest = value.as_object().ok_or("ledger_unavailable")?.clone();
    let mut tables = Vec::new();
    for table in TABLES {
        let records = match rest.remove(table) {
            Some(Value::Array(records)) => records,
            None => Vec::new(),
            Some(_) => return Err("ledger_unavailable".into()),
        };
        let mut keyed = BTreeMap::new();
        for record in records {
            let seq = record
                .get("id")
                .and_then(Value::as_str)
                .and_then(sequence)
                .ok_or("ledger_unavailable")?;
            if keyed.insert(seq, record).is_some() {
                return Err("ledger_unavailable".into());
            }
        }
        tables.push((table, keyed));
    }
    let (mut store, _) = LedgerStore::open_file(path)?;
    let transaction = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(unavailable)?;
    for (table, keyed) in tables {
        let held = raw_rows(&transaction, table)?;
        for seq in held.keys().filter(|seq| !keyed.contains_key(seq)) {
            transaction
                .execute(&format!("DELETE FROM {table} WHERE seq = ?1"), [seq])
                .map_err(unavailable)?;
        }
        for (seq, record) in keyed
            .iter()
            .filter(|(seq, record)| held.get(seq) != Some(record))
        {
            let id = record["id"].as_str().ok_or("ledger_unavailable")?;
            transaction
                .execute(
                    &format!(
                        "INSERT INTO {table} (seq, id, body) VALUES (?1, ?2, ?3) \
                         ON CONFLICT(seq) DO UPDATE SET id = excluded.id, body = excluded.body"
                    ),
                    params![seq, id, record.to_string()],
                )
                .map_err(unavailable)?;
        }
    }
    transaction
        .execute(
            "INSERT INTO ledger (only, body) VALUES (1, ?1) \
             ON CONFLICT(only) DO UPDATE SET body = excluded.body",
            [Value::Object(rest).to_string()],
        )
        .map_err(unavailable)?;
    transaction.commit().map_err(unavailable)
}

/// Copies the store at `path` to `destination`, which must not exist yet,
/// with every committed row, whatever its records hold: a migration keeps
/// the original so before it rewrites it.
pub fn copy(path: &Path, destination: &Path) -> Result<(), String> {
    LedgerStore::open_file(path)?.0.copy_to(destination)
}

/// The tables of records, by the name of the ledger's list each holds.
const TABLES: [&str; 3] = ["letters", "agents", "spawns"];

/// Every row of `table` as its JSON value, by its sequence number.
fn raw_rows(connection: &Connection, table: &str) -> Result<BTreeMap<i64, Value>, String> {
    let unavailable = |_| "ledger_unavailable".to_owned();
    let mut statement = connection
        .prepare(&format!("SELECT seq, body FROM {table}"))
        .map_err(unavailable)?;
    let mut read = statement.query([]).map_err(unavailable)?;
    let mut rows = BTreeMap::new();
    while let Some(row) = read.next().map_err(unavailable)? {
        let seq: i64 = row.get(0).map_err(unavailable)?;
        let body: String = row.get(1).map_err(unavailable)?;
        rows.insert(
            seq,
            serde_json::from_str(&body).map_err(|_| "ledger_unavailable")?,
        );
    }
    Ok(rows)
}

/// The ledger the store at `path` holds; a folder an older build wrote is
/// imported first, as a start would.
pub fn load(path: &Path) -> Result<Ledger, String> {
    LedgerStore::open(path).map(|(_, ledger, _)| ledger)
}

/// Keeps `ledger` in the store at `path` in place of what it holds.
pub fn save(path: &Path, ledger: &Ledger) -> Result<(), SaveError> {
    ledger.check().map_err(SaveError::Validation)?;
    let (mut store, before, _) = LedgerStore::open(path).map_err(SaveError::Validation)?;
    store.write(&before, ledger)
}

/// A record kept as a row of its own table.
trait Row: Serialize + DeserializeOwned + PartialEq {
    const TABLE: &'static str;
    fn id(&self) -> &str;
}

impl Row for Letter {
    const TABLE: &'static str = "letters";
    fn id(&self) -> &str {
        &self.id
    }
}

impl Row for AgentRecord {
    const TABLE: &'static str = "agents";
    fn id(&self) -> &str {
        &self.id
    }
}

impl Row for SpawnRecord {
    const TABLE: &'static str = "spawns";
    fn id(&self) -> &str {
        &self.id
    }
}

/// A record's number in the ledger's one id sequence, which `Ledger::validate`
/// holds unique across letters, watches, registrations and spawns.
fn sequence(id: &str) -> Option<i64> {
    id.split_once('-')?.1.parse().ok()
}

/// The ledger but its records: its one row.
#[derive(Deserialize)]
struct Rest {
    version: u32,
    next_id: u64,
    watches: Vec<Watch>,
    #[serde(default)]
    identity_waits: Vec<IdentityWait>,
    #[serde(default)]
    spawn_placement_revision: u64,
    #[serde(default)]
    human_turns: BTreeMap<String, HumanTurn>,
}

impl Rest {
    /// The ledger of this row and these records. Every field is named, so a
    /// field added to the ledger is not left out of the file unnoticed.
    fn with(
        self,
        letters: Vec<Letter>,
        agents: Vec<AgentRecord>,
        spawns: Vec<SpawnRecord>,
    ) -> Ledger {
        let Self {
            version,
            next_id,
            watches,
            identity_waits,
            spawn_placement_revision,
            human_turns,
        } = self;
        Ledger {
            version,
            next_id,
            letters,
            watches,
            agents,
            spawns,
            identity_waits,
            spawn_placement_revision,
            human_turns,
        }
    }
}

/// The ledger's one row as it is written: [`Rest`], borrowed.
#[derive(Serialize, PartialEq)]
struct RestRef<'a> {
    version: u32,
    next_id: u64,
    watches: &'a [Watch],
    identity_waits: &'a [IdentityWait],
    spawn_placement_revision: u64,
    human_turns: &'a BTreeMap<String, HumanTurn>,
}

/// `ledger` as its one row and its three tables' records. Every field is
/// named, so a field added to the ledger fails to build until it is kept.
fn split(ledger: &Ledger) -> (RestRef<'_>, &[Letter], &[AgentRecord], &[SpawnRecord]) {
    let Ledger {
        version,
        next_id,
        letters,
        watches,
        agents,
        spawns,
        identity_waits,
        spawn_placement_revision,
        human_turns,
    } = ledger;
    (
        RestRef {
            version: *version,
            next_id: *next_id,
            watches,
            identity_waits,
            spawn_placement_revision: *spawn_placement_revision,
            human_turns,
        },
        letters,
        agents,
        spawns,
    )
}

/// The rows of one table a write puts or takes away.
struct Rows<'a, T> {
    put: Vec<&'a T>,
    gone: Vec<&'a str>,
}

impl<'a, T: Row> Rows<'a, T> {
    /// Each record of `after` that `before` does not hold as it is, and each
    /// of `before` that `after` no longer holds, by id.
    fn between(before: &'a [T], after: &'a [T]) -> Self {
        let held: HashMap<&str, &T> = before.iter().map(|record| (record.id(), record)).collect();
        let kept: HashSet<&str> = after.iter().map(Row::id).collect();
        Self {
            put: after
                .iter()
                .filter(|record| held.get(record.id()) != Some(record))
                .collect(),
            gone: before
                .iter()
                .map(Row::id)
                .filter(|id| !kept.contains(id))
                .collect(),
        }
    }

    fn all(records: &'a [T]) -> Self {
        Self {
            put: records.iter().collect(),
            gone: Vec::new(),
        }
    }

    fn write(&self, transaction: &rusqlite::Transaction<'_>) -> Result<(), SaveError> {
        let refused = |error: rusqlite::Error| SaveError::Refused(StoreFailure::of(&error));
        let unencodable = || SaveError::Validation("ledger_unavailable".into());
        if !self.gone.is_empty() {
            let mut delete = transaction
                .prepare_cached(&format!("DELETE FROM {} WHERE seq = ?1", T::TABLE))
                .map_err(refused)?;
            for id in &self.gone {
                delete
                    .execute([sequence(id).ok_or_else(unencodable)?])
                    .map_err(refused)?;
            }
        }
        if !self.put.is_empty() {
            let mut put = transaction
                .prepare_cached(&format!(
                    "INSERT INTO {} (seq, id, body) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(seq) DO UPDATE SET id = excluded.id, body = excluded.body",
                    T::TABLE
                ))
                .map_err(refused)?;
            for record in &self.put {
                let body = serde_json::to_string(record).map_err(|_| unencodable())?;
                put.execute(params![
                    sequence(record.id()).ok_or_else(unencodable)?,
                    record.id(),
                    body
                ])
                .map_err(refused)?;
            }
        }
        Ok(())
    }

    fn ids(&self) -> impl Iterator<Item = &str> {
        self.put
            .iter()
            .map(|record| record.id())
            .chain(self.gone.iter().copied())
    }
}

/// What one write changes: the rows of each table, and the ledger's one
/// row when it moved.
pub struct Changes<'a> {
    letters: Rows<'a, Letter>,
    agents: Rows<'a, AgentRecord>,
    spawns: Rows<'a, SpawnRecord>,
    rest: Option<RestRef<'a>>,
}

impl<'a> Changes<'a> {
    /// The rows that keep `after` in a file holding `before`.
    pub fn between(before: &'a Ledger, after: &'a Ledger) -> Self {
        let (rest_before, letters_before, agents_before, spawns_before) = split(before);
        let (rest, letters, agents, spawns) = split(after);
        Self {
            letters: Rows::between(letters_before, letters),
            agents: Rows::between(agents_before, agents),
            spawns: Rows::between(spawns_before, spawns),
            rest: (rest != rest_before).then_some(rest),
        }
    }

    /// Every row of `ledger`, for a file that holds none.
    fn all(ledger: &'a Ledger) -> Self {
        let (rest, letters, agents, spawns) = split(ledger);
        Self {
            letters: Rows::all(letters),
            agents: Rows::all(agents),
            spawns: Rows::all(spawns),
            rest: Some(rest),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rest.is_none()
            && [
                self.letters.put.len() + self.letters.gone.len(),
                self.agents.put.len() + self.agents.gone.len(),
                self.spawns.put.len() + self.spawns.gone.len(),
            ]
            .iter()
            .all(|count| *count == 0)
    }

    fn write(&self, transaction: &rusqlite::Transaction<'_>) -> Result<(), SaveError> {
        self.letters.write(transaction)?;
        self.agents.write(transaction)?;
        self.spawns.write(transaction)?;
        if let Some(rest) = &self.rest {
            let body = serde_json::to_string(rest)
                .map_err(|_| SaveError::Validation("ledger_unavailable".into()))?;
            transaction
                .execute(
                    "INSERT INTO ledger (only, body) VALUES (1, ?1) \
                     ON CONFLICT(only) DO UPDATE SET body = excluded.body",
                    [body],
                )
                .map_err(|error| SaveError::Refused(StoreFailure::of(&error)))?;
        }
        Ok(())
    }

    /// What a failed write would have changed, for its diagnostic (B29):
    /// the ids of the records it would have put or taken away, the first
    /// [`NAMED`] of each table, and how many of each there were.
    pub fn diagnostic(&self) -> Value {
        const NAMED: usize = 16;
        let table = |ids: Vec<&str>| json!({"count": ids.len(), "ids": ids.iter().take(NAMED).collect::<Vec<_>>()});
        json!({
            "letters": table(self.letters.ids().collect()),
            "agents": table(self.agents.ids().collect()),
            "spawns": table(self.spawns.ids().collect()),
            "ledger_row": self.rest.is_some(),
        })
    }
}

/// A failure's code for a log: SQLite's own kind, never a path or content.
pub(crate) fn code(error: &rusqlite::Error) -> &'static str {
    match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => "full",
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "busy",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "corrupt",
        Some(ErrorCode::ReadOnly | ErrorCode::PermissionDenied | ErrorCode::CannotOpen) => {
            "cannot_write"
        }
        Some(ErrorCode::SystemIoFailure) => "io",
        _ => "failed",
    }
}

/// Logs why the store at `path` could not be opened and answers the code
/// the core refuses delivery with.
fn refused(path: &Path, why: &str) -> String {
    crate::diagnostic!(json!({"component":"delivery","kind":"ledger.store_refused",
        "file":path.file_name().map(|name| name.to_string_lossy().into_owned()),"why":why}));
    "ledger_unavailable".into()
}

fn failed(path: &Path, error: &rusqlite::Error) -> String {
    refused(path, code(error))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::delivery::{Actor, Command, ledger, mailbox};

    /// A private state folder, as the core keeps one.
    fn state() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        (root, state)
    }

    /// `delivery-ledger.json` in `state` holding `bytes`, private as the
    /// builds before the store wrote it.
    fn old_file(state: &Path, bytes: &[u8]) -> PathBuf {
        let path = hide_kit::layout::delivery_ledger(state);
        hide_platform::fs::private::create_new_file(&path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
        path
    }

    fn actor(pane: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "claude".into(),
            device_id: "node".into(),
            session: Some(format!("{pane}-session")),
        }
    }

    fn send(ledger: &mut Ledger, intent: &str, body: &str, now: u64) -> String {
        mailbox::send(
            ledger,
            &actor("w1:p1"),
            &actor("w1:p2"),
            intent,
            body,
            "report",
            None,
            now,
        )
        .unwrap()
        .id
    }

    /// B24: the ledger each released build left is what the first start of
    /// this one reads, imported whole, and the file it was in stays as it
    /// was; a later start reads the store.
    #[test]
    fn every_released_ledger_is_imported_whole_and_its_old_file_stays_as_it_was() {
        let mut imported = 0;
        for fixture in crate::node_migration::upgrade_tests::fixtures() {
            let released = fixture.join("delivery-ledger.json");
            if !released.is_file() {
                continue;
            }
            let bytes = std::fs::read(&released).unwrap();
            let (_root, state) = state();
            let old = old_file(&state, &bytes);
            // What the released build itself started with.
            let expected = ledger::load(&old).unwrap();
            assert!(!expected.letters.is_empty() && !expected.agents.is_empty());
            let path = hide_kit::layout::delivery_store(&state);
            let (kept, records) = crate::diagnostics::capture(|| ledger::recover(&path).unwrap());
            let expected = as_upgraded(expected, &kept);
            assert_eq!(kept, expected, "{fixture:?}");
            assert!(
                records
                    .iter()
                    .any(|record| record["kind"] == "ledger.imported"
                        && record["letters"] == expected.letters.len()),
                "{records:?}"
            );
            assert_eq!(std::fs::read(&old).unwrap(), bytes, "{fixture:?}");
            assert!(hide_platform::fs::private::is_private(&path).unwrap());
            // The second start reads what the first kept and imports nothing.
            let (_, again, opened) = LedgerStore::open(&path).unwrap();
            assert_eq!(opened, Opened::Existing);
            assert_eq!(again, expected);
            assert_eq!(std::fs::read(&old).unwrap(), bytes);
            imported += 1;
        }
        assert!(imported >= 5, "only {imported} released ledgers were found");
    }

    /// B24, D-20: an import is one transaction with the tables it makes. A
    /// start cut short before it committed, or one that could not read the
    /// old file, leaves no ledger behind, and the next start imports again;
    /// the old file is never changed.
    #[test]
    fn a_start_that_did_not_finish_its_import_imports_again_and_never_touches_the_old_file() {
        let (_root, state) = state();
        let path = hide_kit::layout::delivery_store(&state);
        let mut ledger = Ledger::default();
        send(&mut ledger, "kept", "a letter from before the update", 1);
        let bytes = ledger.bytes().unwrap();

        // Cut short: the file was made, nothing committed.
        let old = old_file(&state, b"not a ledger");
        hide_platform::fs::private::create_new_file(&path).unwrap();
        assert_eq!(
            ledger::recover(&path).unwrap_err().code(),
            "ledger_unavailable"
        );
        assert_eq!(std::fs::read(&old).unwrap(), b"not a ledger");
        let version: i64 = Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0, "a refused import commits nothing");

        std::fs::remove_file(&old).unwrap();
        let old = old_file(&state, &bytes);
        let (_, read, opened) = LedgerStore::open(&path).unwrap();
        assert_eq!(opened, Opened::Imported);
        assert_eq!(read, as_upgraded(ledger, &read));
        assert_eq!(std::fs::read(&old).unwrap(), bytes);
    }

    /// B27, D-20: an older build started on the folder after this one reads
    /// `delivery-ledger.json` as the update found it, with its own loader,
    /// whatever this build sent, ended or dropped since; this build goes on
    /// from its store.
    #[test]
    fn an_older_build_started_after_this_one_reads_the_old_file_as_the_update_left_it() {
        let fixture = crate::node_migration::upgrade_tests::fixtures()
            .into_iter()
            .rev()
            .map(|folder| folder.join("delivery-ledger.json"))
            .find(|file| file.is_file())
            .unwrap();
        let bytes = std::fs::read(&fixture).unwrap();
        let (_root, state) = state();
        let old = old_file(&state, &bytes);
        let before = ledger::load(&old).unwrap();
        let path = hide_kit::layout::delivery_store(&state);
        let started = ledger::recover(&path).unwrap();

        // This build works on: a new letter, every registration ended, the
        // first letter gone.
        let mut after = started.clone();
        let sent = send(&mut after, "after-update", "sent after the update", 10);
        for agent in &mut after.agents {
            agent.ended = true;
        }
        after.letters.remove(0);
        let (mut store, held, _) = LedgerStore::open(&path).unwrap();
        store.write(&held, &after).unwrap();
        drop(store);

        // The older build's start.
        assert_eq!(std::fs::read(&old).unwrap(), bytes);
        let older = ledger::load(&old).unwrap();
        assert_eq!(older, before);
        assert!(!older.letters.iter().any(|letter| letter.id == sent));
        assert!(older.agents.iter().any(|agent| !agent.ended));
        // This build's next start reads its store, not the old file.
        let again = ledger::recover(&path).unwrap();
        assert_eq!(again, after);
        assert!(again.letters.iter().any(|letter| letter.id == sent));
    }

    /// D-31: a write is the rows that changed. A ledger kept as it is writes
    /// nothing; one letter confirmed and another dropped are two rows.
    #[test]
    fn a_write_puts_only_the_rows_that_changed() {
        let (_root, state) = state();
        let path = hide_kit::layout::delivery_store(&state);
        let (mut store, empty, opened) = LedgerStore::open(&path).unwrap();
        assert_eq!(opened, Opened::Fresh);
        let mut first = Ledger::default();
        for index in 0..3 {
            send(&mut first, &format!("letter-{index}"), "body", 1);
        }
        store.write(&empty, &first).unwrap();
        let written = |store: &LedgerStore| store.connection.total_changes();

        let before = written(&store);
        store.write(&first, &first.clone()).unwrap();
        assert_eq!(written(&store), before, "an unchanged ledger writes no row");

        let mut second = first.clone();
        let confirmed = second.letters[1].id.clone();
        mailbox::apply(
            &mut second,
            &actor("w1:p2"),
            None,
            &Command::Confirm {
                ids: vec![confirmed],
            },
            2,
        )
        .unwrap();
        second.letters.remove(0);
        assert_eq!(second.next_id, first.next_id);
        store.write(&first, &second).unwrap();
        assert_eq!(written(&store) - before, 2);
        drop(store);
        assert_eq!(load(&path).unwrap(), second);
    }

    /// B29: a write the file cannot take is refused before its commit, so
    /// the file keeps what it held, and the failure says what SQLite
    /// answered without a path or a letter's text.
    #[test]
    fn a_write_the_file_cannot_take_is_refused_and_the_file_keeps_what_it_held() {
        let (_root, state) = state();
        let path = hide_kit::layout::delivery_store(&state);
        let (mut store, empty, _) = LedgerStore::open(&path).unwrap();
        let mut held = Ledger::default();
        send(&mut held, "small", "fits", 1);
        store.write(&empty, &held).unwrap();
        // The file may not grow past the pages it has.
        let pages: i64 = store
            .connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        store
            .connection
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let mut bigger = held.clone();
        for index in 0..8 {
            send(&mut bigger, &format!("big-{index}"), &"y".repeat(16_000), 2);
        }
        let error = store.write(&held, &bigger).unwrap_err();
        assert!(!error.uncertain());
        assert_eq!(error.code(), "ledger_unavailable");
        assert_eq!(error.diagnostic()["phase"], "store_refused");
        assert_eq!(error.diagnostic()["sqlite"], "full");
        assert!(!error.diagnostic().to_string().contains("yyyy"));
        drop(store);
        assert_eq!(load(&path).unwrap(), held);
    }

    /// The ids a failed write names are the records it was to put or take
    /// away, and no more than the first sixteen of each.
    #[test]
    fn a_failed_writes_diagnostic_names_the_records_it_was_to_keep() {
        let mut before = Ledger::default();
        let gone = send(&mut before, "gone", "body", 1);
        let mut after = before.clone();
        after.letters.clear();
        let sent: Vec<String> = (0..20)
            .map(|index| send(&mut after, &format!("new-{index}"), "secret body", 2))
            .collect();
        let diagnostic = Changes::between(&before, &after).diagnostic();
        assert_eq!(diagnostic["letters"]["count"], 21);
        let named = diagnostic["letters"]["ids"].as_array().unwrap();
        assert_eq!(named.len(), 16);
        assert_eq!(named[0], json!(sent[0]));
        assert_eq!(diagnostic["agents"]["count"], 0);
        assert_eq!(diagnostic["ledger_row"], true);
        assert!(!diagnostic.to_string().contains("secret body"));
        let only_gone = Changes::between(
            &before,
            &Ledger {
                letters: Vec::new(),
                ..before.clone()
            },
        )
        .diagnostic();
        assert_eq!(only_gone["letters"]["ids"], json!([gone]));
    }

    /// A file a newer build made, or one that is not a ledger, is refused
    /// and kept as it is; nothing is imported over it.
    #[test]
    fn a_newer_or_damaged_file_is_refused_and_kept() {
        let (_root, state) = state();
        let path = hide_kit::layout::delivery_store(&state);
        drop(LedgerStore::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let mut old = Ledger::default();
        send(&mut old, "old", "body", 1);
        old_file(&state, &old.bytes().unwrap());
        assert_eq!(
            LedgerStore::open(&path).err().unwrap(),
            "ledger_unavailable"
        );
        let version: i64 = Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION + 1);
    }
}
