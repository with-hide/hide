//! `suggestions.sqlite3`: the record ledger, the notebook and each line's
//! story, per Project identity (PRD hide-ai-suggestions B20, D-46). Only
//! the suggestion worker opens it, so it has one writer; every write is one
//! transaction, so an edit an investigation made is on disk before it is
//! answered (D-35). Records older than [`policy::RECORD_RETENTION_MS`] and
//! past [`policy::RECORD_CAP`] are pruned as new ones arrive.

use super::ledger::{Observation, Record, RecordEntry, RecordId, RecordIndex, Signal};
use super::lifecycle::LineLife;
use super::notebook::{Applied, Line, LineId, Notebook};
use super::policy;
use super::settings::Settings;
use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE projects(id TEXT PRIMARY KEY, state TEXT NOT NULL, next_record INTEGER NOT NULL,
    next_line INTEGER NOT NULL);
CREATE TABLE records(project TEXT NOT NULL, id INTEGER NOT NULL, key TEXT NOT NULL,
    at INTEGER NOT NULL, fresh INTEGER NOT NULL, fact TEXT NOT NULL,
    PRIMARY KEY(project, id), UNIQUE(project, key));
CREATE INDEX records_at ON records(project, at);
CREATE TABLE lines(project TEXT NOT NULL, id INTEGER NOT NULL, line TEXT NOT NULL,
    life TEXT NOT NULL, PRIMARY KEY(project, id));
";

/// The store's file in the core's state folder.
pub fn path(state_dir: &Path) -> PathBuf {
    hide_kit::layout::suggestions_store(state_dir)
}

/// A failure's code: what a caller words or logs, never a path or content.
pub fn code(error: &rusqlite::Error) -> &'static str {
    match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => "suggestions_store_full",
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "suggestions_store_busy",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "suggestions_store_corrupt",
        _ => "suggestions_store_failed",
    }
}

fn failed(error: rusqlite::Error) -> String {
    code(&error).to_owned()
}

fn decode<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|_| "suggestions_store_value_invalid".to_owned())
}

fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "suggestions_store_value_invalid".to_owned())
}

/// A Project's settings and schedule, kept as one value so a field added
/// later reads as its default from an older file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectState {
    #[serde(default)]
    pub settings: Settings,
    /// The local day of the last completed run, `YYYY-MM-DD` (D-34).
    #[serde(default)]
    pub last_completed_day: Option<String>,
    /// When the last run started and how it ended.
    #[serde(default)]
    pub last_run_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub last_run_end: Option<String>,
    /// When lines were raised, for the week's cap; only the last
    /// [`policy::WEEK_MS`] are kept.
    #[serde(default)]
    pub raised_at: Vec<u64>,
    /// The machine and folder the Project was last named from, where an
    /// automatic attempt starts and an attempt is followed (B13, B21).
    #[serde(default)]
    pub location: Option<ProjectLocation>,
    /// Since when the Project has raised nothing: its first look at what
    /// rose, then its last raise. A Normal Project quiet for
    /// [`policy::QUIET_SPAN_MS`] from it raises its strongest line under the
    /// threshold (D-10).
    #[serde(default)]
    pub quiet_since: Option<u64>,
    /// The last run failed, so nothing rose from it: the Project shows
    /// `조사 못 함` and the next run investigates even with no new records.
    #[serde(default)]
    pub uninvestigated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectLocation {
    pub node: String,
    pub path: String,
}

/// How the store came to be open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    Fresh,
    Existing,
    /// The file failed its integrity check and was set aside as
    /// `suggestions.sqlite3.corrupt`; this one is new.
    Rebuilt,
}

pub struct SuggestionStore {
    connection: Connection,
}

enum OpenFailure {
    Corrupt,
    Newer,
    Failed(String),
}

