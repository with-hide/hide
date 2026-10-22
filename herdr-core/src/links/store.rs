//! `links.sqlite3`: the link record's one file (D-22, D-39).
//!
//! Nodes are projects, repositories, pull requests, worktrees and sessions;
//! edges carry their source and their first and last time. Every write is an
//! upsert on a natural key, so a file read twice, a backfill run again or a
//! restart converge on the same rows (B37). Only the link worker writes;
//! panels and `hide links` open it read-only.

use super::{
    CREATED_AFTER_MS, CREATED_BEFORE_MS, DAY_MS, FileState, IssueSource, LinkedIssue, LinkedParent,
    LinkedSession, OPEN_EXCEPTION_MS, PANEL_SESSION_LIMIT, PaneFact, ParentFact, ProjectFacts,
    ProjectLinkSummary, SESSION_PR_LIMIT, STORE_PAGE_LIMIT, SUMMARY_SESSION_LIMIT, SessionPrChip,
    SessionRole, WorktreeFact, within,
};
use crate::links::ledger::SessionIdentity;
use hide_session::ConversationCheckpoint;
use hide_session::links::ReadAnswer;
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[path = "ledger_store.rs"]
mod ledger;

const SCHEMA_VERSION: i64 = 9;

const SCHEMA: &str = "
CREATE TABLE projects(key TEXT PRIMARY KEY, device TEXT NOT NULL, root TEXT NOT NULL,
    repo TEXT, name TEXT, seen_at INTEGER NOT NULL);
CREATE TABLE repo_names(name TEXT PRIMARY KEY, repo TEXT NOT NULL);
CREATE TABLE prs(repo TEXT NOT NULL, number INTEGER NOT NULL, project TEXT NOT NULL,
    branch TEXT NOT NULL, title TEXT NOT NULL, url TEXT NOT NULL, created_at INTEGER,
    closed_at INTEGER, merged_at INTEGER, seen_at INTEGER NOT NULL, PRIMARY KEY(repo, number));
CREATE INDEX prs_project ON prs(project, number);
CREATE INDEX prs_branch ON prs(repo, branch);
CREATE TABLE pr_issues(repo TEXT NOT NULL, number INTEGER NOT NULL, issue TEXT NOT NULL,
    source TEXT NOT NULL, start_at INTEGER NOT NULL, end_at INTEGER,
    PRIMARY KEY(repo, number, issue, source));
CREATE INDEX pr_issues_issue ON pr_issues(issue);
CREATE TABLE worktrees(project TEXT NOT NULL, path TEXT NOT NULL, branch TEXT NOT NULL,
    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(project, path, branch));
CREATE TABLE sessions(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL, path TEXT,
    cwd TEXT, interactive INTEGER, started_at INTEGER, ended_at INTEGER, first_parent_uuid TEXT,
    last_uuid TEXT, continues TEXT, forked_from TEXT, last_request TEXT, last_request_at INTEGER,
    file_gone INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(device, agent, id));
CREATE INDEX sessions_last_uuid ON sessions(device, agent, last_uuid);
CREATE INDEX sessions_path ON sessions(device, path);
CREATE TABLE session_branches(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
    file TEXT NOT NULL, branch TEXT NOT NULL, first_at INTEGER NOT NULL, last_at INTEGER NOT NULL,
    last_request TEXT, last_request_at INTEGER,
    PRIMARY KEY(device, agent, id, file, branch, first_at));
CREATE INDEX session_branches_branch ON session_branches(branch, last_at);
CREATE TABLE session_prs(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
    file TEXT NOT NULL, repo_name TEXT NOT NULL, number INTEGER NOT NULL, at INTEGER NOT NULL,
    request TEXT, PRIMARY KEY(device, agent, id, repo_name, number));
CREATE INDEX session_prs_pr ON session_prs(repo_name, number);
CREATE TABLE session_parents(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
    parent_agent TEXT NOT NULL, parent_id TEXT NOT NULL, parent_name TEXT NOT NULL,
    PRIMARY KEY(device, agent, id));
CREATE TABLE cursors(device TEXT NOT NULL, path TEXT NOT NULL, agent TEXT NOT NULL,
    stamp TEXT NOT NULL, checkpoint TEXT, session TEXT, last_branch TEXT,
    subagent INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(device, path));
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// A session joined to a pull request: its role, its request, and whether
/// it worked on the pull request's branch rather than only printing it.
type Member = (SessionRow, SessionRole, Option<String>, bool);

/// The file `pane` in `session_branches`: a span a Hide pane's place said.
const PANE_FILE: &str = "pane";

/// How the store came to be open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    Fresh,
    Existing,
    /// The file failed its integrity check and was set aside as
    /// `links.sqlite3.corrupt`; this one is new and fills again (B38).
    Rebuilt,
}

pub struct LinkStore {
    connection: Connection,
}

enum OpenFailure {
    Corrupt,
    Newer,
    Failed(String),
}

/// A failure's code: what a caller words or logs, never a path or content.
pub fn code(error: &rusqlite::Error) -> &'static str {
    match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => "links_store_full",
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "links_store_busy",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "links_store_corrupt",
        _ => "links_store_failed",
    }
}

fn failed(error: rusqlite::Error) -> String {
    code(&error).to_owned()
}

fn classify(error: rusqlite::Error) -> OpenFailure {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => OpenFailure::Corrupt,
        _ => OpenFailure::Failed(code(&error).to_owned()),
    }
}

/// Where a corrupt file is set aside; one copy, the newest.
pub fn corrupt_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".corrupt");
    path.with_file_name(name)
}

/// A path spelled for an SQLite `file:` URI: everything but unreserved
/// characters and `/` escaped.
fn uri_path(path: &Path) -> String {
    const ESCAPED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'/')
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(&path.to_string_lossy(), ESCAPED).to_string()
}

fn sidecars(path: &Path) -> [PathBuf; 2] {
    ["-wal", "-shm"].map(|suffix| {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        path.with_file_name(name)
    })
}

/// Which of the asked paths the store's own node still holds.
pub type FilesPresent<'a> = dyn FnMut(&[String]) -> Result<BTreeSet<String>, String> + 'a;

/// Which of the asked paths this machine's node still holds, asked without
/// changing anything.
pub type FilesHeld<'a> = dyn Fn(&[String]) -> Result<BTreeSet<String>, String> + 'a;

/// This machine's device id and how to ask its node which session files it
/// still holds, for the reads that show a session's file state.
#[derive(Clone, Copy)]
pub struct LocalFiles<'a> {
    pub device: &'a str,
    pub present: &'a FilesHeld<'a>,
}

impl LinkStore {
    /// Called only by the existing links writer, never under Runtime's mutex.
    pub fn apply_direct_observation(
        &mut self,
        fact: &super::observation::DirectFact,
        now: u64,
    ) -> Result<bool, String> {
        use super::observation::{DirectFact, DirectOutcome, EVENT_LIMIT, TOTAL_BYTE_LIMIT};
        fact.validate(now)?;
        let id = fact.id();
        let tx = self.connection.transaction().map_err(failed)?;
        tx.execute(
            "DELETE FROM direct_observations WHERE at<=?1",
            [now.saturating_sub(super::observation::RETENTION_MS) as i64],
        )
        .map_err(failed)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT fact FROM direct_observations WHERE id=?1",
                [&id],
                |row| row.get(0),
            )
            .optional()
            .map_err(failed)?;
        let mut next = fact.clone();
        if let Some(previous) = previous.as_deref() {
            let held: DirectFact =
                serde_json::from_str(previous).map_err(|_| "direct_observation_value_invalid")?;
            if held.recipient.is_some()
                && next.recipient.is_some()
                && held.recipient != next.recipient
            {
                return Err("direct_observation_identity_conflict".into());
            }
            next.at_unix_ms = held.at_unix_ms.min(next.at_unix_ms);
            // Preserve a complete submission, but never combine separate incomplete
            // callbacks into evidence that neither callback independently supplied.
            if held.outcome == DirectOutcome::Submitted && held.recipient.is_some() {
                next.outcome = held.outcome;
                next.recipient = held.recipient;
            }
            if next.outcome == DirectOutcome::Submitted && next.recipient.is_some() {
                next.reason = None;
            }
        }
        next.validate(now)?;
        let encoded = next.encoded()?;
        if previous.as_deref() == Some(encoded.as_str()) {
            tx.commit().map_err(failed)?;
            return Ok(false);
        }
        let (count, bytes): (usize, usize) = tx
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(bytes),0) FROM direct_observations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(failed)?;
        if count + usize::from(previous.is_none()) > EVENT_LIMIT
            || bytes.saturating_sub(previous.as_ref().map_or(0, String::len)) + encoded.len()
                > TOTAL_BYTE_LIMIT
        {
            return Err("direct_observation_store_capacity".into());
        }
        tx.execute("INSERT INTO direct_observations VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET at=excluded.at,bytes=excluded.bytes,fact=excluded.fact", params![id,next.at_unix_ms as i64,encoded.len() as i64,encoded]).map_err(failed)?;
        tx.commit().map_err(failed)?;
        Ok(true)
    }

    /// The bounded retained metadata; no command or prompt field exists in its schema.
    pub fn direct_observations(
        &self,
        now: u64,
    ) -> Result<Vec<super::observation::DirectFact>, String> {
        let mut statement = self.connection.prepare_cached("SELECT fact FROM direct_observations WHERE at>?1 ORDER BY at DESC,id DESC LIMIT ?2").map_err(failed)?;
        let encoded = statement
            .query_map(
                params![
                    now.saturating_sub(super::observation::RETENTION_MS) as i64,
                    super::observation::EVENT_LIMIT as i64
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        encoded
            .into_iter()
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| "direct_observation_value_invalid".into())
            })
            .collect()
    }

    pub fn prune_direct_observations(&mut self, now: u64) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM direct_observations WHERE at<=?1",
                [now.saturating_sub(super::observation::RETENTION_MS) as i64],
            )
            .map_err(failed)?;
        Ok(())
    }

    /// Moves whenever a row of the file changes, through this connection or
    /// another, so a reader can tell that an answer it derived still stands.
    pub fn revision(&self) -> Result<(u64, i64), String> {
        let version = self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .map_err(failed)?;
        Ok((self.connection.total_changes(), version))
    }

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
            Err(OpenFailure::Newer) => Err("links_store_newer".to_owned()),
            Err(OpenFailure::Failed(code)) => Err(code),
            Err(OpenFailure::Corrupt) => {
                std::fs::rename(path, corrupt_path(path))
                    .map_err(|_| "links_store_set_aside_failed".to_owned())?;
                for sidecar in sidecars(path) {
                    let _ = std::fs::remove_file(sidecar);
                }
                match Self::open_writer(path) {
                    Ok(store) => Ok((store, Opened::Rebuilt)),
                    Err(OpenFailure::Failed(code)) => Err(code),
                    Err(_) => Err("links_store_corrupt".to_owned()),
                }
            }
        }
    }

    fn open_writer(path: &Path) -> Result<Self, OpenFailure> {
        hide_platform::fs::private::open_or_create_file(path)
            .map_err(|_| OpenFailure::Failed("links_store_unavailable".to_owned()))?;
        let connection = Connection::open(path).map_err(classify)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(classify)?;
        connection
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA secure_delete=ON; \
                 PRAGMA max_page_count={STORE_PAGE_LIMIT};"
            ))
            .map_err(classify)?;
        let check: String = connection
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(classify)?;
        if check != "ok" {
            return Err(OpenFailure::Corrupt);
        }
        migrate(&connection)?;
        // Observations before a stopped writer cannot prove which connection
        // an unobserved historical session started under.
        connection
            .execute("DELETE FROM ledger_project_connections", [])
            .map_err(classify)?;
        Ok(Self { connection })
    }

    /// A reader's connection: the panel and `hide links` never write.
    pub fn open_read_only(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err("links_store_missing".to_owned());
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(failed)?;
        connection
            .busy_timeout(Duration::from_millis(500))
            .map_err(failed)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(failed)?;
        if version != SCHEMA_VERSION {
            return Err("links_store_unavailable".to_owned());
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

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [key, value],
            )
            .map(|_| ())
            .map_err(failed)
    }

    pub fn delete_meta(&self, key: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM meta WHERE key=?1", [key])
            .map(|_| ())
            .map_err(failed)
    }

    /// Whether any row still names `device` (PRD core-host-node B2).
    pub fn has_device(&self, device: &str) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE device=?1) \
                 OR EXISTS(SELECT 1 FROM sessions WHERE device=?1) \
                 OR EXISTS(SELECT 1 FROM cursors WHERE device=?1) \
                 OR EXISTS(SELECT 1 FROM session_parents WHERE parent_device=?1)",
                [device],
                |row| row.get(0),
            )
            .map_err(failed)
    }

    /// A consistent copy of the whole file at `destination`, which must not
    /// exist yet; the writer runs in WAL mode, so the main file alone may
    /// miss committed rows.
    pub fn copy_to(&self, destination: &Path) -> Result<(), String> {
        let destination = destination.to_str().ok_or("links_copy_path_not_utf8")?;
        self.connection
            .execute("VACUUM INTO ?1", [destination])
            .map(|_| ())
            .map_err(failed)
    }

    /// Moves every row of device `from` to `to` in one transaction: each
    /// table's `device` column and each project key, a digest of device and
    /// root (PRD core-host-node B2). A row the move would land on fails the
    /// whole move. Returns how many projects moved.
    pub fn convert_device(&mut self, from: &str, to: &str) -> Result<usize, String> {
        let tx = self.connection.transaction().map_err(failed)?;
        let projects = {
            let mut statement = tx
                .prepare("SELECT key, root FROM projects WHERE device=?1")
                .map_err(failed)?;
            statement
                .query_map([from], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?
        };
        for (old, root) in &projects {
            let new = hide_project::project_id(to, Path::new(root));
            tx.execute(
                "UPDATE projects SET key=?1, device=?2 WHERE key=?3",
                params![new, to, old],
            )
            .map_err(|error| format!("moving project {old} to {new} failed: {error}"))?;
            for table in ["prs", "worktrees", "ledger_project_connections"] {
                tx.execute(
                    &format!("UPDATE {table} SET project=?1 WHERE project=?2"),
                    params![new, old],
                )
                .map_err(|error| format!("moving {table} of {old} to {new} failed: {error}"))?;
            }
        }
        // Every other table that names a device, read from the schema so a
        // table added later cannot keep the old name.
        let tables = {
            let mut statement = tx
                .prepare(
                    "SELECT m.name FROM sqlite_master m WHERE m.type='table' \
                     AND m.name!='projects' AND EXISTS(SELECT 1 FROM \
                     pragma_table_info(m.name) c WHERE c.name='device')",
                )
                .map_err(failed)?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?
        };
        for table in tables {
            tx.execute(
                &format!("UPDATE \"{table}\" SET device=?1 WHERE device=?2"),
                [to, from],
            )
            .map_err(|error| format!("moving {table} to {to} failed: {error}"))?;
        }
        tx.execute(
            "UPDATE session_parents SET parent_device=?1 WHERE parent_device=?2",
            [to, from],
        )
        .map_err(|_| "links_parent_device_move_failed".to_owned())?;
        tx.commit().map_err(failed)?;
        Ok(projects.len())
    }

    /// Records that each `(old, new)` project id is now `new`, for the
    /// retention the old session search file names under `old`. An alias
    /// that pointed at `old` follows it to `new`. Answers how many aliases
    /// changed; ids already recorded count none.
    pub fn alias_retention(&mut self, pairs: &[(String, String)]) -> Result<usize, String> {
        let tx = self.connection.transaction().map_err(failed)?;
        let mut moved = 0;
        for (old, new) in pairs.iter().filter(|(old, new)| old != new) {
            moved += tx
                .execute(
                    "UPDATE retention_aliases SET new=?2 WHERE new=?1",
                    [old, new],
                )
                .map_err(failed)?;
            // A second conversion of the same ids changes nothing.
            moved += tx
                .execute(
                    "INSERT INTO retention_aliases(old, new) VALUES(?1, ?2) \
                     ON CONFLICT(old) DO UPDATE SET new=excluded.new \
                     WHERE new != excluded.new",
                    [old, new],
                )
                .map_err(failed)?;
        }
        tx.commit().map_err(failed)?;
        Ok(moved)
    }

    /// Each project's Copied history days, read from the old session search
    /// file's `policy` through a read-only ATTACH (D-22, D-20); a project
    /// with no row keeps the default. The file is only ever read: no file,
    /// or a file without the table, is no policy. A row under an id a
    /// conversion or a move replaced (`retention_aliases`) is that project's
    /// under its new id, unless the new id has a row of its own.
    pub fn policies(&self, search: &Path) -> Result<HashMap<String, u16>, String> {
        if !search.is_file() {
            return Ok(HashMap::new());
        }
        let uri = format!("file:{}?mode=ro", uri_path(search));
        self.connection
            .execute("ATTACH DATABASE ?1 AS search", [uri])
            .map_err(|_| "links_policy_unreadable".to_owned())?;
        let read = (|| {
            let has_table: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM search.sqlite_master \
                 WHERE type='table' AND name='policy')",
                [],
                |row| row.get(0),
            )?;
            if !has_table {
                return Ok(Vec::new());
            }
            let mut statement = self
                .connection
                .prepare("SELECT project, days FROM search.policy")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })();
        let detached = self.connection.execute("DETACH DATABASE search", []);
        let rows = read.map_err(|_| "links_policy_unreadable".to_owned())?;
        detached.map_err(failed)?;
        let aliases: HashMap<String, String> = {
            let mut statement = self
                .connection
                .prepare("SELECT old, new FROM retention_aliases")
                .map_err(failed)?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(failed)?
                .collect::<Result<_, _>>()
                .map_err(failed)?
        };
        let days = |days: i64| u16::try_from(days).unwrap_or(u16::MAX);
        let mut policies: HashMap<String, u16> = rows
            .iter()
            .map(|(project, value)| (project.clone(), days(*value)))
            .collect();
        for (project, value) in &rows {
            if let Some(new) = aliases.get(project) {
                policies.entry(new.clone()).or_insert(days(*value));
            }
        }
        Ok(policies)
    }

    /// A project as the runtime knows it now: its repository, worktrees, and
    /// the pull requests GitHub answered with their issues (D-27, D-35).
    pub fn apply_project(&mut self, project: &ProjectFacts, now: u64) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        let repo = match project.repository.as_deref() {
            Some(name) => Some(name_repo(&tx, name, project.repository_id.as_deref())?),
            None => None,
        };
        tx.execute(
            "INSERT INTO projects VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(key) DO UPDATE SET \
             root=excluded.root, repo=COALESCE(excluded.repo, projects.repo), \
             name=COALESCE(excluded.name, projects.name), seen_at=excluded.seen_at",
            params![
                project.key,
                project.device_id,
                project.root,
                repo,
                project.repository.as_deref().map(str::to_ascii_lowercase),
                now as i64
            ],
        )
        .map_err(failed)?;
        // A repository observation can prove ownership only for sessions
        // started after that connection was known. A later historical file
        // must not inherit today's repository from its cwd.
        tx.execute("INSERT INTO ledger_project_connections(project,repository,since_at,epoch) VALUES(?1,?2,?3,?4) ON CONFLICT(project) DO UPDATE SET since_at=CASE WHEN repository IS NOT excluded.repository OR epoch!=excluded.epoch THEN excluded.since_at ELSE since_at END,repository=excluded.repository,epoch=excluded.epoch",
            params![project.key,project.repository.as_deref().map(str::to_ascii_lowercase),now as i64,project.connection_epoch as i64]).map_err(failed)?;
        for worktree in &project.worktrees {
            let Some(branch) = worktree.branch.as_deref() else {
                continue;
            };
            tx.execute(
                "INSERT INTO worktrees VALUES(?1,?2,?3,?4,?4) ON CONFLICT(project,path,branch) \
                 DO UPDATE SET last_seen=excluded.last_seen",
                params![project.key, worktree.path, branch, now as i64],
            )
            .map_err(failed)?;
        }
        if project.prs_read {
            for pr in &project.prs {
                let repo = repo_of_name(&tx, &pr.repository)?;
                tx.execute(
                    "INSERT INTO prs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
                     ON CONFLICT(repo,number) DO UPDATE SET project=excluded.project, \
                     branch=excluded.branch, title=excluded.title, url=excluded.url, \
                     created_at=COALESCE(excluded.created_at, prs.created_at), \
                     closed_at=excluded.closed_at, merged_at=excluded.merged_at, \
                     seen_at=excluded.seen_at",
                    params![
                        repo,
                        pr.number as i64,
                        project.key,
                        pr.branch,
                        pr.title,
                        pr.url,
                        pr.created_at.map(|at| at as i64),
                        pr.closed_at.map(|at| at as i64),
                        pr.merged_at.map(|at| at as i64),
                        now as i64,
                    ],
                )
                .map_err(failed)?;
                for source in [IssueSource::Closes, IssueSource::Hide] {
                    if source == IssueSource::Hide && !pr.hide_issue_known {
                        continue;
                    }
                    let current = pr
                        .issues
                        .iter()
                        .filter(|(_, from)| *from == source)
                        .map(|(key, _)| super::issue_key(key))
                        .collect::<BTreeSet<_>>();
                    // The source says the link is gone: close it at this
                    // pass, never delete it (B27).
                    let open = issues_open(&tx, &repo, pr.number, source)?;
                    for issue in open.iter().filter(|issue| !current.contains(*issue)) {
                        tx.execute(
                            "UPDATE pr_issues SET end_at=?5 WHERE repo=?1 AND number=?2 \
                             AND issue=?3 AND source=?4",
                            params![repo, pr.number as i64, issue, source.as_str(), now as i64],
                        )
                        .map_err(failed)?;
                    }
                    for issue in current {
                        tx.execute(
                            "INSERT INTO pr_issues VALUES(?1,?2,?3,?4,?5,NULL) \
                             ON CONFLICT(repo,number,issue,source) DO UPDATE SET end_at=NULL",
                            params![repo, pr.number as i64, issue, source.as_str(), now as i64],
                        )
                        .map_err(failed)?;
                    }
                }
            }
        }
        ledger::apply_output_facts(&tx, &project.outputs)?;
        tx.commit().map_err(failed)
    }

    /// The stamps of the files already read on a device, so a listing reads
    /// only what changed since.
    pub fn stamps(&self, device: &str) -> Result<HashMap<String, String>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT path, stamp FROM cursors WHERE device=?1")
            .map_err(failed)?;
        statement
            .query_map([device], |row| Ok((row.get(0)?, row.get(1)?)))
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    /// Where the last read of a file stopped.
    pub fn checkpoint(
        &self,
        device: &str,
        path: &str,
    ) -> Result<Option<ConversationCheckpoint>, String> {
        let stored: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT checkpoint FROM cursors WHERE device=?1 AND path=?2",
                [device, path],
                |row| row.get(0),
            )
            .optional()
            .map_err(failed)?;
        Ok(stored
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok()))
    }

    /// One file's read: its facts join the session it names, and the cursor
    /// moves to where the read stopped. `stamp` is recorded only once the
    /// file has been read to its end, so a partly read file is read again.
    #[cfg(test)]
    pub fn apply_answer(
        &mut self,
        device: &str,
        answer: &ReadAnswer,
        stamp: &str,
    ) -> Result<(), String> {
        self.apply_answer_observed(device, answer, stamp, Some(u64::MAX))
    }

    /// Only a current project observation may certify a native birth, and
    /// that birth must precede the node read that witnessed it.
    pub fn apply_answer_observed(
        &mut self,
        device: &str,
        answer: &ReadAnswer,
        stamp: &str,
        origin_before: Option<u64>,
    ) -> Result<(), String> {
        // Takes the write lock at the start: a deferred transaction that reads
        // and then writes after another connection committed fails busy at once,
        // without the busy timeout, where this one waits for that writer.
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(failed)?;
        let path = answer.path.as_str();
        if let Some(error) = answer.error.as_deref() {
            if error == "session_file_missing" {
                tx.execute(
                    "UPDATE sessions SET file_gone=1 WHERE device=?1 AND path=?2",
                    [device, path],
                )
                .map_err(failed)?;
                tx.execute(
                    "DELETE FROM cursors WHERE device=?1 AND path=?2",
                    [device, path],
                )
                .map_err(failed)?;
            } else {
                // Recorded as read, so the same failure is not retried until
                // the file changes; the worker logs the code.
                tx.execute(
                    "INSERT INTO cursors(device,path,agent,stamp) VALUES(?1,?2,?3,?4) \
                     ON CONFLICT(device,path) DO UPDATE SET stamp=excluded.stamp",
                    params![device, path, answer.agent.as_str(), stamp],
                )
                .map_err(failed)?;
            }
            return tx.commit().map_err(failed);
        }
        let previous: Option<CursorRow> = tx
            .query_row(
                "SELECT checkpoint, last_branch, session, subagent FROM cursors \
                 WHERE device=?1 AND path=?2",
                [device, path],
                |row| {
                    Ok(CursorRow {
                        checkpoint: row.get(0)?,
                        last_branch: row.get(1)?,
                        session: row.get(2)?,
                        subagent: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()
            .map_err(failed)?;
        let named_before = previous.as_ref().and_then(|row| row.session.clone());
        let continuing = !answer.rescanned
            && previous
                .as_ref()
                .is_some_and(|row| row.checkpoint.is_some());
        if !continuing {
            // Read from the start: what this file said before is said again.
            tx.execute(
                "DELETE FROM session_branches WHERE device=?1 AND file=?2",
                [device, path],
            )
            .map_err(failed)?;
            tx.execute(
                "DELETE FROM session_prs WHERE device=?1 AND file=?2",
                [device, path],
            )
            .map_err(failed)?;
        }
        let seen_session = previous.as_ref().and_then(|row| row.session.clone());
        let previous = previous.filter(|_| continuing);
        // A Codex file names its session only in its first record: a later
        // read of it belongs to the session the cursor recorded.
        let mut owned = answer.facts.clone();
        if owned.session_id.is_none()
            && let Some(row) = previous.as_ref()
        {
            owned.session_id.clone_from(&row.session);
            owned.subagent = row.subagent;
        }
        let facts = &owned;
        let agent = answer.agent.as_str();
        let mut last_branch = previous.and_then(|row| row.last_branch);
        if let Some(id) = facts.session_id.as_deref() {
            // A chunk that printed an address before any request of its own
            // continues the request an earlier read recorded.
            let before: Option<String> = if continuing {
                tx.query_row(
                    "SELECT last_request FROM sessions WHERE device=?1 AND agent=?2 AND id=?3",
                    [device, agent, id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(failed)?
                .flatten()
            } else {
                None
            };
            if named_before.as_deref() != Some(id) {
                name_file(&tx, device, agent, path, id)?;
            }
            upsert_session(&tx, device, agent, id, path, facts)?;
            if !facts.subagent {
                ledger::capture_session_origin(
                    &tx,
                    &crate::links::ledger::SessionIdentity {
                        device_id: device.into(),
                        agent: agent.into(),
                        session_id: id.into(),
                    },
                    facts.cwd.as_deref(),
                    facts.first_at_unix_ms.filter(|at| {
                        !continuing
                            && seen_session.as_deref() != Some(id)
                            && origin_before.is_some_and(|before| *at < before)
                    }),
                )?;
                ledger::apply_reader(&tx, device, agent, id, facts)?;
            }
            for (index, span) in facts.spans.iter().enumerate() {
                let stated = if span.inherited {
                    last_branch.clone()
                } else {
                    span.branch.clone()
                };
                let Some(branch) = stated.as_deref() else {
                    last_branch = None;
                    continue;
                };
                let extend = index == 0 && last_branch.as_deref() == Some(branch);
                let extended = extend
                    && tx
                        .execute(
                            "UPDATE session_branches SET last_at=MAX(last_at,?6), \
                             last_request=COALESCE(?7,last_request), \
                             last_request_at=CASE WHEN ?7 IS NULL THEN last_request_at ELSE ?6 END \
                             WHERE rowid=(SELECT rowid FROM session_branches WHERE device=?1 \
                             AND agent=?2 AND id=?3 AND file=?4 AND branch=?5 \
                             ORDER BY first_at DESC LIMIT 1)",
                            params![
                                device,
                                agent,
                                id,
                                path,
                                branch,
                                span.last_at_unix_ms as i64,
                                span.last_request,
                            ],
                        )
                        .map_err(failed)?
                        > 0;
                if !extended {
                    tx.execute(
                        "INSERT INTO session_branches VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) \
                         ON CONFLICT(device,agent,id,file,branch,first_at) DO UPDATE SET \
                         last_at=MAX(last_at,excluded.last_at), \
                         last_request=COALESCE(excluded.last_request,last_request), \
                         last_request_at=COALESCE(excluded.last_request_at,last_request_at)",
                        params![
                            device,
                            agent,
                            id,
                            path,
                            branch,
                            span.first_at_unix_ms as i64,
                            span.last_at_unix_ms as i64,
                            span.last_request,
                            span.last_request
                                .as_ref()
                                .map(|_| span.last_at_unix_ms as i64),
                        ],
                    )
                    .map_err(failed)?;
                }
                last_branch = Some(branch.to_owned());
            }
            let mut held: i64 = tx
                .query_row(
                    "SELECT count(*) FROM session_prs WHERE device=?1 AND agent=?2 AND id=?3",
                    [device, agent, id],
                    |row| row.get(0),
                )
                .map_err(failed)?;
            for mark in &facts.prs {
                let known = tx
                    .query_row(
                        "SELECT 1 FROM session_prs WHERE device=?1 AND agent=?2 AND id=?3 \
                         AND repo_name=?4 AND number=?5",
                        params![
                            device,
                            agent,
                            id,
                            mark.repository.to_ascii_lowercase(),
                            mark.number as i64
                        ],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(failed)?
                    .is_some();
                if !known && held >= SESSION_PR_LIMIT as i64 {
                    crate::diagnostic!(serde_json::json!({
                        "component": "links", "kind": "session.pr_limit",
                        "device_id": device, "agent": agent, "session_id": id,
                    }));
                    break;
                }
                // The first sighting stands: it is the one GitHub's creation
                // time is matched against (D-44).
                tx.execute(
                    "INSERT INTO session_prs VALUES(?1,?2,?3,?4,?5,?6,?7,?8) \
                     ON CONFLICT(device,agent,id,repo_name,number) DO UPDATE SET \
                     request=CASE WHEN excluded.at < at THEN excluded.request ELSE request END, \
                     at=MIN(at, excluded.at)",
                    params![
                        device,
                        agent,
                        id,
                        path,
                        mark.repository.to_ascii_lowercase(),
                        mark.number as i64,
                        mark.at_unix_ms as i64,
                        mark.request.as_ref().or(before.as_ref()),
                    ],
                )
                .map_err(failed)?;
                if !known {
                    held += 1;
                }
            }
            link_continuations(&tx, device)?;
        }
        let checkpoint = answer
            .checkpoint
            .as_ref()
            .map(|checkpoint| serde_json::to_string(checkpoint).unwrap_or_default());
        let stamp = if answer.has_more { "" } else { stamp };
        tx.execute(
            "INSERT INTO cursors VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(device,path) DO UPDATE SET \
             agent=excluded.agent, stamp=excluded.stamp, checkpoint=excluded.checkpoint, \
             session=COALESCE(excluded.session,session), last_branch=excluded.last_branch, \
             subagent=excluded.subagent",
            params![
                device,
                path,
                agent,
                stamp,
                checkpoint,
                facts.session_id,
                last_branch,
                facts.subagent
            ],
        )
        .map_err(failed)?;
        if !facts.prs.is_empty() || !facts.issues.is_empty() {
            ledger::refresh_outputs(&tx)?;
        }
        tx.commit().map_err(failed)
    }

    /// The sessions Hide panes carry now, with where each works: a span on
    /// the pane's branch that grows while the pane stays (D-15).
    pub fn apply_panes(&mut self, panes: &[PaneFact], now: u64) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        for pane in panes {
            let agent = pane.agent.as_str();
            tx.execute(
                "INSERT INTO sessions(device,agent,id,cwd,interactive,started_at,ended_at) \
                 VALUES(?1,?2,?3,?4,1,?5,?5) ON CONFLICT(device,agent,id) DO UPDATE SET \
                 cwd=COALESCE(cwd,excluded.cwd), interactive=COALESCE(interactive,1), \
                 ended_at=MAX(COALESCE(ended_at,0),excluded.ended_at)",
                params![pane.device_id, agent, pane.session_id, pane.cwd, now as i64],
            )
            .map_err(failed)?;
            let Some(branch) = pane.branch.as_deref() else {
                continue;
            };
            let latest: Option<String> = tx
                .query_row(
                    "SELECT branch FROM session_branches WHERE device=?1 AND agent=?2 AND id=?3 \
                     AND file=?4 ORDER BY last_at DESC LIMIT 1",
                    params![pane.device_id, agent, pane.session_id, PANE_FILE],
                    |row| row.get(0),
                )
                .optional()
                .map_err(failed)?;
            if latest.as_deref() == Some(branch) {
                tx.execute(
                    "UPDATE session_branches SET last_at=MAX(last_at,?5) WHERE rowid=(SELECT rowid \
                     FROM session_branches WHERE device=?1 AND agent=?2 AND id=?3 AND file=?4 \
                     ORDER BY last_at DESC LIMIT 1)",
                    params![
                        pane.device_id,
                        agent,
                        pane.session_id,
                        PANE_FILE,
                        now as i64
                    ],
                )
                .map_err(failed)?;
            } else {
                tx.execute(
                    "INSERT OR IGNORE INTO session_branches(device,agent,id,file,branch,first_at,last_at) \
                     VALUES(?1,?2,?3,?4,?5,?6,?6)",
                    params![
                        pane.device_id,
                        agent,
                        pane.session_id,
                        PANE_FILE,
                        branch,
                        now as i64
                    ],
                )
                .map_err(failed)?;
            }
        }
        tx.commit().map_err(failed)
    }

    /// Which session `hide agent spawn` started for which (D-27), and the
    /// name the record gives each child: a fact can name a session by the
    /// file Herdr reported for it, the record by its reader's name.
    pub fn apply_parents(
        &mut self,
        parents: &[ParentFact],
    ) -> Result<Vec<SessionIdentity>, String> {
        let tx = self.connection.transaction().map_err(failed)?;
        let mut children = Vec::with_capacity(parents.len());
        for parent in parents {
            let child = SessionIdentity {
                device_id: parent.device_id.clone(),
                agent: parent.agent.clone(),
                session_id: session_name(
                    &tx,
                    &parent.device_id,
                    &parent.agent,
                    &parent.session_id,
                )?,
            };
            let named = Parent {
                device: parent.parent_device_id.clone(),
                agent: parent.parent_agent.clone(),
                id: session_name(
                    &tx,
                    &parent.parent_device_id,
                    &parent.parent_agent,
                    &parent.parent_session_id,
                )?,
                name: parent.parent_name.clone(),
            };
            record_parent(&tx, &child, &named)?;
            children.push(child);
        }
        tx.commit().map_err(failed)?;
        Ok(children)
    }

    /// Removes the sessions their project's Copied history no longer keeps
    /// (D-18, D-25, B33). `days` answers a project key's policy (0 is Off,
    /// None the default 90); `file_exists` says whether a local file is
    /// still there. Returns the sessions that left, which the session
    /// archive drops too (PRD session-archive B31).
    pub fn prune(
        &mut self,
        days: &dyn Fn(&str) -> Option<u16>,
        local_device: &str,
        files_present: &mut FilesPresent<'_>,
        now: u64,
    ) -> Result<Vec<SessionIdentity>, String> {
        let projects = self.projects()?;
        let mut owner: HashMap<SessionKey, usize> = HashMap::new();
        let sessions = self.all_sessions()?;
        for (index, project) in projects.iter().enumerate() {
            let paths = self.project_paths(project)?;
            for session in &sessions {
                if session.inside(&paths) {
                    owner.entry(session.key.clone()).or_insert(index);
                }
            }
        }
        // Per project, the latest end among the pull requests each session
        // is on; one still open counts as now.
        let mut latest: HashMap<SessionKey, u64> = HashMap::new();
        for project in &projects {
            for pr in self.candidate_prs(&project.key)? {
                let end = pr.end().unwrap_or(now);
                for line in self.pr_lines(project, &pr, None)? {
                    for id in &line.ids {
                        let key = SessionKey {
                            device: line.device_id.clone(),
                            agent: line.agent.clone(),
                            id: id.clone(),
                        };
                        let held = latest.entry(key).or_insert(end);
                        *held = (*held).max(end);
                    }
                }
            }
        }
        let policy = |session: &SessionRow| {
            owner
                .get(&session.key)
                .and_then(|index| days(&projects[*index].key))
                .unwrap_or(90)
        };
        // An OpenCode session lives in OpenCode's database, not a file: its
        // read marks it gone when the database drops it. This machine's other
        // files are asked about once, together.
        fn checked<'a>(session: &'a SessionRow, local_device: &str) -> Option<&'a str> {
            session.path.as_deref().filter(|path| {
                session.key.device == local_device
                    && !path.starts_with(hide_session::links::OPENCODE_PREFIX)
            })
        }
        let asked = sessions
            .iter()
            .filter(|session| policy(session) == 0)
            .filter_map(|session| checked(session, local_device).map(str::to_owned))
            .collect::<Vec<_>>();
        let present = if asked.is_empty() {
            BTreeSet::new()
        } else {
            files_present(&asked)?
        };
        let mut gone = Vec::new();
        for session in &sessions {
            let policy = policy(session);
            let ended = session.ended_at.unwrap_or(0);
            let keep = if policy == 0 {
                match (checked(session, local_device), session.path.as_deref()) {
                    (Some(path), _) => present.contains(path),
                    (None, Some(_)) => !session.file_gone,
                    (None, None) => true,
                }
            } else {
                let cutoff = now.saturating_sub(u64::from(policy) * DAY_MS);
                ended >= cutoff
                    || (now.saturating_sub(ended) < OPEN_EXCEPTION_MS
                        && latest.get(&session.key).is_some_and(|end| *end >= cutoff))
            };
            if !keep {
                gone.push(session.key.clone());
            }
        }
        self.remove_sessions(&gone)?;
        Ok(gone.into_iter().map(SessionKey::identity).collect())
    }

    /// Removes Hide's record lines of the named sessions, and nothing of the
    /// agents' own conversation files (PRD session-archive D-15, B29).
    pub fn delete_sessions(&mut self, sessions: &[SessionIdentity]) -> Result<(), String> {
        let keys = sessions
            .iter()
            .map(|session| SessionKey {
                device: session.device_id.clone(),
                agent: session.agent.clone(),
                id: session.session_id.clone(),
            })
            .collect::<Vec<_>>();
        self.remove_sessions(&keys)
    }

    /// The archived sessions the record holds no line for whose project's
    /// Copied history has run out since they were archived (D-14, B31): Off
    /// keeps them, an unknown project takes the default 90 days.
    pub fn expired_archived(
        &self,
        archived: &[ArchivedSession],
        days: &dyn Fn(&str) -> Option<u16>,
        now: u64,
    ) -> Result<Vec<SessionIdentity>, String> {
        if archived.is_empty() {
            return Ok(Vec::new());
        }
        let projects = self.projects()?;
        let mut expired = Vec::new();
        for held in archived {
            let key = SessionKey {
                device: held.session.device_id.clone(),
                agent: held.session.agent.clone(),
                id: held.session.session_id.clone(),
            };
            if self.session(&key)?.is_some() {
                continue;
            }
            let policy = projects
                .iter()
                .find(|project| {
                    project.device == held.session.device_id && project.root == held.project_root
                })
                .and_then(|project| days(&project.key))
                .unwrap_or(90);
            if policy != 0
                && held.archived_at_unix_ms < now.saturating_sub(u64::from(policy) * DAY_MS)
            {
                expired.push(held.session.clone());
            }
        }
        Ok(expired)
    }

    fn remove_sessions(&mut self, gone: &[SessionKey]) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        for key in gone {
            for table in [
                "sessions",
                "session_branches",
                "session_prs",
                "session_parents",
                "ledger_requests",
                "ledger_excluded",
                "ledger_unjudged",
                "ledger_sightings",
                "ledger_titles",
                "ledger_chapters",
                "ledger_missed",
                "ledger_turns",
                "ledger_outputs",
                "ledger_issue_attempts",
                "ledger_session_origins",
                "ledger_plans",
            ] {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE device=?1 AND agent=?2 AND id=?3"),
                    params![key.device, key.agent, key.id],
                )
                .map_err(failed)?;
            }
        }
        tx.execute("DELETE FROM ledger_output_facts AS f WHERE NOT EXISTS(SELECT 1 FROM ledger_outputs o WHERE o.repo=f.repo AND o.kind=f.kind AND o.number=f.number) AND NOT EXISTS(SELECT 1 FROM ledger_sightings s WHERE s.repo=f.repo AND s.kind=f.kind AND s.number=f.number) AND NOT EXISTS(SELECT 1 FROM session_prs s WHERE f.kind='pr' AND s.repo_name=f.repo AND s.number=f.number)", []).map_err(failed)?;
        tx.commit().map_err(failed)?;
        if !gone.is_empty() {
            // `secure_delete` clears the main file's pages; the pruned text
            // also leaves the write-ahead log only once it is checkpointed.
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .map_err(failed)?;
        }
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<ProjectRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT key, device, root, repo, name FROM projects ORDER BY key")
            .map_err(failed)?;
        statement
            .query_map([], |row| {
                Ok(ProjectRow {
                    key: row.get(0)?,
                    device: row.get(1)?,
                    root: row.get(2)?,
                    repo: row.get(3)?,
                    name: row.get(4)?,
                })
            })
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    pub fn project(&self, key: &str) -> Result<Option<ProjectRow>, String> {
        Ok(self
            .projects()?
            .into_iter()
            .find(|project| project.key == key))
    }

    fn all_sessions(&self) -> Result<Vec<SessionRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached(&format!("SELECT {SESSION_COLUMNS} FROM sessions"))
            .map_err(failed)?;
        statement
            .query_map([], session_row)
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    fn session(&self, key: &SessionKey) -> Result<Option<SessionRow>, String> {
        self.connection
            .query_row(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM sessions WHERE device=?1 AND agent=?2 AND id=?3"
                ),
                params![key.device, key.agent, key.id],
                session_row,
            )
            .optional()
            .map_err(failed)
    }

    /// The folders a project's sessions work in, each on its device: its
    /// root and the worktrees recorded for it, and where a session that
    /// printed one of its pull requests worked, on any device. The last keeps
    /// a removed worktree's sessions in the project after its checkout is
    /// gone, and joins a device's sessions to the project its repository is
    /// registered as here (D-21). A learned folder that holds a registered
    /// project's place (a home folder, `/`) or lies in another project's
    /// would widen this project over sessions that are not its own, so it is
    /// left out (D-23).
    fn project_paths(&self, project: &ProjectRow) -> Result<Vec<DevicePath>, String> {
        let mut paths = self.registered_paths(project)?;
        if let Some(repo) = project.repo.as_deref() {
            let mut statement = self
                .connection
                .prepare_cached(
                    "SELECT DISTINCT s.device, s.cwd FROM session_prs sp JOIN sessions s \
                     ON s.device=sp.device AND s.agent=sp.agent AND s.id=sp.id \
                     LEFT JOIN repo_names r ON r.name=sp.repo_name \
                     WHERE COALESCE(r.repo, sp.repo_name)=?1 AND s.cwd IS NOT NULL",
                )
                .map_err(failed)?;
            let learned: Vec<DevicePath> = statement
                .query_map([repo], |row| Ok((row.get(0)?, row.get(1)?)))
                .and_then(Iterator::collect)
                .map_err(failed)?;
            if !learned.is_empty() {
                let fences = self.fences(project)?;
                paths.extend(learned.into_iter().filter(|place| fences.admit(place)));
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn fences(&self, project: &ProjectRow) -> Result<Fences, String> {
        let mut fences = Fences {
            every: self.registered_paths(project)?,
            others: Vec::new(),
        };
        for other in self.projects()? {
            if other.key != project.key {
                let registered = self.registered_paths(&other)?;
                fences.every.extend(registered.iter().cloned());
                fences.others.extend(registered);
            }
        }
        Ok(fences)
    }

    /// A project's own places: its root and the worktrees recorded for it.
    fn registered_paths(&self, project: &ProjectRow) -> Result<Vec<DevicePath>, String> {
        let mut paths = vec![(project.device.clone(), project.root.clone())];
        let mut statement = self
            .connection
            .prepare_cached("SELECT DISTINCT path FROM worktrees WHERE project=?1")
            .map_err(failed)?;
        let worktrees: Vec<String> = statement
            .query_map([&project.key], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(failed)?;
        paths.extend(
            worktrees
                .into_iter()
                .map(|path| (project.device.clone(), path)),
        );
        Ok(paths)
    }

    pub fn pr(&self, project: &str, number: u64) -> Result<Option<PrRow>, String> {
        self.connection
            .query_row(
                &format!(
                    "SELECT {PR_COLUMNS} FROM prs WHERE project=?1 AND number=?2 \
                     ORDER BY seen_at DESC LIMIT 1"
                ),
                params![project, number as i64],
                pr_row,
            )
            .optional()
            .map_err(failed)
    }

    /// The pull requests of a project whose head is `branch`, newest first.
    pub fn prs_on_branch(&self, project: &str, branch: &str) -> Result<Vec<PrRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached(&format!(
                "SELECT {PR_COLUMNS} FROM prs WHERE project=?1 AND branch=?2 \
                 ORDER BY created_at DESC"
            ))
            .map_err(failed)?;
        statement
            .query_map(params![project, branch], pr_row)
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    /// The issues a pull request is linked to now, by any source (B4).
    pub fn pr_issues(&self, pr: &PrRow) -> Result<Vec<LinkedIssue>, String> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT issue, source FROM pr_issues WHERE repo=?1 AND number=?2 \
                 AND end_at IS NULL ORDER BY source, issue",
            )
            .map_err(failed)?;
        let rows: Vec<(String, String)> = statement
            .query_map(params![pr.repo, pr.number as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .and_then(Iterator::collect)
            .map_err(failed)?;
        Ok(rows
            .into_iter()
            .filter_map(|(key, source)| {
                IssueSource::parse(&source).map(|source| LinkedIssue { key, source })
            })
            .collect())
    }

    /// The worktrees recorded for a pull request's branch, newest first.
    pub fn pr_worktrees(&self, project: &str, branch: &str) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT path FROM worktrees WHERE project=?1 AND branch=?2 ORDER BY last_seen DESC",
            )
            .map_err(failed)?;
        statement
            .query_map([project, branch], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    /// The pull requests an issue is linked to now in a project.
    pub fn issue_prs(&self, project: &str, issue: &str) -> Result<Vec<PrRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached(&format!(
                "SELECT {PR_COLUMNS_P} FROM prs p JOIN pr_issues i ON i.repo=p.repo \
                 AND i.number=p.number WHERE p.project=?1 AND i.issue=?2 AND i.end_at IS NULL \
                 GROUP BY p.repo, p.number ORDER BY p.number DESC"
            ))
            .map_err(failed)?;
        statement
            .query_map([project, issue], pr_row)
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    /// The session lines of one pull request, newest first: a session that
    /// printed its address, and one that worked on its branch in a folder of
    /// the project while it lived (D-15, D-44, B30). A run that is not
    /// interactive makes no line (D-43). `local_device` asks for each file's
    /// presence on this machine; `None` leaves it unknown.
    pub fn pr_lines(
        &self,
        project: &ProjectRow,
        pr: &PrRow,
        local: Option<LocalFiles<'_>>,
    ) -> Result<Vec<LinkedSession>, String> {
        let names = self.repo_names(&pr.repo)?;
        let mut printed: BTreeMap<SessionKey, (u64, Option<String>)> = BTreeMap::new();
        {
            let mut statement = self
                .connection
                .prepare_cached(
                    "SELECT device, agent, id, at, request FROM session_prs \
                     WHERE repo_name=?1 AND number=?2",
                )
                .map_err(failed)?;
            for name in &names {
                let rows: Vec<(SessionKey, u64, Option<String>)> = statement
                    .query_map(params![name, pr.number as i64], |row| {
                        Ok((
                            SessionKey {
                                device: row.get(0)?,
                                agent: row.get(1)?,
                                id: row.get(2)?,
                            },
                            row.get::<_, i64>(3)? as u64,
                            row.get(4)?,
                        ))
                    })
                    .and_then(Iterator::collect)
                    .map_err(failed)?;
                for (key, at, request) in rows {
                    let entry = printed.entry(key).or_insert((at, request.clone()));
                    if at < entry.0 {
                        *entry = (at, request);
                    }
                }
            }
        }
        let start = self.window_start(pr)?;
        let end = pr.end().unwrap_or(u64::MAX);
        let mut paths = self.project_paths(project)?;
        if !printed.is_empty() {
            let fences = self.fences(project)?;
            for key in printed.keys() {
                if let Some(cwd) = self.session(key)?.and_then(|session| session.cwd) {
                    let place = (key.device.clone(), cwd);
                    if fences.admit(&place) {
                        paths.push(place);
                    }
                }
            }
        }
        let mut worked: BTreeMap<SessionKey, BranchWork> = BTreeMap::new();
        {
            let mut statement = self
                .connection
                .prepare_cached(
                    "SELECT device, agent, id, first_at, last_at, last_request, last_request_at \
                     FROM session_branches WHERE branch=?1 AND last_at>=?2 AND first_at<=?3",
                )
                .map_err(failed)?;
            let rows: Vec<SpanRow> = statement
                .query_map(
                    params![pr.branch, start as i64, end.min(i64::MAX as u64) as i64],
                    |row| {
                        Ok((
                            SessionKey {
                                device: row.get(0)?,
                                agent: row.get(1)?,
                                id: row.get(2)?,
                            },
                            row.get::<_, i64>(3)? as u64,
                            row.get::<_, i64>(4)? as u64,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .and_then(Iterator::collect)
                .map_err(failed)?;
            for (key, first, last, request, request_at) in rows {
                let entry = worked.entry(key).or_insert(BranchWork {
                    first,
                    last,
                    request: None,
                });
                entry.first = entry.first.min(first);
                entry.last = entry.last.max(last);
                if let (Some(request), Some(at)) = (request, request_at) {
                    let at = at as u64;
                    if at <= end && entry.request.as_ref().is_none_or(|(held, _)| at > *held) {
                        entry.request = Some((at, request));
                    }
                }
            }
        }
        let mut members: Vec<Member> = Vec::new();
        let keys = printed
            .keys()
            .chain(worked.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in keys {
            let Some(session) = self.session(&key)? else {
                continue;
            };
            if session.interactive == Some(false) {
                continue;
            }
            let created = pr.created_at.is_some_and(|created| {
                printed.get(&key).is_some_and(|(at, _)| {
                    *at + CREATED_BEFORE_MS >= created && *at <= created + CREATED_AFTER_MS
                })
            });
            if !printed.contains_key(&key) && !session.inside(&paths) {
                continue;
            }
            let (role, request) = if created {
                (
                    SessionRole::Created,
                    printed.get(&key).and_then(|(_, request)| request.clone()),
                )
            } else {
                let branch_request = worked
                    .get(&key)
                    .and_then(|work| work.request.clone().map(|(_, text)| text));
                let printed_request = printed.get(&key).and_then(|(_, request)| request.clone());
                (SessionRole::Worked, branch_request.or(printed_request))
            };
            members.push((session, role, request, worked.contains_key(&key)));
        }
        self.lines(members, pr.number, local)
    }

    /// Joins sessions that continue one another into one line each (B17),
    /// newest first, with their parent and file state.
    fn lines(
        &self,
        members: Vec<Member>,
        pr: u64,
        local: Option<LocalFiles<'_>>,
    ) -> Result<Vec<LinkedSession>, String> {
        let mut groups: BTreeMap<SessionKey, Vec<Member>> = BTreeMap::new();
        for member in members {
            let root = self.chain_root(&member.0)?;
            groups.entry(root).or_default().push(member);
        }
        let mut groups = groups.into_values().collect::<Vec<_>>();
        for group in &mut groups {
            group.sort_by_key(|(session, _, _, _)| session.started_at.unwrap_or(0));
        }
        // This machine's files are asked about once, together, through its
        // node; an OpenCode session lives in OpenCode's database, not a file.
        let held = |session: &SessionRow| -> Option<String> {
            let local = local?;
            session.path.clone().filter(|path| {
                session.key.device == local.device
                    && !session.file_gone
                    && !path.starts_with(hide_session::links::OPENCODE_PREFIX)
            })
        };
        let asked = groups
            .iter()
            .filter_map(|group| held(&group[group.len() - 1].0))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let present = match local {
            Some(local) if !asked.is_empty() => Some((local.present)(&asked)),
            _ => None,
        };
        let mut lines = Vec::new();
        for group in groups {
            let last = &group[group.len() - 1].0;
            let role = if group
                .iter()
                .any(|(_, role, _, _)| *role == SessionRole::Created)
            {
                SessionRole::Created
            } else {
                SessionRole::Worked
            };
            let request = group
                .iter()
                .rev()
                .find(|(_, member_role, request, _)| *member_role == role && request.is_some())
                .and_then(|(_, _, request, _)| request.clone());
            let file = match (last.path.as_deref(), local) {
                (None, _) => FileState::Unknown,
                (Some(_), _) if last.file_gone => FileState::Missing,
                (Some(path), Some(local)) if last.key.device == local.device => {
                    match (held(last), &present) {
                        (None, _) => FileState::Present,
                        (Some(_), Some(Ok(present))) if present.contains(path) => {
                            FileState::Present
                        }
                        (Some(_), Some(Ok(_))) => FileState::Missing,
                        (Some(_), _) => FileState::Unknown,
                    }
                }
                (Some(_), _) => FileState::Unknown,
            };
            let parent = self.parent(&last.key)?;
            lines.push(LinkedSession {
                agent: last.key.agent.clone(),
                id: last.key.id.clone(),
                ids: group
                    .iter()
                    .map(|(session, _, _, _)| session.key.id.clone())
                    .collect(),
                device_id: last.key.device.clone(),
                role,
                pr,
                request,
                started_at_unix_ms: group.iter().filter_map(|(s, _, _, _)| s.started_at).min(),
                ended_at_unix_ms: group.iter().filter_map(|(s, _, _, _)| s.ended_at).max(),
                path: last
                    .path
                    .clone()
                    .filter(|path| !path.starts_with("opencode/")),
                cwd: last.cwd.clone(),
                file,
                parent,
                on_branch: group.iter().any(|(_, _, _, on_branch)| *on_branch),
            });
        }
        lines.sort_by(|left, right| {
            right
                .ended_at_unix_ms
                .cmp(&left.ended_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(lines)
    }

    fn chain_root(&self, session: &SessionRow) -> Result<SessionKey, String> {
        let mut key = session.key.clone();
        let mut next = session.continues.clone();
        for _ in 0..20 {
            let Some(id) = next else { break };
            let previous = SessionKey {
                device: key.device.clone(),
                agent: key.agent.clone(),
                id,
            };
            if previous == session.key {
                break;
            }
            next = self.session(&previous)?.and_then(|row| row.continues);
            key = previous;
        }
        Ok(key)
    }

    fn parent(&self, key: &SessionKey) -> Result<Option<LinkedParent>, String> {
        let row: Option<(String, String, String, String)> = self
            .connection
            .query_row(
                "SELECT parent_agent, parent_id, parent_name, parent_device FROM session_parents \
                 WHERE device=?1 AND agent=?2 AND id=?3",
                params![key.device, key.agent, key.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(failed)?;
        let Some((agent, session_id, name, device)) = row else {
            return Ok(None);
        };
        let available = self
            .session(&SessionKey {
                device,
                agent: agent.clone(),
                id: session_id.clone(),
            })?
            .is_some();
        Ok(Some(LinkedParent {
            name,
            agent,
            session_id,
            available,
        }))
    }

    /// When a pull request's life began: when the previous one on its
    /// branch closed, or from the start (D-44).
    fn window_start(&self, pr: &PrRow) -> Result<u64, String> {
        let Some(created) = pr.created_at else {
            return Ok(0);
        };
        let previous: Option<i64> = self
            .connection
            .query_row(
                "SELECT MAX(COALESCE(merged_at, closed_at)) FROM prs WHERE repo=?1 AND branch=?2 \
                 AND number<>?3 AND created_at<?4",
                params![pr.repo, pr.branch, pr.number as i64, created as i64],
                |row| row.get(0),
            )
            .map_err(failed)?;
        Ok(previous.map_or(0, |at| at as u64))
    }

    fn repo_names(&self, repo: &str) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT name FROM repo_names WHERE repo=?1")
            .map_err(failed)?;
        let mut names: Vec<String> = statement
            .query_map([repo], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(failed)?;
        if !repo.starts_with("id:") && !names.iter().any(|name| name == repo) {
            names.push(repo.to_owned());
        }
        Ok(names)
    }

    /// The pull request panel's sessions, capped at [`PANEL_SESSION_LIMIT`],
    /// with the full count (B6).
    pub fn pr_panel(
        &self,
        project: &str,
        number: u64,
        local: Option<LocalFiles<'_>>,
    ) -> Result<Option<PrLinks>, String> {
        let Some(project) = self.project(project)? else {
            return Ok(None);
        };
        let Some(pr) = self.pr(&project.key, number)? else {
            return Ok(None);
        };
        let mut sessions = self.pr_lines(&project, &pr, local)?;
        let total = sessions.len();
        sessions.truncate(PANEL_SESSION_LIMIT);
        Ok(Some(PrLinks {
            issues: self.pr_issues(&pr)?,
            worktrees: self.pr_worktrees(&project.key, &pr.branch)?,
            pr,
            sessions,
            total,
        }))
    }

    /// The issue panel's sessions: the lines of every pull request the issue
    /// is linked to now, one per session, a created role winning (B34).
    pub fn issue_panel(
        &self,
        project: &str,
        issue: &str,
        local: Option<LocalFiles<'_>>,
    ) -> Result<IssueLinks, String> {
        let Some(project) = self.project(project)? else {
            return Ok(IssueLinks::default());
        };
        let prs = self.issue_prs(&project.key, &super::issue_key(issue))?;
        let mut sessions: Vec<LinkedSession> = Vec::new();
        for pr in &prs {
            for line in self.pr_lines(&project, pr, local)? {
                match sessions.iter_mut().find(|held| {
                    held.device_id == line.device_id
                        && held.agent == line.agent
                        && held.id == line.id
                }) {
                    Some(held) => {
                        if held.role == SessionRole::Worked && line.role == SessionRole::Created {
                            *held = line;
                        }
                    }
                    None => sessions.push(line),
                }
            }
        }
        sessions.sort_by(|left, right| {
            right
                .ended_at_unix_ms
                .cmp(&left.ended_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        let total = sessions.len();
        sessions.truncate(PANEL_SESSION_LIMIT);
        Ok(IssueLinks {
            prs: prs.iter().map(|pr| pr.number).collect(),
            sessions,
            total,
        })
    }

    /// The pull requests a session made or worked on in a project, newest
    /// first, for its chips and `hide links session`.
    pub fn session_prs(
        &self,
        project: &ProjectRow,
        session_id: &str,
    ) -> Result<Vec<(PrRow, LinkedSession)>, String> {
        let mut found = Vec::new();
        for pr in self.candidate_prs(&project.key)? {
            if let Some(line) = self
                .pr_lines(project, &pr, None)?
                .into_iter()
                .find(|line| line.ids.iter().any(|id| id == session_id))
            {
                found.push((pr, line));
            }
        }
        found.sort_by_key(|(pr, _)| std::cmp::Reverse(pr.number));
        Ok(found)
    }

    /// The pull requests of a project that any session could be linked to:
    /// printed by one, or on a branch one worked on.
    fn candidate_prs(&self, project: &str) -> Result<Vec<PrRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached(&format!(
                "SELECT {PR_COLUMNS_P} FROM prs p WHERE p.project=?1 AND (\
                 EXISTS(SELECT 1 FROM session_branches b WHERE b.branch=p.branch) OR \
                 EXISTS(SELECT 1 FROM session_prs s LEFT JOIN repo_names r ON r.name=s.repo_name \
                 WHERE s.number=p.number AND COALESCE(r.repo, s.repo_name)=p.repo)) \
                 ORDER BY p.number"
            ))
            .map_err(failed)?;
        statement
            .query_map([project], pr_row)
            .and_then(Iterator::collect)
            .map_err(failed)
    }

    /// The sessions that worked on a branch in a project at any time, and
    /// the pull requests made from it, for `hide links branch`.
    pub fn branch_links(
        &self,
        project: &str,
        branch: &str,
        local: Option<LocalFiles<'_>>,
    ) -> Result<Option<BranchLinks>, String> {
        let Some(row) = self.project(project)? else {
            return Ok(None);
        };
        let prs = self.prs_on_branch(project, branch)?;
        // The branch's whole life: no window, no address.
        let whole = PrRow {
            repo: row.repo.clone().unwrap_or_default(),
            number: 0,
            branch: branch.to_owned(),
            title: String::new(),
            url: String::new(),
            created_at: None,
            closed_at: None,
            merged_at: None,
        };
        let sessions = self.pr_lines(&row, &whole, local)?;
        if prs.is_empty() && sessions.is_empty() {
            return Ok(None);
        }
        Ok(Some((prs, sessions)))
    }

    /// One session's own line in a project and the pull requests it made or
    /// worked on, for `hide links session`; none when the session never
    /// worked in the project.
    pub fn session_links(
        &self,
        project: &str,
        session_id: &str,
        local: Option<LocalFiles<'_>>,
    ) -> Result<Option<SessionLinks>, String> {
        let Some(row) = self.project(project)? else {
            return Ok(None);
        };
        let prs = self.session_prs(&row, session_id)?;
        let paths = self.project_paths(&row)?;
        let mut statement = self
            .connection
            .prepare_cached(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"
            ))
            .map_err(failed)?;
        let found: Vec<SessionRow> = statement
            .query_map(params![session_id], session_row)
            .and_then(Iterator::collect)
            .map_err(failed)?;
        let Some(session) = found.into_iter().find(|session| {
            session.interactive != Some(false) && (!prs.is_empty() || session.inside(&paths))
        }) else {
            return Ok(None);
        };
        let request = self.session_request(&session.key)?;
        let role = if prs
            .iter()
            .any(|(_, line)| line.role == SessionRole::Created)
        {
            SessionRole::Created
        } else {
            SessionRole::Worked
        };
        let on_branch = prs.iter().any(|(_, line)| line.on_branch);
        let line = self
            .lines(vec![(session, role, request, on_branch)], 0, local)?
            .remove(0);
        Ok(Some((line, prs)))
    }

    fn session_request(&self, key: &SessionKey) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT last_request FROM sessions WHERE device=?1 AND agent=?2 AND id=?3",
                params![key.device, key.agent, key.id],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(failed)
    }

    /// A project's counts and chips (D-45), and which of its `checkouts`
    /// (the ones it has now) hold work that landed.
    pub fn summary(
        &self,
        project: &str,
        checkouts: &[WorktreeFact],
    ) -> Result<ProjectLinkSummary, String> {
        let mut summary = ProjectLinkSummary::default();
        let Some(row) = self.project(project)? else {
            return Ok(summary);
        };
        let mut by_pr: BTreeMap<u64, Vec<(String, String, String)>> = BTreeMap::new();
        let mut chips: BTreeMap<String, BTreeSet<SessionPrChip>> = BTreeMap::new();
        // Per checkout: whether a pull request its sessions worked on merged,
        // and whether one is still open.
        let mut work: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
        for pr in self.candidate_prs(project)? {
            let lines = self.pr_lines(&row, &pr, None)?;
            if lines.is_empty() {
                continue;
            }
            summary.prs.insert(pr.number, lines.len() as u32);
            for line in &lines {
                for id in &line.ids {
                    chips.entry(id.clone()).or_default().insert(SessionPrChip {
                        number: pr.number,
                        created: line.role == SessionRole::Created,
                    });
                }
                // Only work counts: a session that made the pull request or
                // worked on its branch, not one that printed its address.
                if line.role != SessionRole::Created && !line.on_branch {
                    continue;
                }
                summary
                    .pr_workers
                    .entry(pr.number)
                    .or_default()
                    .extend(line.ids.iter().cloned());
                if let Some(checkout) = line
                    .cwd
                    .as_deref()
                    .filter(|_| line.device_id == row.device)
                    .and_then(|cwd| deepest_checkout(checkouts, cwd))
                {
                    summary
                        .pr_checkouts
                        .entry(pr.number)
                        .or_default()
                        .insert(checkout.path.clone());
                }
                // A checkout's sessions are the ones that started in it since
                // it was added, so a worktree made again at a used path
                // inherits nothing, and one whose age is not read yet has
                // none. On a branch it weighs only that branch's pull
                // requests, so switching to a new branch starts clean.
                let checkout = line
                    .cwd
                    .as_deref()
                    .filter(|_| line.device_id == row.device)
                    .and_then(|cwd| deepest_checkout(checkouts, cwd))
                    .filter(|checkout| {
                        checkout.created_at_unix_ms.is_some_and(|added| {
                            line.started_at_unix_ms
                                .is_some_and(|started| started >= added)
                        })
                    })
                    .filter(|checkout| checkout.branch.as_ref().is_none_or(|b| *b == pr.branch));
                if let Some(checkout) = checkout {
                    let (merged, open) = work.entry(checkout.path.as_str()).or_default();
                    *merged |= pr.merged_at.is_some();
                    *open |= pr.end().is_none();
                }
            }
            by_pr.insert(
                pr.number,
                lines
                    .into_iter()
                    .map(|line| (line.device_id, line.agent, line.id))
                    .collect(),
            );
        }
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT DISTINCT i.issue, p.number FROM pr_issues i JOIN prs p ON p.repo=i.repo \
                 AND p.number=i.number WHERE p.project=?1 AND i.end_at IS NULL",
            )
            .map_err(failed)?;
        let links: Vec<(String, i64)> = statement
            .query_map([project], |row| Ok((row.get(0)?, row.get(1)?)))
            .and_then(Iterator::collect)
            .map_err(failed)?;
        let mut issues: BTreeMap<String, BTreeSet<(String, String, String)>> = BTreeMap::new();
        for (issue, number) in links {
            if let Some(lines) = by_pr.get(&(number as u64)) {
                issues
                    .entry(issue)
                    .or_default()
                    .extend(lines.iter().cloned());
            }
        }
        summary.issues = issues
            .into_iter()
            .map(|(issue, lines)| (issue, lines.len() as u32))
            .collect();
        summary.sessions = chips
            .into_iter()
            .take(SUMMARY_SESSION_LIMIT)
            .map(|(id, chips)| {
                let mut chips: Vec<_> = chips.into_iter().collect();
                // A pull request a session made shows once, as made.
                chips.sort_by(|left, right| {
                    right
                        .number
                        .cmp(&left.number)
                        .then(right.created.cmp(&left.created))
                });
                chips.dedup_by_key(|chip| chip.number);
                (id, chips)
            })
            .collect();
        summary.landed = work
            .into_iter()
            .filter(|(_, (merged, open))| *merged && !*open)
            .map(|(checkout, _)| checkout.to_owned())
            .collect();
        Ok(summary)
    }
}

/// The checkout a session in `cwd` worked in: the deepest one holding it, so
/// a worktree inside another checkout keeps its own sessions.
fn deepest_checkout<'a>(checkouts: &'a [WorktreeFact], cwd: &str) -> Option<&'a WorktreeFact> {
    checkouts
        .iter()
        .filter(|checkout| within(cwd, &checkout.path))
        .max_by_key(|checkout| checkout.path.trim_end_matches('/').len())
}

/// The project ids a node conversion or a core move gave new ids, so the
/// old session search file's `policy`, which is never written (D-20), still
/// names each project's Copied history days. Version 4.
const RETENTION_ALIAS_SCHEMA: &str = "
CREATE TABLE retention_aliases(old TEXT PRIMARY KEY, new TEXT NOT NULL);
";

fn migrate(connection: &Connection) -> Result<(), OpenFailure> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(classify)?;
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Err(OpenFailure::Newer);
    }
    if version < SCHEMA_VERSION {
        let initial = if version == 0 { SCHEMA } else { "" };
        let work_ledger = if version < 2 { ledger::SCHEMA } else { "" };
        let origins = if version < 3 {
            ledger::ORIGIN_SCHEMA
        } else {
            ""
        };
        let aliases = if version < 4 {
            RETENTION_ALIAS_SCHEMA
        } else {
            ""
        };
        let backlog = if version < 4 {
            ledger::BACKLOG_SCHEMA
        } else {
            ""
        };
        let observations = if version < 5 {
            super::observation::SCHEMA
        } else {
            ""
        };
        let metadata = if version < 6 {
            ledger::METADATA_SCHEMA
        } else {
            ""
        };
        let flows = if version < 7 { ledger::FLOW_SCHEMA } else { "" };
        let closed_flows = if version < 8 {
            ledger::CLOSED_FLOW_SCHEMA
        } else {
            ""
        };
        let plans = if version < 9 { ledger::PLAN_SCHEMA } else { "" };
        connection
            .execute_batch(&format!(
                "BEGIN IMMEDIATE; {initial} {work_ledger} {origins} {backlog} {aliases} {observations} {metadata} {flows} {closed_flows} {plans} PRAGMA user_version={SCHEMA_VERSION}; COMMIT;"
            ))
            .map_err(|error| {
                let _ = connection.execute_batch("ROLLBACK");
                classify(error)
            })?;
    }
    name_read_files(connection)
}

/// The record's key for a repository GitHub named: its id when GitHub gave
/// one, so a rename keeps every row (B28), else its name.
fn name_repo(tx: &Transaction<'_>, name: &str, id: Option<&str>) -> Result<String, String> {
    let name = name.to_ascii_lowercase();
    let Some(id) = id else {
        return repo_of_name(tx, &name);
    };
    let repo = format!("id:{id}");
    let previous = repo_of_name(tx, &name)?;
    tx.execute(
        "INSERT INTO repo_names VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET repo=excluded.repo",
        [&name, &repo],
    )
    .map_err(failed)?;
    if previous != repo {
        // Rows keyed by the name before its id was known move to the id.
        for table in ["prs", "pr_issues"] {
            tx.execute(
                &format!("UPDATE OR REPLACE {table} SET repo=?1 WHERE repo=?2"),
                [&repo, &previous],
            )
            .map_err(failed)?;
        }
        tx.execute(
            "UPDATE projects SET repo=?1 WHERE repo=?2",
            [&repo, &previous],
        )
        .map_err(failed)?;
    }
    Ok(repo)
}

fn repo_of_name(tx: &Transaction<'_>, name: &str) -> Result<String, String> {
    let name = name.to_ascii_lowercase();
    Ok(tx
        .query_row(
            "SELECT repo FROM repo_names WHERE name=?1",
            [&name],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?
        .unwrap_or(name))
}

fn issues_open(
    tx: &Transaction<'_>,
    repo: &str,
    number: u64,
    source: IssueSource,
) -> Result<Vec<String>, String> {
    let mut statement = tx
        .prepare_cached(
            "SELECT issue FROM pr_issues WHERE repo=?1 AND number=?2 AND source=?3 AND end_at IS NULL",
        )
        .map_err(failed)?;
    statement
        .query_map(params![repo, number as i64, source.as_str()], |row| {
            row.get(0)
        })
        .and_then(Iterator::collect)
        .map_err(failed)
}

/// The parent side of a lineage edge, named as the record names sessions.
struct Parent {
    device: String,
    agent: String,
    id: String,
    name: String,
}

/// What the record calls the session a fact names by `reference`.
///
/// The record calls a session what its reader calls it: the id in the
/// session's own records. Herdr reports a Pi or omp session as its file, so a
/// fact can name a session by a path; once the reader has read that file, its
/// cursor holds the reader's name for it. A reference no read file answers
/// is kept as it is: either it is already that name, or it is a file not read
/// yet (Pi writes its file at the first message), whose edges [`name_file`]
/// moves when the reader first names it.
fn session_name(
    tx: &Transaction<'_>,
    device: &str,
    agent: &str,
    reference: &str,
) -> Result<String, String> {
    Ok(tx
        .query_row(
            "SELECT session FROM cursors WHERE device=?1 AND path=?2 AND agent=?3 \
             AND session IS NOT NULL",
            params![device, reference, agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?
        .unwrap_or_else(|| reference.to_owned()))
}

/// Records `child` as started by `parent`. A parent learned after the
/// child's requests were recorded makes its first request the parent's and
/// retracts what was judged as the operator's.
fn record_parent(
    tx: &Transaction<'_>,
    child: &SessionIdentity,
    parent: &Parent,
) -> Result<(), String> {
    let changed = tx
        .query_row(
            "SELECT parent_device,parent_agent,parent_id,parent_name FROM session_parents \
             WHERE device=?1 AND agent=?2 AND id=?3",
            params![child.device_id, child.agent, child.session_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(failed)?
        .is_none_or(|(device, agent, id, name)| {
            device != parent.device
                || agent != parent.agent
                || id != parent.id
                || name != parent.name
        });
    tx.execute(
        "INSERT INTO session_parents VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(device,agent,id) \
         DO UPDATE SET parent_agent=excluded.parent_agent, parent_id=excluded.parent_id, \
         parent_name=excluded.parent_name,parent_device=excluded.parent_device",
        params![
            child.device_id,
            child.agent,
            child.session_id,
            parent.agent,
            parent.id,
            parent.name,
            parent.device,
        ],
    )
    .map_err(failed)?;
    tx.execute(
        "UPDATE ledger_requests SET author=?4 WHERE device=?1 AND agent=?2 AND id=?3 AND offset=(SELECT MIN(offset) FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3)",
        params![
            child.device_id,
            child.agent,
            child.session_id,
            serde_json::to_string(&crate::links::ledger::WorkAuthor::Parent)
                .map_err(|_| "ledger_value_invalid")?
        ],
    )
    .map_err(failed)?;
    ledger::retract_operator_work(tx, child)?;
    tx.execute(
        "DELETE FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 AND author=?4",
        params![
            child.device_id,
            child.agent,
            child.session_id,
            "\"operator\""
        ],
    )
    .map_err(failed)?;
    if changed {
        // Lineage arrived after the first catalog read: rescan only
        // this session through its existing bounded reader.
        tx.execute(
            "UPDATE cursors SET stamp='',checkpoint=NULL WHERE device=?1 AND agent=?2 AND session=?3",
            params![child.device_id, child.agent, child.session_id],
        )
        .map_err(failed)?;
        tx.execute(
            "DELETE FROM ledger_excluded WHERE device=?1 AND agent=?2 AND id=?3",
            params![child.device_id, child.agent, child.session_id],
        )
        .map_err(failed)?;
    }
    Ok(())
}

/// The reader named the file `path` as session `id`: an edge recorded under
/// the file before it was read moves to that name, as it would have been
/// recorded had the read come first.
fn name_file(
    tx: &Transaction<'_>,
    device: &str,
    agent: &str,
    path: &str,
    id: &str,
) -> Result<(), String> {
    tx.execute(
        "UPDATE OR REPLACE session_parents SET parent_id=?4 \
         WHERE parent_device=?1 AND parent_agent=?2 AND parent_id=?3",
        params![device, agent, path, id],
    )
    .map_err(failed)?;
    let parent = tx
        .query_row(
            "SELECT parent_device,parent_agent,parent_id,parent_name FROM session_parents \
             WHERE device=?1 AND agent=?2 AND id=?3",
            params![device, agent, path],
            |r| {
                Ok(Parent {
                    device: r.get(0)?,
                    agent: r.get(1)?,
                    id: r.get(2)?,
                    name: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(failed)?;
    let Some(parent) = parent else {
        return Ok(());
    };
    tx.execute(
        "DELETE FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3",
        params![device, agent, path],
    )
    .map_err(failed)?;
    record_parent(
        tx,
        &SessionIdentity {
            device_id: device.to_owned(),
            agent: agent.to_owned(),
            session_id: id.to_owned(),
        },
        &parent,
    )
}

/// The `meta` key of the edges' naming: `1` once every edge recorded under a
/// file the reader had read is under the reader's name. An older build reads
/// the same tables, so the store's schema version stays where it is.
const SESSION_NAMES: &str = "session_names";

/// Moves the edges an earlier build recorded under a Pi or omp session's file
/// to the reader's name for it, once.
fn name_read_files(connection: &Connection) -> Result<(), OpenFailure> {
    let done: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key=?1",
            [SESSION_NAMES],
            |row| row.get(0),
        )
        .optional()
        .map_err(classify)?;
    if done.as_deref() == Some("1") {
        return Ok(());
    }
    let tx = connection.unchecked_transaction().map_err(classify)?;
    let files = {
        let mut statement = tx
            .prepare(
                "SELECT c.device,c.agent,c.path,c.session FROM session_parents p JOIN cursors c \
                 ON c.device=p.device AND c.path=p.id AND c.agent=p.agent \
                 WHERE c.session IS NOT NULL \
                 UNION SELECT c.device,c.agent,c.path,c.session FROM session_parents p \
                 JOIN cursors c ON c.device=p.parent_device AND c.path=p.parent_id \
                 AND c.agent=p.parent_agent WHERE c.session IS NOT NULL",
            )
            .map_err(classify)?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(classify)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(classify)?
    };
    for (device, agent, path, id) in &files {
        name_file(&tx, device, agent, path, id).map_err(OpenFailure::Failed)?;
    }
    tx.execute(
        "INSERT INTO meta VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [SESSION_NAMES],
    )
    .map_err(classify)?;
    tx.commit().map_err(classify)?;
    if !files.is_empty() {
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "store.session_names_moved", "files": files.len(),
        }));
    }
    Ok(())
}

/// Where the last read of a file stopped, and what it knew of the file.
struct CursorRow {
    checkpoint: Option<String>,
    last_branch: Option<String>,
    session: Option<String>,
    subagent: bool,
}

fn upsert_session(
    tx: &Transaction<'_>,
    device: &str,
    agent: &str,
    id: &str,
    path: &str,
    facts: &hide_session::links::LinkFacts,
) -> Result<(), String> {
    // A subagent's file adds its activity to its parent's line and nothing
    // that names the parent's own file or request (D-28).
    let main = !facts.subagent;
    let (request_at, request) = facts
        .last_request
        .clone()
        .filter(|_| main)
        .map_or((None, None), |(at, text)| (Some(at as i64), Some(text)));
    tx.execute(
        "INSERT INTO sessions(device,agent,id,path,cwd,interactive,started_at,ended_at,\
         first_parent_uuid,last_uuid,forked_from,last_request,last_request_at,file_gone) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0) ON CONFLICT(device,agent,id) DO UPDATE SET \
         path=COALESCE(excluded.path,path), cwd=COALESCE(excluded.cwd,cwd), \
         interactive=COALESCE(excluded.interactive,interactive), \
         started_at=MIN(COALESCE(started_at,excluded.started_at),COALESCE(excluded.started_at,started_at)), \
         ended_at=MAX(COALESCE(ended_at,excluded.ended_at),COALESCE(excluded.ended_at,ended_at)), \
         first_parent_uuid=COALESCE(first_parent_uuid,excluded.first_parent_uuid), \
         last_uuid=COALESCE(excluded.last_uuid,last_uuid), \
         forked_from=COALESCE(excluded.forked_from,forked_from), \
         last_request=CASE WHEN excluded.last_request_at>=COALESCE(last_request_at,0) \
           THEN excluded.last_request ELSE last_request END, \
         last_request_at=MAX(COALESCE(last_request_at,0),COALESCE(excluded.last_request_at,0)), \
         file_gone=CASE WHEN excluded.path IS NULL THEN file_gone ELSE 0 END",
        params![
            device,
            agent,
            id,
            main.then_some(path),
            facts.cwd.as_deref().filter(|_| main),
            facts.interactive.filter(|_| main),
            facts.first_at_unix_ms.map(|at| at as i64),
            facts.last_at_unix_ms.map(|at| at as i64),
            facts.first_parent_uuid.as_deref().filter(|_| main),
            facts.last_uuid.as_deref().filter(|_| main),
            facts.forked_from.as_deref().filter(|_| main),
            request,
            request_at,
        ],
    )
    .map_err(failed)?;
    Ok(())
}

/// A session whose first record answers another's last continues it (B17).
fn link_continuations(tx: &Transaction<'_>, device: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE sessions SET continues=(SELECT p.id FROM sessions p WHERE p.device=sessions.device \
         AND p.agent=sessions.agent AND p.last_uuid=sessions.first_parent_uuid AND p.id<>sessions.id \
         LIMIT 1) WHERE device=?1 AND continues IS NULL AND first_parent_uuid IS NOT NULL",
        [device],
    )
    .map_err(failed)?;
    Ok(())
}

/// A session's work on a branch inside a pull request's life: its first
/// and last time there and its last request then.
struct BranchWork {
    first: u64,
    last: u64,
    request: Option<(u64, String)>,
}

/// One `session_branches` row: who, first, last, request, its time.
type SpanRow = (SessionKey, u64, u64, Option<String>, Option<i64>);

/// A branch's pull requests and the sessions that worked on it.
pub type BranchLinks = (Vec<PrRow>, Vec<LinkedSession>);

/// A session's own line and the pull requests it is on, each with its line.
pub type SessionLinks = (LinkedSession, Vec<(PrRow, LinkedSession)>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SessionKey {
    device: String,
    agent: String,
    id: String,
}

impl SessionKey {
    fn identity(self) -> SessionIdentity {
        SessionIdentity {
            device_id: self.device,
            agent: self.agent,
            session_id: self.id,
        }
    }
}

/// A session the archive holds, which retention answers for even when the
/// record has no line of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedSession {
    pub session: SessionIdentity,
    pub project_root: String,
    pub archived_at_unix_ms: u64,
}

/// A folder on a device.
type DevicePath = (String, String);

impl SessionRow {
    /// The session worked in one of `paths`, on that path's device.
    fn inside(&self, paths: &[DevicePath]) -> bool {
        self.cwd.as_deref().is_some_and(|cwd| {
            paths
                .iter()
                .any(|(device, path)| *device == self.key.device && within(cwd, path))
        })
    }
}

/// What keeps a learned place (a folder a session printed this project's
/// pull request from) to this project: a folder holding any project's
/// registered place, or lying inside another project's, is not this one's.
struct Fences {
    every: Vec<DevicePath>,
    others: Vec<DevicePath>,
}

impl Fences {
    fn admit(&self, (device, cwd): &DevicePath) -> bool {
        !self
            .every
            .iter()
            .any(|(at, path)| at == device && within(path, cwd))
            && !self
                .others
                .iter()
                .any(|(at, path)| at == device && within(cwd, path))
    }
}

struct SessionRow {
    key: SessionKey,
    path: Option<String>,
    cwd: Option<String>,
    interactive: Option<bool>,
    started_at: Option<u64>,
    ended_at: Option<u64>,
    continues: Option<String>,
    file_gone: bool,
}

const SESSION_COLUMNS: &str =
    "device, agent, id, path, cwd, interactive, started_at, ended_at, continues, file_gone";

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        key: SessionKey {
            device: row.get(0)?,
            agent: row.get(1)?,
            id: row.get(2)?,
        },
        path: row.get(3)?,
        cwd: row.get(4)?,
        interactive: row.get::<_, Option<i64>>(5)?.map(|value| value != 0),
        started_at: row.get::<_, Option<i64>>(6)?.map(|at| at as u64),
        ended_at: row.get::<_, Option<i64>>(7)?.map(|at| at as u64),
        continues: row.get(8)?,
        file_gone: row.get::<_, i64>(9)? != 0,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectRow {
    pub key: String,
    pub device: String,
    pub root: String,
    #[serde(skip)]
    pub repo: Option<String>,
    /// `owner/name` as GitHub last answered it.
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrRow {
    #[serde(skip)]
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub title: String,
    pub url: String,
    pub created_at: Option<u64>,
    pub closed_at: Option<u64>,
    pub merged_at: Option<u64>,
}

impl PrRow {
    /// When the pull request's life ended, merged or closed.
    pub fn end(&self) -> Option<u64> {
        self.merged_at.or(self.closed_at)
    }

    /// Its lifecycle as the record knows it: merged, closed, else open.
    pub fn badge(&self) -> crate::model::PullRequestBadge {
        use crate::model::PullRequestBadge;
        if self.merged_at.is_some() {
            PullRequestBadge::Merged
        } else if self.closed_at.is_some() {
            PullRequestBadge::Closed
        } else {
            PullRequestBadge::Open
        }
    }
}

const PR_COLUMNS: &str = "repo, number, branch, title, url, created_at, closed_at, merged_at";
const PR_COLUMNS_P: &str =
    "p.repo, p.number, p.branch, p.title, p.url, p.created_at, p.closed_at, p.merged_at";

fn pr_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PrRow> {
    Ok(PrRow {
        repo: row.get(0)?,
        number: row.get::<_, i64>(1)? as u64,
        branch: row.get(2)?,
        title: row.get(3)?,
        url: row.get(4)?,
        created_at: row.get::<_, Option<i64>>(5)?.map(|at| at as u64),
        closed_at: row.get::<_, Option<i64>>(6)?.map(|at| at as u64),
        merged_at: row.get::<_, Option<i64>>(7)?.map(|at| at as u64),
    })
}

/// One pull request as the record holds it, with its sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrLinks {
    pub pr: PrRow,
    pub issues: Vec<LinkedIssue>,
    pub worktrees: Vec<String>,
    pub sessions: Vec<LinkedSession>,
    pub total: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IssueLinks {
    pub prs: Vec<u64>,
    pub sessions: Vec<LinkedSession>,
    pub total: usize,
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