fn classify(error: rusqlite::Error) -> OpenFailure {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => OpenFailure::Corrupt,
        _ => OpenFailure::Failed(code(&error).to_owned()),
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// What one `record` call kept: the new records, the records a pull
/// request's failure on a newer head moved, and what the prune took away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub written: Vec<RecordId>,
    pub moved: Vec<RecordId>,
    pub pruned: Pruned,
}

/// What a prune took away: the records, and the lines' references to them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub records: usize,
    pub references: usize,
}

impl SuggestionStore {
    /// Opens the writer's connection, making the file private when it is
    /// new. A file that fails its integrity check is set aside once and made
    /// again; a file of a newer schema is refused and left as it is.
    pub fn open(path: &Path) -> Result<(Self, Opened), String> {
        let existed = path.exists();
        match Self::open_writer(path) {
            Ok(store) => Ok((
                store,
                if existed {
                    Opened::Existing
                } else {
                    Opened::Fresh
                },
            )),
            Err(OpenFailure::Newer) => Err("suggestions_store_newer".to_owned()),
            Err(OpenFailure::Failed(code)) => Err(code),
            Err(OpenFailure::Corrupt) => {
                std::fs::rename(path, sibling(path, ".corrupt"))
                    .map_err(|_| "suggestions_store_set_aside_failed".to_owned())?;
                for suffix in ["-wal", "-shm"] {
                    let _ = std::fs::remove_file(sibling(path, suffix));
                }
                match Self::open_writer(path) {
                    Ok(store) => Ok((store, Opened::Rebuilt)),
                    Err(OpenFailure::Failed(code)) => Err(code),
                    Err(_) => Err("suggestions_store_corrupt".to_owned()),
                }
            }
        }
    }

    fn open_writer(path: &Path) -> Result<Self, OpenFailure> {
        hide_platform::fs::private::open_or_create_file(path)
            .map_err(|_| OpenFailure::Failed("suggestions_store_unavailable".to_owned()))?;
        let connection = Connection::open(path).map_err(classify)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(classify)?;
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(classify)?;
        let check: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(classify)?;
        if check != "ok" {
            return Err(OpenFailure::Corrupt);
        }
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(classify)?;
        match version {
            0 => connection
                .execute_batch(&format!(
                    "BEGIN; {SCHEMA} PRAGMA user_version={SCHEMA_VERSION}; COMMIT;"
                ))
                .map_err(classify)?,
            SCHEMA_VERSION => {}
            newer if newer > SCHEMA_VERSION => return Err(OpenFailure::Newer),
            _ => return Err(OpenFailure::Corrupt),
        }
        Ok(Self { connection })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(failed)
    }

    pub fn set_meta(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(failed)?;
        Ok(())
    }

    /// Every Project the store has heard of.
    pub fn projects(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT id FROM projects ORDER BY id")
            .map_err(failed)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)
    }

    /// The Project's settings and schedule; the defaults for one never seen.
    pub fn state(&self, project: &str) -> Result<ProjectState, String> {
        let state: Option<String> = self
            .connection
            .query_row("SELECT state FROM projects WHERE id=?1", [project], |row| {
                row.get(0)
            })
            .optional()
            .map_err(failed)?;
        state.map_or_else(|| Ok(ProjectState::default()), |state| decode(&state))
    }

    pub fn known(&self, project: &str) -> Result<bool, String> {
        self.connection
            .query_row("SELECT 1 FROM projects WHERE id=?1", [project], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
            .map_err(failed)
    }

    /// How many improvement records the session `fact` names already keeps.
    fn session_improvements(
        tx: &Transaction<'_>,
        project: &str,
        fact: &Observation,
    ) -> Result<usize, String> {
        let Some(unit) = fact.unit() else {
            return Ok(0);
        };
        let escaped = unit
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{}\u{1f}%\u{1f}{escaped}", Signal::Improvement.as_str());
        tx.query_row(
            "SELECT COUNT(*) FROM records WHERE project=?1 AND key LIKE ?2 ESCAPE '\\'",
            params![project, pattern],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count as usize)
        .map_err(failed)
    }

    fn ensure(tx: &Transaction<'_>, project: &str) -> Result<(), String> {
        tx.execute(
            "INSERT INTO projects VALUES(?1,?2,1,1) ON CONFLICT(id) DO NOTHING",
            params![project, encode(&ProjectState::default())?],
        )
        .map_err(failed)?;
        Ok(())
    }

    pub fn save_state(&mut self, project: &str, state: &ProjectState) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        Self::ensure(&tx, project)?;
        tx.execute(
            "UPDATE projects SET state=?2 WHERE id=?1",
            params![project, encode(state)?],
        )
        .map_err(failed)?;
        tx.commit().map_err(failed)
    }

    /// Keeps `facts` as records of `project`, each fact folding into the
    /// record already kept under its key (B4: once per session or pull
    /// request), and prunes past the age and count bounds. A pull request's
    /// failure on a head other than the one kept moves its one record to
    /// that head and time, so a watch asks about the head it failed on last.
    pub fn record(
        &mut self,
        project: &str,
        facts: Vec<Observation>,
        now: u64,
    ) -> Result<Recorded, String> {
        let tx = self.connection.transaction().map_err(failed)?;
        Self::ensure(&tx, project)?;
        let mut next: u64 = tx
            .query_row(
                "SELECT next_record FROM projects WHERE id=?1",
                [project],
                |row| row.get::<_, i64>(0),
            )
            .map_err(failed)? as u64;
        let mut written = Vec::new();
        let mut moved = Vec::new();
        let mut over_cap = 0;
        for fact in facts {
            let Some(fact) = fact.normalized() else {
                continue;
            };
            let Some(key) = fact.key() else {
                continue;
            };
            if fact.signal == Signal::Improvement
                && Self::session_improvements(&tx, project, &fact)?
                    >= policy::IMPROVEMENTS_PER_SESSION
            {
                over_cap += 1;
                continue;
            }
            let inserted = tx
                .execute(
                    "INSERT INTO records VALUES(?1,?2,?3,?4,1,?5) ON CONFLICT(project,key) DO NOTHING",
                    params![project, next as i64, key, fact.at_unix_ms as i64, encode(&fact)?],
                )
                .map_err(failed)?;
            if inserted == 1 {
                written.push(RecordId(next));
                next += 1;
            } else if fact.head.is_some() {
                let (id, kept): (i64, String) = tx
                    .query_row(
                        "SELECT id, fact FROM records WHERE project=?1 AND key=?2",
                        params![project, key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(failed)?;
                if decode::<Observation>(&kept)?.head != fact.head {
                    tx.execute(
                        "UPDATE records SET at=?3, fact=?4 WHERE project=?1 AND id=?2",
                        params![project, id, fact.at_unix_ms as i64, encode(&fact)?],
                    )
                    .map_err(failed)?;
                    moved.push(RecordId(id as u64));
                }
            }
        }
        if over_cap > 0 {
            crate::diagnostic!(serde_json::json!({
                "component": "suggestions", "kind": "record.session_cap",
                "signal": Signal::Improvement.as_str(), "dropped": over_cap,
                "limit": policy::IMPROVEMENTS_PER_SESSION,
            }));
        }
        tx.execute(
            "UPDATE projects SET next_record=?2 WHERE id=?1",
            params![project, next as i64],
        )
        .map_err(failed)?;
        let pruned = Self::prune_in(&tx, project, now)?;
        tx.commit().map_err(failed)?;
        Ok(Recorded {
            written,
            moved,
            pruned,
        })
    }

    fn prune_in(tx: &Transaction<'_>, project: &str, now: u64) -> Result<Pruned, String> {
        let old = tx
            .execute(
                "DELETE FROM records WHERE project=?1 AND at<?2",
                params![
                    project,
                    now.saturating_sub(policy::RECORD_RETENTION_MS) as i64
                ],
            )
            .map_err(failed)?;
        let over = tx
            .execute(
                "DELETE FROM records WHERE project=?1 AND id IN (SELECT id FROM records \
                 WHERE project=?1 ORDER BY at DESC, id DESC LIMIT -1 OFFSET ?2)",
                params![project, policy::RECORD_CAP as i64],
            )
            .map_err(failed)?;
        let records = old + over;
        let references = if records == 0 {
            0
        } else {
            Self::drop_references(tx, project)?
        };
        Ok(Pruned {
            records,
            references,
        })
    }

    /// Takes the references to records no longer kept off every line, in
    /// the prune's transaction, so no line holds more than the ledger's own
    /// bounds; answers how many went.
    fn drop_references(tx: &Transaction<'_>, project: &str) -> Result<usize, String> {
        let kept = tx
            .prepare_cached("SELECT id FROM records WHERE project=?1")
            .map_err(failed)?
            .query_map([project], |row| row.get::<_, i64>(0))
            .map_err(failed)?
            .map(|id| id.map(|id| RecordId(id as u64)))
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(failed)?;
        let lines = tx
            .prepare_cached("SELECT id, line FROM lines WHERE project=?1")
            .map_err(failed)?
            .query_map([project], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut dropped = 0;
        for (id, text) in lines {
            let mut line: Line = decode(&text)?;
            let before = line.records.len();
            line.records.retain(|record| kept.contains(record));
            if line.records.len() == before {
                continue;
            }
            dropped += before - line.records.len();
            tx.execute(
                "UPDATE lines SET line=?3 WHERE project=?1 AND id=?2",
                params![project, id, encode(&line)?],
            )
            .map_err(failed)?;
        }
        Ok(dropped)
    }

    /// The records of `ids` the Project still keeps, in the order asked.
    pub fn records_of(&self, project: &str, ids: &[RecordId]) -> Result<Vec<Record>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT fact FROM records WHERE project=?1 AND id=?2")
            .map_err(failed)?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let fact = statement
                .query_row(params![project, id.0 as i64], |row| row.get::<_, String>(0))
                .optional()
                .map_err(failed)?;
            if let Some(fact) = fact {
                records.push(Record {
                    id: *id,
                    fact: decode(&fact)?,
                });
            }
        }
        Ok(records)
    }

    /// Every record the Project keeps, oldest number first.
    pub fn records(&self, project: &str) -> Result<Vec<(Record, bool)>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT id, fresh, fact FROM records WHERE project=?1 ORDER BY id")
            .map_err(failed)?;
        let rows = statement
            .query_map([project], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        rows.into_iter()
            .map(|(id, fresh, fact)| {
                Ok((
                    Record {
                        id: RecordId(id as u64),
                        fact: decode(&fact)?,
                    },
                    fresh,
                ))
            })
            .collect()
    }

    /// The Project's notebook and each line's story.
    pub fn notebook(
        &self,
        project: &str,
    ) -> Result<(Notebook, BTreeMap<LineId, LineLife>), String> {
        let next_line: Option<i64> = self
            .connection
            .query_row(
                "SELECT next_line FROM projects WHERE id=?1",
                [project],
                |row| row.get(0),
            )
            .optional()
            .map_err(failed)?;
        let mut statement = self
            .connection
            .prepare_cached("SELECT line, life FROM lines WHERE project=?1 ORDER BY id")
            .map_err(failed)?;
        let rows = statement
            .query_map([project], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut notebook = Notebook {
            lines: Vec::with_capacity(rows.len()),
            next_line: next_line.unwrap_or(1) as u64,
        };
        let mut lives = BTreeMap::new();
        for (line, life) in rows {
            let line: Line = decode(&line)?;
            lives.insert(line.id, decode(&life)?);
            notebook.lines.push(line);
        }
        Ok((notebook, lives))
    }

    /// The index a run is handed: every kept record with the line it is on.
    pub fn index(&self, project: &str, notebook: &Notebook) -> Result<RecordIndex, String> {
        let on_line = notebook
            .lines
            .iter()
            .flat_map(|line| line.records.iter().map(move |record| (*record, line.id)))
            .collect::<BTreeMap<_, _>>();
        Ok(RecordIndex {
            entries: self
                .records(project)?
                .into_iter()
                .map(|(record, fresh)| RecordEntry {
                    id: record.id,
                    signal: record.fact.signal,
                    tag: record.fact.tag,
                    session: record.fact.session,
                    pr: record.fact.pr.map(|pr| pr.number),
                    at_unix_ms: record.fact.at_unix_ms,
                    always: record.fact.always,
                    improvement: record.fact.improvement,
                    line: on_line.get(&record.id).copied(),
                    fresh,
                })
                .collect(),
        })
    }

    /// Saves exactly what one accepted edit changed, in one transaction.
    pub fn save_edit(
        &mut self,
        project: &str,
        notebook: &Notebook,
        applied: &Applied,
    ) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        Self::ensure(&tx, project)?;
        for line in &applied.removed {
            tx.execute(
                "DELETE FROM lines WHERE project=?1 AND id=?2",
                params![project, line.0 as i64],
            )
            .map_err(failed)?;
        }
        for id in &applied.touched {
            let Some(line) = notebook.line(*id) else {
                continue;
            };
            tx.execute(
                "INSERT INTO lines VALUES(?1,?2,?3,?4) \
                 ON CONFLICT(project,id) DO UPDATE SET line=excluded.line",
                params![
                    project,
                    id.0 as i64,
                    encode(line)?,
                    encode(&LineLife::default())?
                ],
            )
            .map_err(failed)?;
        }
        for record in &applied.seen {
            tx.execute(
                "UPDATE records SET fresh=0 WHERE project=?1 AND id=?2",
                params![project, record.0 as i64],
            )
            .map_err(failed)?;
        }
        tx.execute(
            "UPDATE projects SET next_line=?2 WHERE id=?1",
            params![project, notebook.next_line.max(1) as i64],
        )
        .map_err(failed)?;
        tx.commit().map_err(failed)
    }

    /// Saves one line as the lifecycle moved it, with its story.
    pub fn save_line(&mut self, project: &str, line: &Line, life: &LineLife) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        Self::ensure(&tx, project)?;
        tx.execute(
            "INSERT INTO lines VALUES(?1,?2,?3,?4) \
             ON CONFLICT(project,id) DO UPDATE SET line=excluded.line, life=excluded.life",
            params![project, line.id.0 as i64, encode(line)?, encode(life)?],
        )
        .map_err(failed)?;
        tx.commit().map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::{ImprovementKind, PrRef, SessionRef, Signal, SignalGroup};
    use crate::suggestions::notebook::{NotebookEdit, Stage};
    use crate::suggestions::policy::DAY_MS;

    const NOW: u64 = 400 * DAY_MS;

    fn failed_tool(tag: &str, session: &str, at: u64) -> Observation {
        Observation {
            signal: Signal::FailedTool,
            tag: tag.into(),
            session: Some(SessionRef {
                agent: "claude".into(),
                id: session.into(),
            }),
            pr: None,
            checkout: Some("/repo".into()),
            at_unix_ms: at,
            always: false,
            head: None,
            improvement: None,
        }
    }

    #[test]
    fn a_session_keeps_its_improvements_up_to_the_cap_and_its_kind() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = SuggestionStore::open(&path(dir.path())).unwrap();
        let improvement = |text: String, session: &str| Observation {
            signal: Signal::Improvement,
            improvement: Some(ImprovementKind::NoDoc),
            ..failed_tool(&text, session, NOW)
        };
        let facts = (0..policy::IMPROVEMENTS_PER_SESSION + 2)
            .map(|n| improvement(format!("포트 규칙 {n}"), "a"))
            .chain([improvement("포트 규칙 0".into(), "b")])
            .collect();
        let written = store.record("p", facts, NOW).unwrap().written;
        assert_eq!(
            written.len(),
            policy::IMPROVEMENTS_PER_SESSION + 1,
            "the cap is one session's; another session keeps its own"
        );
        let index = store.index("p", &Notebook::default()).unwrap();
        assert!(
            index
                .entries
                .iter()
                .all(|entry| entry.improvement == Some(ImprovementKind::NoDoc))
        );
    }

    #[test]
    fn a_record_is_kept_once_per_session_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        let (mut store, opened) = SuggestionStore::open(&file).unwrap();
        assert_eq!(opened, Opened::Fresh);
        let written = store
            .record(
                "p",
                vec![
                    failed_tool("cargo test", "a", NOW),
                    failed_tool("cargo  test", "a", NOW + 1),
                    failed_tool("cargo test", "b", NOW),
                ],
                NOW,
            )
            .unwrap()
            .written;
        assert_eq!(
            written,
            [RecordId(1), RecordId(2)],
            "the second sighting folds"
        );
        let written = store
            .record("p", vec![failed_tool("cargo test", "a", NOW + 5)], NOW)
            .unwrap()
            .written;
        assert!(
            written.is_empty(),
            "a later sighting in the same session folds too"
        );
        drop(store);

        let (store, opened) = SuggestionStore::open(&file).unwrap();
        assert_eq!(opened, Opened::Existing);
        let records = store.records("p").unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|(_, fresh)| *fresh));
        assert!(store.records("other").unwrap().is_empty());
    }

    /// B4, B15: a pull request's check stays one record, but a failure on
    /// a newer head moves it there, with that failure's time; the same
    /// head again moves nothing.
    #[test]
    fn a_pull_request_failure_on_a_newer_head_moves_its_one_record() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = SuggestionStore::open(&path(dir.path())).unwrap();
        let check = |head: &str, at: u64| Observation {
            signal: Signal::CheckFailed,
            tag: "verify".into(),
            session: None,
            pr: Some(PrRef {
                number: 41,
                url: "https://github.com/acme/app/pull/41".into(),
            }),
            checkout: None,
            at_unix_ms: at,
            always: false,
            head: Some(head.into()),
            improvement: None,
        };
        let first = store.record("p", vec![check("a", NOW)], NOW).unwrap();
        assert_eq!(first.written, [RecordId(1)]);
        let newer = store.record("p", vec![check("b", NOW + 5)], NOW).unwrap();
        assert_eq!(
            (newer.written, newer.moved),
            (Vec::new(), vec![RecordId(1)])
        );
        let again = store.record("p", vec![check("b", NOW + 9)], NOW).unwrap();
        assert!(again.moved.is_empty(), "the same head moves nothing");
        let kept = store.records("p").unwrap();
        assert_eq!(kept.len(), 1, "still one pull request");
        assert_eq!(kept[0].0.fact.head.as_deref(), Some("b"));
        assert_eq!(kept[0].0.fact.at_unix_ms, NOW + 5);
    }

    #[test]
    fn records_older_than_ninety_days_or_past_the_cap_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = SuggestionStore::open(&path(dir.path())).unwrap();
        store
            .record(
                "p",
                vec![
                    failed_tool("old", "a", NOW - policy::RECORD_RETENTION_MS - 1),
                    failed_tool("new", "a", NOW - DAY_MS),
                ],
                NOW - DAY_MS,
            )
            .unwrap();
        let pruned = store.record("p", Vec::new(), NOW).unwrap().pruned;
        assert_eq!(pruned.records, 1);
        let kept = store.records("p").unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0.fact.tag, "new");
    }

    /// A line's references go with the records they name, so one problem
    /// that recurs for years holds no more than the ledger keeps.
    #[test]
    fn a_pruned_record_leaves_every_line_that_named_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = SuggestionStore::open(&path(dir.path())).unwrap();
        let old = NOW - policy::RECORD_RETENTION_MS - 1;
        store
            .record(
                "p",
                vec![
                    failed_tool("cargo test", "a", old),
                    failed_tool("cargo test", "b", NOW - DAY_MS),
                ],
                NOW - DAY_MS,
            )
            .unwrap();
        let (mut notebook, _) = store.notebook("p").unwrap();
        let index = store.index("p", &notebook).unwrap();
        let applied = notebook
            .apply(
                NotebookEdit::NewLine {
                    discomfort: None,
                    records: vec![RecordId(1), RecordId(2)],
                    basis: None,
                    outcome: None,
                },
                &index,
                NOW - DAY_MS,
            )
            .unwrap();
        store.save_edit("p", &notebook, &applied).unwrap();

        let pruned = store.record("p", Vec::new(), NOW).unwrap().pruned;
        assert_eq!(
            pruned,
            Pruned {
                records: 1,
                references: 1
            }
        );
        let (notebook, _) = store.notebook("p").unwrap();
        assert_eq!(notebook.lines[0].records, [RecordId(2)]);
    }

    #[test]
    fn an_accepted_edit_is_on_disk_with_its_seen_records_and_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        let (mut store, _) = SuggestionStore::open(&file).unwrap();
        store
            .record(
                "p",
                vec![failed_tool("a", "s", NOW), failed_tool("b", "s", NOW)],
                NOW,
            )
            .unwrap();
        let (mut notebook, _) = store.notebook("p").unwrap();
        let index = store.index("p", &notebook).unwrap();
        let applied = notebook
            .apply(
                NotebookEdit::NewLine {
                    discomfort: Some("x".into()),
                    records: vec![RecordId(1)],
                    basis: None,
                    outcome: None,
                },
                &index,
                NOW,
            )
            .unwrap();
        store.save_edit("p", &notebook, &applied).unwrap();
        let mut line = notebook.lines[0].clone();
        line.stage = Stage::Suggested;
        let life = LineLife {
            raised_at_unix_ms: Some(NOW),
            ..LineLife::default()
        };
        store.save_line("p", &line, &life).unwrap();
        drop(store);

        let (store, _) = SuggestionStore::open(&file).unwrap();
        let (notebook, lives) = store.notebook("p").unwrap();
        assert_eq!(notebook.lines, [line]);
        assert_eq!(notebook.next_line, 2);
        assert_eq!(lives[&LineId(1)], life);
        let index = store.index("p", &notebook).unwrap();
        assert_eq!(index.get(RecordId(1)).unwrap().line, Some(LineId(1)));
        assert!(!index.get(RecordId(1)).unwrap().fresh);
        assert!(index.get(RecordId(2)).unwrap().fresh);
    }

    #[test]
    fn a_file_of_a_newer_schema_is_refused_and_a_corrupt_one_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        drop(SuggestionStore::open(&file).unwrap());
        Connection::open(&file)
            .unwrap()
            .execute_batch("PRAGMA user_version=99")
            .unwrap();
        assert_eq!(
            SuggestionStore::open(&file).err().as_deref(),
            Some("suggestions_store_newer")
        );

        let broken = dir.path().join("broken").join("suggestions.sqlite3");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"not a database at all, just bytes").unwrap();
        let (_, opened) = SuggestionStore::open(&broken).unwrap();
        assert_eq!(opened, Opened::Rebuilt);
        assert!(sibling(&broken, ".corrupt").exists());
    }

    #[test]
    fn settings_and_the_schedule_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        let (mut store, _) = SuggestionStore::open(&file).unwrap();
        assert_eq!(store.state("p").unwrap(), ProjectState::default());
        let mut state = ProjectState::default();
        state.settings.suggestions_enabled = false;
        state.settings.groups_off.insert(SignalGroup::Github);
        state.last_completed_day = Some("2026-10-27".into());
        store.save_state("p", &state).unwrap();
        drop(store);
        let (store, _) = SuggestionStore::open(&file).unwrap();
        assert_eq!(store.state("p").unwrap(), state);
        assert_eq!(store.projects().unwrap(), ["p"]);
    }
}
