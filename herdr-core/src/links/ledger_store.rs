//! The work ledger shares the link writer, version, size cap and retention.

use super::{LinkStore, ProjectPass, SessionKey, failed};
use crate::labels::analysis::LabelEnd;
use crate::links::ledger::*;
use crate::links::{CREATED_AFTER_MS, CREATED_BEFORE_MS, SUMMARY_SESSION_LIMIT};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const METADATA_SCHEMA: &str = "
ALTER TABLE ledger_titles ADD COLUMN goal TEXT;
ALTER TABLE ledger_titles ADD COLUMN summary TEXT NOT NULL DEFAULT '';
ALTER TABLE ledger_titles ADD COLUMN label_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ledger_titles ADD COLUMN analysed_start INTEGER;
ALTER TABLE ledger_titles ADD COLUMN analysed_end INTEGER;
";

pub(super) const SCHEMA: &str = "
ALTER TABLE session_parents ADD COLUMN parent_device TEXT NOT NULL DEFAULT '';
UPDATE session_parents SET parent_device=device;
CREATE TABLE ledger_titles(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 native_title TEXT, manual_title TEXT, state TEXT, PRIMARY KEY(device,agent,id));
CREATE TABLE ledger_excluded(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 offset INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY(device,agent,id,offset));
CREATE TABLE ledger_unjudged(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 offset INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY(device,agent,id,offset));
CREATE TABLE ledger_sightings(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 kind TEXT NOT NULL, repo TEXT NOT NULL, number INTEGER NOT NULL, at INTEGER NOT NULL,
 PRIMARY KEY(device,agent,id,kind,repo,number));
CREATE TABLE ledger_output_facts(repo TEXT NOT NULL, kind TEXT NOT NULL, number INTEGER NOT NULL,
 title TEXT NOT NULL, url TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER, stale INTEGER NOT NULL, read_at INTEGER,
 PRIMARY KEY(repo,kind,number));
CREATE TABLE ledger_requests(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 offset INTEGER NOT NULL, at INTEGER NOT NULL, text TEXT NOT NULL, author TEXT NOT NULL,
 chapter TEXT, task_ids TEXT NOT NULL DEFAULT '[]', judged INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(device,agent,id,offset));
CREATE INDEX ledger_requests_time ON ledger_requests(device,agent,id,at);
CREATE TABLE ledger_chapters(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 chapter TEXT NOT NULL, goal TEXT NOT NULL, at INTEGER NOT NULL,
 PRIMARY KEY(device,agent,id,chapter));
CREATE TABLE ledger_tasks(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 task TEXT NOT NULL, chapter TEXT NOT NULL, name TEXT NOT NULL, at INTEGER NOT NULL,
 last_at INTEGER NOT NULL, end TEXT NOT NULL, PRIMARY KEY(device,agent,id,task));
CREATE TABLE ledger_missed(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 missed TEXT NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL, status TEXT NOT NULL,
 at INTEGER NOT NULL, turn_offset INTEGER NOT NULL, issue_repo TEXT, issue_number INTEGER,
 resolved_issue_number INTEGER, resolved_turn_offset INTEGER, turn_witness TEXT,
 PRIMARY KEY(device,agent,id,missed));
CREATE TABLE ledger_turns(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 offset INTEGER NOT NULL, start_at INTEGER NOT NULL, end_at INTEGER NOT NULL,
 end TEXT NOT NULL, task_ids TEXT NOT NULL, output_task TEXT,
 PRIMARY KEY(device,agent,id,offset));
CREATE TABLE ledger_outputs(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 kind TEXT NOT NULL, repo TEXT NOT NULL, number INTEGER NOT NULL, title TEXT NOT NULL,
 url TEXT NOT NULL, state TEXT NOT NULL, created_at INTEGER NOT NULL,
 sighted_at INTEGER NOT NULL, task TEXT, PRIMARY KEY(device,agent,id,kind,repo,number));
CREATE TABLE ledger_issue_attempts(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 missed TEXT NOT NULL, repo TEXT NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL,
 started_at INTEGER NOT NULL, state TEXT NOT NULL, number INTEGER, url TEXT, failure TEXT,
 PRIMARY KEY(device,agent,id,missed));
INSERT INTO ledger_requests(device,agent,id,offset,at,text,author)
 SELECT device,agent,id,0,COALESCE(last_request_at,ended_at,started_at,0),last_request,'\"operator\"'
 FROM sessions WHERE last_request IS NOT NULL AND trim(last_request)!='';
";

pub(super) const ORIGIN_SCHEMA: &str = "
CREATE TABLE ledger_project_connections(project TEXT PRIMARY KEY, repository TEXT, since_at INTEGER NOT NULL, epoch INTEGER NOT NULL);
CREATE TABLE ledger_session_origins(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 project TEXT, repository TEXT, PRIMARY KEY(device,agent,id));
INSERT INTO ledger_session_origins(device,agent,id) SELECT device,agent,id FROM sessions;
";

/// One immutable, nullable identity per retained session. A first discovery
/// of old history is not evidence of the repository connected today.
pub(super) fn capture_session_origin(
    tx: &Transaction<'_>,
    session: &SessionIdentity,
    cwd: Option<&str>,
    started_at: Option<u64>,
) -> Result<(), String> {
    if tx
        .query_row(
            "SELECT 1 FROM ledger_session_origins WHERE device=?1 AND agent=?2 AND id=?3",
            params![session.device_id, session.agent, session.session_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(failed)?
        .is_some()
    {
        return Ok(());
    }
    // The owner is the deepest project root or recorded worktree on the
    // session's device whose folder holds its cwd (`super::within`), chosen
    // by SQLite: the record keeps every worktree a project has had, so
    // loading them all to choose here bounded new sessions by that history.
    let owner = match cwd {
        Some(cwd) => tx
            .prepare_cached(
                "SELECT root,repository,since_at FROM (SELECT p.root,c.repository,c.since_at,rtrim(p.root,'/') AS path FROM projects p JOIN ledger_project_connections c ON c.project=p.key WHERE p.device=?1 UNION SELECT p.root,c.repository,c.since_at,rtrim(w.path,'/') FROM projects p JOIN ledger_project_connections c ON c.project=p.key JOIN worktrees w ON w.project=p.key WHERE p.device=?1) WHERE path!='' AND (?2=path OR substr(?2,1,length(path)+1)=path||'/') ORDER BY length(path) DESC,root DESC,repository DESC,since_at DESC LIMIT 1",
            )
            .map_err(failed)?
            .query_row(params![session.device_id, cwd], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })
            .optional()
            .map_err(failed)?,
        None => None,
    };
    let (project, repository) = owner.map_or((None, None), |(root, repository, since)| {
        if started_at.is_some_and(|at| at >= since) {
            (Some(root), repository)
        } else {
            (None, None)
        }
    });
    tx.execute(
        "INSERT INTO ledger_session_origins VALUES(?1,?2,?3,?4,?5)",
        params![
            session.device_id,
            session.agent,
            session.session_id,
            project,
            repository
        ],
    )
    .map_err(failed)?;
    Ok(())
}

/// The backlog (PRD overview-device-tasks D-13, D-34): which item a missed
/// line became, and the typed items' durable GitHub attempts. Version 4,
/// after the session origins of version 3.
pub(super) const BACKLOG_SCHEMA: &str = "
ALTER TABLE ledger_missed ADD COLUMN backlog_key TEXT;
CREATE TABLE ledger_backlog(intent TEXT PRIMARY KEY, device TEXT NOT NULL, project TEXT NOT NULL,
 workspace TEXT NOT NULL, repo TEXT NOT NULL, title TEXT NOT NULL, started_at INTEGER NOT NULL,
 state TEXT NOT NULL, number INTEGER, url TEXT);
";

/// Version 7 (PRD session-record-goal-flow): the AI task checklist goes,
/// with no way back, and the record keeps what a goal flow needs instead:
/// a summary per request, who sent a letter, an image count, a line and the
/// non-PR outputs per turn, and how many requests the cap dropped. Requests
/// already recorded are owed no summary (Non-goals): only those after this
/// moment are.
pub(super) const FLOW_SCHEMA: &str = "
DROP TABLE ledger_tasks;
ALTER TABLE ledger_requests DROP COLUMN task_ids;
ALTER TABLE ledger_requests DROP COLUMN chapter;
ALTER TABLE ledger_requests ADD COLUMN summary TEXT;
ALTER TABLE ledger_requests ADD COLUMN answers TEXT;
ALTER TABLE ledger_requests ADD COLUMN letter_from TEXT;
ALTER TABLE ledger_requests ADD COLUMN images INTEGER NOT NULL DEFAULT 0;
ALTER TABLE ledger_turns DROP COLUMN task_ids;
ALTER TABLE ledger_turns DROP COLUMN output_task;
ALTER TABLE ledger_turns ADD COLUMN line TEXT NOT NULL DEFAULT '';
ALTER TABLE ledger_turns ADD COLUMN artifacts TEXT NOT NULL DEFAULT '[]';
ALTER TABLE ledger_outputs DROP COLUMN task;
ALTER TABLE ledger_titles ADD COLUMN omitted_requests INTEGER NOT NULL DEFAULT 0;
INSERT OR REPLACE INTO meta(key,value) VALUES('summaries_owed_since',CAST(CAST(unixepoch('subsec')*1000 AS INTEGER) AS TEXT));
";

/// Version 8 (PRD session-record-fact-status D-02): a flow keeps the line
/// the label that closed it gave. Nothing recorded changes.
pub(super) const CLOSED_FLOW_SCHEMA: &str = "
ALTER TABLE ledger_chapters ADD COLUMN closed_result TEXT;
";

/// The inverse of [`CLOSED_FLOW_SCHEMA`], for tests that open an older record.
#[cfg(test)]
pub(super) const UNDO_CLOSED_FLOW_SCHEMA: &str = "
ALTER TABLE ledger_chapters DROP COLUMN closed_result;
";

/// Version 9 (PRD hide-plan D-30): each session's own plan, one row per
/// native session, the plan as JSON. It is deleted with the session's other
/// rows; a store a test rolled back to an earlier version may still hold it.
pub(super) const PLAN_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ledger_plans(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 plan TEXT NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY(device,agent,id));
";

/// The inverse of [`FLOW_SCHEMA`], for tests that open a version 6 record.
#[cfg(test)]
pub(super) const UNDO_FLOW_SCHEMA: &str = "
CREATE TABLE ledger_tasks(device TEXT NOT NULL, agent TEXT NOT NULL, id TEXT NOT NULL,
 task TEXT NOT NULL, chapter TEXT NOT NULL, name TEXT NOT NULL, at INTEGER NOT NULL,
 last_at INTEGER NOT NULL, end TEXT NOT NULL, PRIMARY KEY(device,agent,id,task));
ALTER TABLE ledger_requests ADD COLUMN chapter TEXT;
ALTER TABLE ledger_requests ADD COLUMN task_ids TEXT NOT NULL DEFAULT '[]';
ALTER TABLE ledger_requests DROP COLUMN summary;
ALTER TABLE ledger_requests DROP COLUMN answers;
ALTER TABLE ledger_requests DROP COLUMN letter_from;
ALTER TABLE ledger_requests DROP COLUMN images;
ALTER TABLE ledger_turns ADD COLUMN task_ids TEXT NOT NULL DEFAULT '[]';
ALTER TABLE ledger_turns ADD COLUMN output_task TEXT;
ALTER TABLE ledger_turns DROP COLUMN line;
ALTER TABLE ledger_turns DROP COLUMN artifacts;
ALTER TABLE ledger_outputs ADD COLUMN task TEXT;
ALTER TABLE ledger_titles DROP COLUMN omitted_requests;
DELETE FROM meta WHERE key='summaries_owed_since';
";

fn encoded(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| "ledger_value_invalid".to_owned())
}

fn decoded<T: serde::de::DeserializeOwned>(value: String) -> Result<T, String> {
    serde_json::from_str(&value).map_err(|_| "ledger_record_invalid".to_owned())
}

fn identity(key: &SessionKey) -> SessionIdentity {
    SessionIdentity {
        device_id: key.device.clone(),
        agent: key.agent.clone(),
        session_id: key.id.clone(),
    }
}

/// A late delegation changes the authority of past requests. Retract their
/// judgments atomically; an already acknowledged external intent is retained.
pub(super) fn retract_operator_work(
    tx: &Transaction<'_>,
    session: &SessionIdentity,
) -> Result<(), String> {
    let invalid: Vec<u64> = tx.prepare("SELECT offset FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 AND author='\"operator\"'")
        .map_err(failed)?.query_map(params![session.device_id,session.agent,session.session_id],|r|r.get(0)).map_err(failed)?.collect::<Result<_,_>>().map_err(failed)?;
    for offset in &invalid {
        tx.execute("UPDATE ledger_missed SET status='\"open\"',issue_repo=NULL,issue_number=NULL,resolved_issue_number=NULL,resolved_turn_offset=NULL WHERE device=?1 AND agent=?2 AND id=?3 AND resolved_turn_offset=?4 AND NOT EXISTS(SELECT 1 FROM ledger_issue_attempts a WHERE a.device=ledger_missed.device AND a.agent=ledger_missed.agent AND a.id=ledger_missed.id AND a.missed=ledger_missed.missed)",params![session.device_id,session.agent,session.session_id,*offset as i64]).map_err(failed)?;
        tx.execute(
            "DELETE FROM ledger_missed WHERE device=?1 AND agent=?2 AND id=?3 AND turn_offset=?4",
            params![
                session.device_id,
                session.agent,
                session.session_id,
                *offset as i64
            ],
        )
        .map_err(failed)?;
        tx.execute(
            "DELETE FROM ledger_turns WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4",
            params![
                session.device_id,
                session.agent,
                session.session_id,
                *offset as i64
            ],
        )
        .map_err(failed)?;
    }
    if invalid.is_empty() {
        return Ok(());
    }
    crate::diagnostic!(
        serde_json::json!({"component":"links","kind":"ledger.lineage_retracted","device_id":session.device_id,"session_id":session.session_id,"count":invalid.len()})
    );
    Ok(())
}

fn key(session: &SessionIdentity) -> SessionKey {
    SessionKey {
        device: session.device_id.clone(),
        agent: session.agent.clone(),
        id: session.session_id.clone(),
    }
}

fn issue_attempt(tx: &Transaction<'_>, key: &AttemptKey) -> Result<Option<IssueAttempt>, String> {
    let Some(mut attempt) = stored_attempt(tx, key)? else {
        return Ok(None);
    };
    if attempt.number.is_none() {
        attempt.claimed = claimed_numbers(tx, &attempt.repository)?;
    }
    Ok(Some(attempt))
}

/// Every issue number an attempt in `repository` already names.
fn claimed_numbers(tx: &Transaction<'_>, repository: &str) -> Result<Vec<u64>, String> {
    let mut statement = tx
        .prepare(
            "SELECT number FROM ledger_issue_attempts WHERE number IS NOT NULL AND repo=?1 COLLATE NOCASE \
             UNION SELECT number FROM ledger_backlog WHERE number IS NOT NULL AND repo=?1 COLLATE NOCASE",
        )
        .map_err(failed)?;
    let numbers = statement
        .query_map([repository], |row| row.get::<_, i64>(0))
        .map_err(failed)?
        .filter_map(|number| number.ok().and_then(|number| u64::try_from(number).ok()))
        .collect();
    Ok(numbers)
}

fn stored_attempt(tx: &Transaction<'_>, key: &AttemptKey) -> Result<Option<IssueAttempt>, String> {
    match key {
        AttemptKey::Missed(source) => tx.query_row("SELECT repo,title,body,started_at,number,url FROM ledger_issue_attempts WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4",
            params![source.session.device_id,source.session.agent,source.session.session_id,source.id],|r|Ok(IssueAttempt {
                repository:r.get(0)?, title:r.get(1)?, body:r.get(2)?, started_at_unix_ms:r.get(3)?,reconcile:true,number:r.get(4)?,url:r.get(5)?,claimed:Vec::new(),
            })).optional().map_err(failed),
        AttemptKey::Intent(intent) => tx.query_row("SELECT repo,title,started_at,number,url FROM ledger_backlog WHERE intent=?1",
            params![intent.intent],|r|Ok(IssueAttempt {
                repository:r.get(0)?, title:r.get(1)?, body:String::new(), started_at_unix_ms:r.get(2)?,reconcile:true,number:r.get(3)?,url:r.get(4)?,claimed:Vec::new(),
            })).optional().map_err(failed),
    }
}

fn ledger_missed_rows(
    connection: &rusqlite::Connection,
    session: &SessionIdentity,
) -> Result<Vec<MissedWork>, String> {
    let mut missed_items = Vec::new();
    let mut statement=connection.prepare("SELECT missed,kind,text,status,at,turn_offset,issue_repo,issue_number, \
        EXISTS(SELECT 1 FROM ledger_issue_attempts a WHERE a.device=m.device AND a.agent=m.agent AND a.id=m.id \
        AND a.missed=m.missed AND a.state='creating'),turn_witness,backlog_key FROM ledger_missed m WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC").map_err(failed)?;
    let missed = statement
        .query_map(
            params![session.device_id, session.agent, session.session_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, u64>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<u64>>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .map_err(failed)?;
    for missed in missed {
        let (id, kind, text, status, at, offset, repo, number, creating, witness, backlog_key) =
            missed.map_err(failed)?;
        missed_items.push(MissedWork {
            id,
            kind: decoded(kind)?,
            text,
            status: decoded(status)?,
            at_unix_ms: at,
            turn_offset: offset,
            turn_witness: witness,
            issue_repository: repo,
            issue_number: number,
            backlog_key,
            creating,
        });
    }
    Ok(missed_items)
}

pub(super) fn apply_reader(
    tx: &Transaction<'_>,
    device: &str,
    agent: &str,
    id: &str,
    facts: &hide_session::links::LinkFacts,
) -> Result<(), String> {
    let session = SessionIdentity {
        device_id: device.into(),
        agent: agent.into(),
        session_id: id.into(),
    };
    if facts.requests_capped > 0 {
        crate::diagnostic!(
            serde_json::json!({"component":"links","kind":"ledger.reader_capacity","session_id":id,"device_id":device,"agent":agent,"dropped":facts.requests_capped})
        );
    }
    let parent: Option<String> = tx
        .query_row(
            "SELECT parent_name FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3",
            params![device, agent, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(failed)?;
    let known_first: Option<u64> = tx
        .query_row(
            "SELECT MIN(offset) FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3",
            params![device, agent, id],
            |r| r.get(0),
        )
        .map_err(failed)?;
    let first = known_first
        .into_iter()
        .chain(facts.requests.iter().map(|request| request.offset))
        .min();
    for request in &facts.requests {
        // A letter is kept as a dim line under its sender; only the
        // operator's input, or a delegated session's brief and its parent's
        // work requests, count as requests (B24).
        let letter_from = match (parent.as_deref(), request.sender.as_deref()) {
            (Some(parent), Some(sender))
                if sender == parent && request.letter_kind.as_deref() == Some("request") =>
            {
                None
            }
            (_, Some(sender)) => Some(sender),
            (Some(_), None) if Some(request.offset) != first => continue,
            (_, None) => None,
        };
        let author = if parent.is_some() {
            WorkAuthor::Parent
        } else {
            WorkAuthor::Operator
        };
        tx.execute("INSERT INTO ledger_requests(device,agent,id,offset,at,text,author,letter_from) \
            SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE NOT EXISTS(SELECT 1 FROM ledger_excluded WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4) \
            ON CONFLICT(device,agent,id,offset) DO UPDATE SET text=excluded.text,at=excluded.at,letter_from=COALESCE(excluded.letter_from,letter_from)",
            params![device,agent,id,request.offset as i64,request.at_unix_ms as i64,hide_session::links::cut(&request.text),encoded(&author)?,letter_from]).map_err(failed)?;
    }
    for (kind, marks) in [("issue", &facts.issues)] {
        for mark in marks {
            tx.execute("INSERT INTO ledger_sightings VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET at=MIN(at,excluded.at)",
                params![device,agent,id,kind,mark.repository.to_ascii_lowercase(),mark.number as i64,mark.at_unix_ms as i64]).map_err(failed)?;
        }
    }
    cap_requests(tx, &session)?;
    cap(tx, &session, "ledger_sightings", SIGHTING_LIMIT)?;
    Ok(())
}

pub(super) fn apply_output_facts(
    tx: &Transaction<'_>,
    outputs: &[crate::links::OutputFact],
) -> Result<(), String> {
    for output in outputs {
        tx.execute("INSERT INTO ledger_output_facts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(repo,kind,number) DO UPDATE SET title=excluded.title,url=excluded.url,state=excluded.state,created_at=COALESCE(excluded.created_at,created_at),stale=excluded.stale,read_at=COALESCE(excluded.read_at,read_at)",
            params![output.repository.to_ascii_lowercase(),output.kind,output.number as i64,output.title,output.url,output.state,output.created_at.map(|n|n as i64),i64::from(output.stale),output.last_read_at.map(|n|n as i64)]).map_err(failed)?;
    }
    Ok(())
}

/// Whether a turn at this offset is already judged or recorded unjudged.
fn turn_recorded(
    connection: &rusqlite::Connection,
    session: &SessionIdentity,
    offset: u64,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_turns \
            WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4) OR EXISTS(SELECT 1 FROM ledger_unjudged \
            WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4)",
            params![session.device_id, session.agent, session.session_id, offset as i64],
            |row| row.get(0),
        )
        .map_err(failed)
}

fn cap(
    tx: &Transaction<'_>,
    session: &SessionIdentity,
    table: &str,
    limit: usize,
) -> Result<(), String> {
    let count: usize = tx
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE device=?1 AND agent=?2 AND id=?3"),
            params![session.device_id, session.agent, session.session_id],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if count > limit {
        tx.execute(
            &format!(
                "DELETE FROM {table} WHERE rowid IN (SELECT rowid FROM {table} \
                WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at,rowid LIMIT ?4)"
            ),
            params![
                session.device_id,
                session.agent,
                session.session_id,
                (count - limit) as i64
            ],
        )
        .map_err(failed)?;
        crate::diagnostic!(serde_json::json!({
            "component":"links", "kind":"ledger.capacity", "table":table,
            "device_id":session.device_id, "agent":session.agent,
            "session_id":session.session_id, "removed":count-limit,
        }));
    }
    Ok(())
}

/// The request cap (B30): the oldest go, and the record counts them so it
/// can say how many it no longer shows.
fn cap_requests(tx: &Transaction<'_>, session: &SessionIdentity) -> Result<(), String> {
    let count: u64 = tx
        .query_row(
            "SELECT count(*) FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3",
            params![session.device_id, session.agent, session.session_id],
            |row| row.get(0),
        )
        .map_err(failed)?;
    let Some(removed) = count.checked_sub(REQUEST_LIMIT as u64).filter(|n| *n > 0) else {
        return Ok(());
    };
    cap(tx, session, "ledger_requests", REQUEST_LIMIT)?;
    tx.execute(
        "INSERT INTO ledger_titles(device,agent,id,omitted_requests) VALUES(?1,?2,?3,?4) \
         ON CONFLICT(device,agent,id) DO UPDATE SET omitted_requests=omitted_requests+excluded.omitted_requests",
        params![session.device_id, session.agent, session.session_id, removed as i64],
    )
    .map_err(failed)?;
    Ok(())
}

impl LinkStore {
    /// Small history read on the existing writer. No task tree, raw text,
    /// path, status, question or END crosses this boundary.
    fn ledger_metadata(
        &self,
        session: &SessionIdentity,
    ) -> Result<Option<SessionMetadata>, String> {
        let mut metadata = self.connection.query_row(
            "SELECT native_title,manual_title,COALESCE(goal,(SELECT goal FROM ledger_chapters c \
             WHERE c.device=t.device AND c.agent=t.agent AND c.id=t.id ORDER BY at DESC LIMIT 1)),summary,analysed_start,analysed_end \
             FROM ledger_titles t WHERE device=?1 AND agent=?2 AND id=?3",
            params![session.device_id, session.agent, session.session_id],
            |r| Ok(SessionMetadata { native_title:r.get(0)?, manual_title:r.get(1)?, goal:r.get(2)?, summary:r.get(3)?, created_prs:vec![], analysed_start:r.get::<_,Option<i64>>(4)?.map(|v|v as u64), analysed_end:r.get::<_,Option<i64>>(5)?.map(|v|v as u64) })
        ).optional().map_err(failed)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT repo,number,max(at) FROM (SELECT repo,number,sighted_at AS at FROM ledger_outputs WHERE device=?1 AND agent=?2 AND id=?3 AND kind='pr' UNION ALL SELECT repo,number,at FROM ledger_sightings WHERE device=?1 AND agent=?2 AND id=?3 AND kind='pr') GROUP BY repo,number ORDER BY max(at) DESC LIMIT 20"
        ).map_err(failed)?;
        let prs = statement
            .query_map(
                params![session.device_id, session.agent, session.session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        if !prs.is_empty() {
            metadata.get_or_insert_default().created_prs = prs;
        }
        Ok(metadata)
    }

    /// An incremental native read may omit the session header. The cursor's
    /// retained identity is authoritative for the context that changed.
    pub fn ledger_file_session(
        &self,
        device: &str,
        path: &str,
    ) -> Result<Option<SessionIdentity>, String> {
        self.connection
            .query_row(
                "SELECT agent,session FROM cursors WHERE device=?1 AND path=?2 AND session IS NOT NULL AND subagent=0",
                [device, path],
                |row| Ok(SessionIdentity {
                    device_id: device.to_owned(),
                    agent: row.get(0)?,
                    session_id: row.get(1)?,
                }),
            )
            .optional()
            .map_err(failed)
    }

    pub fn recover_issue_attempts(&self) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE ledger_issue_attempts SET state='unknown' WHERE state='creating'",
                [],
            )
            .map_err(failed)?;
        Ok(())
    }

    pub fn ledger_call(&mut self, call: LedgerCall) -> Result<LedgerAnswer, String> {
        if let LedgerCall::SaveMetadata {
            session,
            metadata,
            at_unix_ms,
        } = call
        {
            self.apply_ledger_update(&LedgerUpdate::Metadata {
                session: session.clone(),
                native_title: metadata.native_title,
                manual_title: metadata.manual_title,
                goal: metadata.goal,
                summary: metadata.summary,
                created_prs: metadata.created_prs,
                at_unix_ms,
                analysed_start: metadata.analysed_start,
                analysed_end: metadata.analysed_end,
            })?;
            return self.ledger_metadata(&session).map(LedgerAnswer::Metadata);
        }
        if let LedgerCall::Metadata(session) = &call {
            return self.ledger_metadata(session).map(LedgerAnswer::Metadata);
        }
        if let LedgerCall::Record(session) = &call {
            return self
                .ledger_record(session)
                .map(Box::new)
                .map(LedgerAnswer::Record);
        }
        if let LedgerCall::Plans = &call {
            return self.ledger_plans().map(LedgerAnswer::Plans);
        }
        // Only explicit external-effect calls use FULL. Its WAL sync completes
        // before the RPC acknowledgement permits GitHub to run.
        self.connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(failed)?;
        let result = self.ledger_issue_call(call);
        let restored = self
            .connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(failed);
        restored?;
        result
    }

    fn ledger_issue_call(&mut self, call: LedgerCall) -> Result<LedgerAnswer, String> {
        let tx = self.connection.transaction().map_err(failed)?;
        let answer = match call {
            LedgerCall::Record(_)
            | LedgerCall::Metadata(_)
            | LedgerCall::SaveMetadata { .. }
            | LedgerCall::Plans => {
                unreachable!()
            }
            LedgerCall::ReserveIssue {
                key,
                repository,
                title,
                body,
                at_unix_ms,
            } => {
                if !hide_node_link::gh::is_repository(&repository) {
                    return Err("ledger_issue_repository_invalid".into());
                }
                if let Some(attempt) = issue_attempt(&tx, &key)? {
                    if !attempt.repository.eq_ignore_ascii_case(&repository) {
                        return Err("ledger_issue_repository_changed".into());
                    }
                    LedgerAnswer::Attempt(attempt)
                } else {
                    let title = crate::local_issues::validated_title(&title)
                        .map_err(|_| "ledger_issue_title_invalid")?;
                    let body = crate::local_issues::validated_body(&body)
                        .map_err(|_| "ledger_issue_body_invalid")?;
                    match &key {
                        AttemptKey::Missed(source) => {
                            let status:Option<String> = tx.query_row("SELECT status FROM ledger_missed WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4",
                                params![source.session.device_id,source.session.agent,source.session.session_id,source.id],|r|r.get(0)).optional().map_err(failed)?;
                            if status.as_deref() != Some(encoded(&MissedStatus::Open)?.as_str()) {
                                return Err("ledger_missed_not_open".into());
                            }
                            tx.execute("INSERT INTO ledger_issue_attempts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'creating',NULL,NULL,NULL)",
                                params![source.session.device_id,source.session.agent,source.session.session_id,source.id,repository,title,body,at_unix_ms as i64]).map_err(failed)?;
                        }
                        AttemptKey::Intent(intent) => {
                            tx.execute("DELETE FROM ledger_backlog WHERE rowid IN (SELECT rowid FROM ledger_backlog WHERE state IN ('complete','dismissed','undone') ORDER BY started_at DESC LIMIT -1 OFFSET ?1)",
                                params![(BACKLOG_INTENT_LIMIT / 2) as i64]).map_err(failed)?;
                            // An unconfirmed attempt is never evicted: its row may
                            // still retry it. Past what a restart shows, a new
                            // create waits until one is confirmed or put away.
                            let pending: i64 = tx
                                .query_row("SELECT COUNT(*) FROM ledger_backlog WHERE state IN ('creating','unknown')", [], |r| r.get(0))
                                .map_err(failed)?;
                            if pending >= PENDING_BACKLOG_LIMIT as i64 {
                                return Err("ledger_backlog_pending_capacity".into());
                            }
                            let kept: i64 = tx
                                .query_row("SELECT COUNT(*) FROM ledger_backlog", [], |r| r.get(0))
                                .map_err(failed)?;
                            if kept >= BACKLOG_INTENT_LIMIT as i64 {
                                return Err("ledger_backlog_capacity".into());
                            }
                            tx.execute("INSERT INTO ledger_backlog VALUES(?1,?2,?3,?4,?5,?6,?7,'creating',NULL,NULL)",
                                params![intent.intent,intent.device,intent.project,intent.workspace_id,repository,title,at_unix_ms as i64]).map_err(failed)?;
                        }
                    }
                    LedgerAnswer::Attempt(IssueAttempt {
                        repository,
                        title,
                        body,
                        started_at_unix_ms: at_unix_ms,
                        reconcile: false,
                        number: None,
                        url: None,
                        claimed: Vec::new(),
                    })
                }
            }
            LedgerCall::StartIssue {
                key,
                title,
                body,
                at_unix_ms,
            } => {
                let title = crate::local_issues::validated_title(&title)
                    .map_err(|_| "ledger_issue_title_invalid")?;
                let body = crate::local_issues::validated_body(&body)
                    .map_err(|_| "ledger_issue_body_invalid")?;
                let changed = match &key {
                    AttemptKey::Missed(source) => tx.execute("UPDATE ledger_issue_attempts SET title=?5,body=?6,started_at=?7,state='creating',failure=NULL WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND number IS NULL",
                        params![source.session.device_id,source.session.agent,source.session.session_id,source.id,title,body,at_unix_ms as i64]).map_err(failed)?,
                    AttemptKey::Intent(intent) => tx.execute("UPDATE ledger_backlog SET title=?2,started_at=?3,state='creating' WHERE intent=?1 AND number IS NULL",
                        params![intent.intent,title,at_unix_ms as i64]).map_err(failed)?,
                };
                if changed != 1 {
                    return Err("ledger_issue_attempt_missing".into());
                }
                LedgerAnswer::Saved
            }
            LedgerCall::FinishIssue { key, number, url } => {
                let attempt = issue_attempt(&tx, &key)?.ok_or("ledger_issue_attempt_missing")?;
                let reference = crate::issues::IssueReference::parse(&url, None)
                    .map_err(|_| "ledger_issue_url_invalid")?;
                if !reference
                    .repository
                    .eq_ignore_ascii_case(&attempt.repository)
                    || u64::from(reference.number) != number
                    || attempt.number.is_some_and(|kept| kept != number)
                {
                    return Err("ledger_issue_answer_mismatched".into());
                }
                match &key {
                    AttemptKey::Missed(source) => {
                        tx.execute("UPDATE ledger_issue_attempts SET state='complete',number=?5,url=?6,failure=NULL WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4",
                            params![source.session.device_id,source.session.agent,source.session.session_id,source.id,number as i64,url]).map_err(failed)?;
                        let backlog_key = crate::tasks::github_key(&reference);
                        let changed=tx.execute("UPDATE ledger_missed SET status=?5,issue_repo=?6,issue_number=?7,backlog_key=?8 WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4",
                            params![source.session.device_id,source.session.agent,source.session.session_id,source.id,encoded(&MissedStatus::Backlog)?,attempt.repository,number as i64,backlog_key]).map_err(failed)?;
                        if changed != 1 {
                            return Err("ledger_missed_missing".into());
                        }
                    }
                    AttemptKey::Intent(intent) => {
                        tx.execute("UPDATE ledger_backlog SET state='complete',number=?2,url=?3 WHERE intent=?1",
                            params![intent.intent,number as i64,url]).map_err(failed)?;
                    }
                }
                LedgerAnswer::Saved
            }
            LedgerCall::UnknownIssue(key) => {
                match &key {
                    AttemptKey::Missed(source) => tx.execute("UPDATE ledger_issue_attempts SET state='unknown' WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND number IS NULL",
                        params![source.session.device_id,source.session.agent,source.session.session_id,source.id]).map_err(failed)?,
                    AttemptKey::Intent(intent) => tx.execute("UPDATE ledger_backlog SET state='unknown' WHERE intent=?1 AND number IS NULL",
                        params![intent.intent]).map_err(failed)?,
                };
                LedgerAnswer::Saved
            }
            LedgerCall::BacklogLocal {
                source,
                backlog_key,
            } => {
                let changed = tx.execute("UPDATE ledger_missed SET status=?5,backlog_key=?6 WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND status=?7",
                    params![source.session.device_id,source.session.agent,source.session.session_id,source.id,encoded(&MissedStatus::Backlog)?,backlog_key,encoded(&MissedStatus::Open)?]).map_err(failed)?;
                if changed != 1 {
                    return Err("ledger_missed_not_open".into());
                }
                LedgerAnswer::Saved
            }
            LedgerCall::ReleaseBacklog { target } => {
                let backlog_key = target.key();
                let device = target.local_device();
                // A typed item's issue was closed: its create no longer
                // names an issue `되돌리기` may close.
                if let BacklogTarget::Github(reference) = &target {
                    tx.execute("UPDATE ledger_backlog SET state='undone' WHERE state='complete' AND lower(repo)=lower(?1) AND number=?2",
                        params![reference.repository,reference.number]).map_err(failed)?;
                }
                tx.execute("DELETE FROM ledger_issue_attempts WHERE EXISTS(SELECT 1 FROM ledger_missed m WHERE m.device=ledger_issue_attempts.device AND m.agent=ledger_issue_attempts.agent AND m.id=ledger_issue_attempts.id AND m.missed=ledger_issue_attempts.missed AND m.backlog_key=?1 AND (?2 IS NULL OR m.device=?2))",
                    params![backlog_key,device]).map_err(failed)?;
                tx.execute("UPDATE ledger_missed SET status=?2,issue_repo=NULL,issue_number=NULL,backlog_key=NULL WHERE backlog_key=?1 AND (?3 IS NULL OR device=?3)",
                    params![backlog_key,encoded(&MissedStatus::Open)?,device]).map_err(failed)?;
                LedgerAnswer::Saved
            }
            LedgerCall::DismissBacklog { intent } => {
                tx.execute("UPDATE ledger_backlog SET state='dismissed' WHERE intent=?1 AND state!='complete'",
                    params![intent]).map_err(failed)?;
                LedgerAnswer::Saved
            }
            LedgerCall::PendingBacklog => {
                let mut statement = tx.prepare("SELECT intent,device,project,workspace,title,started_at FROM ledger_backlog WHERE state='creating' OR state='unknown' ORDER BY started_at").map_err(failed)?;
                let rows = statement
                    .query_map([], |r| {
                        Ok(PendingBacklog {
                            intent: BacklogIntent {
                                intent: r.get(0)?,
                                device: r.get(1)?,
                                project: r.get(2)?,
                                workspace_id: r.get(3)?,
                            },
                            title: r.get(4)?,
                            started_at_unix_ms: r.get(5)?,
                        })
                    })
                    .map_err(failed)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(failed)?;
                drop(statement);
                LedgerAnswer::Pending(rows)
            }
            LedgerCall::BacklogOrigin { target } => {
                let backlog_key = target.key();
                let found: Option<(String, String, String)> = tx
                    .query_row(
                        "SELECT device,agent,id FROM ledger_missed WHERE backlog_key=?1 AND (?2 IS NULL OR device=?2) ORDER BY at DESC,device,agent,id,missed LIMIT 1",
                        params![backlog_key,target.local_device()],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(failed)?;
                let origin = match found {
                    None => None,
                    Some((device_id, agent, session_id)) => {
                        let session = SessionIdentity {
                            device_id,
                            agent,
                            session_id,
                        };
                        let missed = ledger_missed_rows(&tx, &session)?
                            .into_iter()
                            .find(|m| m.backlog_key.as_deref() == Some(backlog_key.as_str()))
                            .ok_or("ledger_missed_missing")?;
                        Some(Box::new(BacklogOrigin { session, missed }))
                    }
                };
                LedgerAnswer::Origin(origin)
            }
            LedgerCall::BacklogKeys => {
                let mut statement = tx
                    .prepare("SELECT backlog_key,CASE WHEN substr(backlog_key,1,6)='local:' THEN device ELSE NULL END AS owner_device FROM ledger_missed WHERE backlog_key IS NOT NULL GROUP BY backlog_key,owner_device ORDER BY MAX(at) DESC,backlog_key,owner_device LIMIT ?1")
                    .map_err(failed)?;
                let missed = statement
                    .query_map(params![BACKLOG_KEYS_LIMIT as i64], |r| {
                        Ok(BacklogMissedKey {
                            task_key: r.get(0)?,
                            device_id: r.get(1)?,
                        })
                    })
                    .map_err(failed)?
                    .collect::<Result<Vec<BacklogMissedKey>, _>>()
                    .map_err(failed)?;
                drop(statement);
                let mut statement = tx
                    .prepare("SELECT repo,number FROM (SELECT repo,number,started_at FROM ledger_backlog WHERE state='complete' AND number IS NOT NULL UNION ALL SELECT repo,number,started_at FROM ledger_issue_attempts WHERE state='complete' AND number IS NOT NULL) GROUP BY lower(repo),number ORDER BY MAX(started_at) DESC LIMIT ?1")
                    .map_err(failed)?;
                let made = statement
                    .query_map(params![BACKLOG_KEYS_LIMIT as i64], |r| {
                        Ok(crate::tasks::github_key(&crate::issues::IssueReference {
                            repository: r.get(0)?,
                            number: r.get(1)?,
                        }))
                    })
                    .map_err(failed)?
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(failed)?;
                drop(statement);
                LedgerAnswer::BacklogKeys { missed, made }
            }
            LedgerCall::MadeIssue { repository, number } => {
                let made: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM ledger_backlog WHERE state='complete' AND lower(repo)=lower(?1) AND number=?2) OR EXISTS(SELECT 1 FROM ledger_issue_attempts WHERE state='complete' AND lower(repo)=lower(?1) AND number=?2)",
                        params![repository, number],
                        |r| r.get(0),
                    )
                    .map_err(failed)?;
                LedgerAnswer::Made(made)
            }
        };
        tx.commit().map_err(failed)?;
        Ok(answer)
    }

    pub fn apply_ledger_update(&mut self, update: &LedgerUpdate) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        match update {
            LedgerUpdate::Plan { session, plan } => {
                let args = params![session.device_id, session.agent, session.session_id];
                match plan {
                    Some(plan) => {
                        let json =
                            serde_json::to_string(plan).map_err(|error| error.to_string())?;
                        tx.execute(
                            "INSERT INTO ledger_plans(device,agent,id,plan,updated_at) VALUES(?1,?2,?3,?4,?5) \
                             ON CONFLICT(device,agent,id) DO UPDATE SET plan=excluded.plan,updated_at=excluded.updated_at",
                            params![session.device_id, session.agent, session.session_id, json, plan.updated_at_unix_ms],
                        )
                        .map_err(failed)?;
                    }
                    None => {
                        tx.execute(
                            "DELETE FROM ledger_plans WHERE device=?1 AND agent=?2 AND id=?3",
                            args,
                        )
                        .map_err(failed)?;
                    }
                }
            }
            LedgerUpdate::Metadata {
                session,
                native_title,
                manual_title,
                goal,
                summary,
                created_prs,
                at_unix_ms,
                analysed_start,
                analysed_end,
            } => {
                // The same bounded text as labels.json; no raw conversation
                // or runtime verdict is copied into this display record.
                if goal.as_ref().is_some_and(|s| s.chars().count() > 256)
                    || summary.chars().count() > 256
                    || created_prs.len() > 20
                {
                    return Err("ledger_metadata_capacity".into());
                }
                let indexed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE device=?1 AND agent=?2 AND id=?3)",
                    params![session.device_id,session.agent,session.session_id], |r| r.get(0)).map_err(failed)?;
                if !indexed {
                    return Err("ledger_metadata_session_not_indexed".into());
                }
                tx.execute("INSERT INTO ledger_titles(device,agent,id,goal,summary,label_at,analysed_start,analysed_end,native_title,manual_title) \
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(device,agent,id) DO UPDATE SET \
                    native_title=excluded.native_title,manual_title=excluded.manual_title,goal=excluded.goal,summary=excluded.summary,label_at=excluded.label_at,analysed_start=excluded.analysed_start,analysed_end=excluded.analysed_end \
                    WHERE excluded.label_at>=ledger_titles.label_at",
                    params![session.device_id, session.agent, session.session_id, goal, summary, *at_unix_ms as i64, analysed_start.map(|v|v as i64),analysed_end.map(|v|v as i64),native_title,manual_title]).map_err(failed)?;
                for (repository, number, at) in created_prs {
                    tx.execute("INSERT INTO ledger_sightings(device,agent,id,kind,repo,number,at) VALUES(?1,?2,?3,'pr',?4,?5,?6) ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET at=max(at,excluded.at)",
                        params![session.device_id,session.agent,session.session_id,repository,number,*at as i64]).map_err(failed)?;
                }
                cap(&tx, session, "ledger_sightings", SIGHTING_LIMIT)?;
                // A pull request the label judged this session made is one of
                // its outputs even when the link reader's window missed it (D-14).
                refresh_outputs(&tx)?;
            }
            LedgerUpdate::Unjudged { session, offsets } => {
                if offsets.len() > REQUEST_LIMIT {
                    return Err("ledger_request_capacity".into());
                }
                for offset in offsets {
                    tx.execute(
                        "INSERT OR IGNORE INTO ledger_unjudged VALUES(?1,?2,?3,?4,?4)",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            *offset as i64
                        ],
                    )
                    .map_err(failed)?;
                }
                cap(&tx, session, "ledger_unjudged", REQUEST_LIMIT)?;
            }
            LedgerUpdate::ExcludeRequests {
                session,
                parent_name,
                offsets,
            } => {
                let current_parent: Option<String> = tx.query_row("SELECT parent_name FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3", params![session.device_id,session.agent,session.session_id], |r| r.get(0)).optional().map_err(failed)?;
                if &current_parent != parent_name {
                    return Ok(());
                }
                if offsets.len() > REQUEST_LIMIT {
                    return Err("ledger_request_capacity".into());
                }
                for offset in offsets {
                    tx.execute(
                        "INSERT OR IGNORE INTO ledger_excluded VALUES(?1,?2,?3,?4,?4)",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            *offset as i64
                        ],
                    )
                    .map_err(failed)?;
                    tx.execute("DELETE FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4 AND judged=0",
                    params![session.device_id,session.agent,session.session_id,*offset as i64]).map_err(failed)?;
                }
                cap(&tx, session, "ledger_excluded", REQUEST_LIMIT)?;
            }
            LedgerUpdate::Title {
                session,
                native_title,
                manual_title,
                state,
            } => {
                tx.execute(
                    "INSERT INTO ledger_titles(device,agent,id,native_title,manual_title,state) VALUES(?1,?2,?3,?4,?5,?6) \
                    ON CONFLICT(device,agent,id) DO UPDATE SET native_title=excluded.native_title, \
                    manual_title=excluded.manual_title,state=excluded.state",
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        native_title,
                        manual_title,
                        state
                    ],
                )
                .map_err(failed)?;
            }
            LedgerUpdate::Requests {
                session,
                cwd,
                path,
                requests,
            } => {
                if requests.len() > REQUEST_LIMIT {
                    return Err("ledger_request_capacity".to_owned());
                }
                tx.execute("INSERT INTO sessions(device,agent,id,path,cwd) VALUES(?1,?2,?3,?4,?5) \
                    ON CONFLICT(device,agent,id) DO UPDATE SET cwd=COALESCE(excluded.cwd,cwd),path=COALESCE(excluded.path,path)",
                    params![session.device_id,session.agent,session.session_id,path,cwd]).map_err(failed)?;
                for request in requests {
                    let parent: Option<String> = tx.query_row("SELECT parent_name FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3",params![session.device_id,session.agent,session.session_id],|r|r.get(0)).optional().map_err(failed)?;
                    let first: Option<u64> = tx.query_row("SELECT MIN(offset) FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3",params![session.device_id,session.agent,session.session_id],|r|r.get(0)).map_err(failed)?;
                    if parent.is_some()
                        && request.author == WorkAuthor::Operator
                        && request.letter_from.is_none()
                        && first.is_some_and(|first| first != request.offset)
                    {
                        continue;
                    }
                    let author = if parent.is_some() {
                        WorkAuthor::Parent
                    } else {
                        request.author.clone()
                    };
                    let text = hide_session::links::cut(&request.text);
                    // A letter verdict, once made, stays: the link reader
                    // writes the same message with no sender for a `🦉` line.
                    tx.execute(
                        "INSERT INTO ledger_requests(device,agent,id,offset,at,text,author,letter_from,images) \
                        SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9 WHERE NOT EXISTS(SELECT 1 FROM ledger_excluded \
                        WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4) ON CONFLICT(device,agent,id,offset) DO UPDATE \
                        SET text=excluded.text,at=excluded.at,letter_from=COALESCE(excluded.letter_from,letter_from),images=max(images,excluded.images)",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            request.offset as i64,
                            request.at_unix_ms as i64,
                            text,
                            encoded(&author)?,
                            request.letter_from,
                            request.images
                        ],
                    )
                    .map_err(failed)?;
                }
                cap_requests(&tx, session)?;
            }
            LedgerUpdate::Chapter {
                session,
                goal,
                at_unix_ms,
                change,
                closed_result,
            } => {
                let previous: Option<(String, String)> = tx
                    .query_row(
                        "SELECT chapter,goal FROM ledger_chapters \
                    WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,rowid DESC LIMIT 1",
                        params![session.device_id, session.agent, session.session_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(failed)?;
                match (change, previous) {
                    (_, Some((_, held))) if held == *goal => {}
                    // A clearer name for the same work renames the flow it
                    // is in, within the turn (B14, B16).
                    (FlowChange::Rename, Some((chapter, _))) => {
                        tx.execute(
                            "UPDATE ledger_chapters SET goal=?5 WHERE device=?1 AND agent=?2 AND id=?3 AND chapter=?4",
                            params![session.device_id, session.agent, session.session_id, chapter, goal],
                        )
                        .map_err(failed)?;
                    }
                    // A flow opened at this same request (the turn's start)
                    // and replaced at its end is one flow, under the newer name.
                    _ => {
                        // The flow this one follows keeps how the label
                        // said it ended (D-02); a start and an end that open
                        // the same flow both say it, and the later stands.
                        if let Some(line) = closed_result
                            && *change == FlowChange::New
                        {
                            tx.execute(
                                "UPDATE ledger_chapters SET closed_result=?5 WHERE rowid=(SELECT rowid FROM ledger_chapters \
                                 WHERE device=?1 AND agent=?2 AND id=?3 AND at<?4 ORDER BY at DESC,rowid DESC LIMIT 1)",
                                params![session.device_id, session.agent, session.session_id, *at_unix_ms as i64, line],
                            )
                            .map_err(failed)?;
                        }
                        tx.execute(
                            "INSERT INTO ledger_chapters(device,agent,id,chapter,goal,at) VALUES(?1,?2,?3,?4,?5,?6) \
                             ON CONFLICT(device,agent,id,chapter) DO UPDATE SET goal=excluded.goal",
                            params![
                                session.device_id,
                                session.agent,
                                session.session_id,
                                format!("chapter-{at_unix_ms}"),
                                goal,
                                *at_unix_ms as i64
                            ],
                        )
                        .map_err(failed)?;
                        cap(&tx, session, "ledger_chapters", FLOW_LIMIT)?;
                    }
                }
            }
            LedgerUpdate::Summaries { session, summaries } => {
                if summaries.len() > SUMMARY_BATCH {
                    return Err("ledger_summary_capacity".to_owned());
                }
                for summary in summaries {
                    if summary.summary.trim().is_empty()
                        || summary.summary.chars().count() > SUMMARY_CHARS
                        || summary
                            .answers
                            .as_ref()
                            .is_some_and(|answers| answers.chars().count() > MISSED_CHARS)
                    {
                        return Err("ledger_summary_invalid".to_owned());
                    }
                    tx.execute(
                        "UPDATE ledger_requests SET summary=?5,answers=?6 WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            summary.offset as i64,
                            summary.summary,
                            summary.answers
                        ],
                    )
                    .map_err(failed)?;
                }
            }
            LedgerUpdate::TurnEnd {
                session,
                offset,
                started_at_unix_ms,
                at_unix_ms,
                end,
                line,
                work_author,
                asks_answered,
                turn_witness,
                judgment,
            } => {
                if turn_recorded(&tx, session, *offset)? {
                    return Ok(());
                }
                if judgment.missed.len() > NEW_MISSED_LIMIT
                    || judgment.resolved.len() > MISSED_LIMIT
                    || judgment.artifacts.len() > ARTIFACTS_PER_TURN
                {
                    return Err("ledger_judgment_capacity".to_owned());
                }
                if judgment.artifacts.iter().any(|artifact| {
                    artifact.trim().is_empty() || artifact.chars().count() > ARTIFACT_CHARS
                }) {
                    return Err("ledger_artifact_invalid".to_owned());
                }
                let start: Option<u64> = tx
                    .query_row(
                        "SELECT at FROM ledger_requests \
                    WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            *offset as i64
                        ],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(failed)?;
                // Lineage may have arrived while END was in flight. A
                // request removed by its current owner cannot create work.
                if *work_author && start.is_none() {
                    crate::diagnostic!(
                        serde_json::json!({"component":"links","kind":"ledger.work_request_unavailable","device_id":session.device_id,"session_id":session.session_id,"offset":offset})
                    );
                    return Ok(());
                }
                // A turn with no request of its own starts at the input that
                // woke it (D-03).
                let start_at = start.unwrap_or(*started_at_unix_ms).min(*at_unix_ms);
                tx.execute(
                    "INSERT INTO ledger_turns(device,agent,id,offset,start_at,end_at,end,line,artifacts) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        *offset as i64,
                        start_at as i64,
                        *at_unix_ms as i64,
                        encoded(end)?,
                        line,
                        encoded(&judgment.artifacts)?
                    ],
                )
                .map_err(failed)?;
                tx.execute(
                    "UPDATE ledger_requests SET judged=1 WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4",
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        *offset as i64
                    ],
                )
                .map_err(failed)?;
                // A reply answers what was asked before it (B12): an ask the
                // agent still waits on is listed again by this turn.
                if *asks_answered {
                    tx.execute(
                        "UPDATE ledger_missed SET status=?4,resolved_turn_offset=?5 \
                        WHERE device=?1 AND agent=?2 AND id=?3 AND kind=?6 AND status=?7 AND turn_offset<?5",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            encoded(&MissedStatus::Resolved)?,
                            *offset as i64,
                            encoded(&MissedKind::Ask)?,
                            encoded(&MissedStatus::Open)?
                        ],
                    )
                    .map_err(failed)?;
                }
                for (index, missed) in judgment.missed.iter().enumerate() {
                    if missed.text.trim().is_empty() || missed.text.chars().count() > MISSED_CHARS {
                        return Err("ledger_missed_invalid".to_owned());
                    }
                    // A turn a letter woke repeats a question still open, or one
                    // the operator put away: one line, and a put-away one stays put away.
                    // A question of a past flow is no longer open, so asking it
                    // again in a later turn of this one opens it again (D-03, B6).
                    if missed.kind == MissedKind::Ask
                        && tx
                            .query_row(
                                "WITH flow(at) AS (SELECT COALESCE(MAX(at),0) FROM ledger_chapters WHERE device=?1 AND agent=?2 AND id=?3) \
                                SELECT 1 FROM ledger_missed, flow WHERE device=?1 AND agent=?2 AND id=?3 \
                                AND kind=?4 AND trim(text)=?6 AND (status=?7 OR (status=?5 AND ledger_missed.at>=flow.at)) LIMIT 1",
                                params![
                                    session.device_id,
                                    session.agent,
                                    session.session_id,
                                    encoded(&MissedKind::Ask)?,
                                    encoded(&MissedStatus::Open)?,
                                    missed.text.trim(),
                                    encoded(&MissedStatus::Dismissed)?
                                ],
                                |_| Ok(()),
                            )
                            .optional()
                            .map_err(failed)?
                            .is_some()
                    {
                        continue;
                    }
                    tx.execute("INSERT INTO ledger_missed(device,agent,id,missed,kind,text,status,at,turn_offset,turn_witness) \
                        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                        params![session.device_id,session.agent,session.session_id,format!("missed-{offset}-{index}"),
                            encoded(&missed.kind)?,missed.text,encoded(&MissedStatus::Open)?,*at_unix_ms as i64,*offset as i64,turn_witness]).map_err(failed)?;
                }
                for resolved in &judgment.resolved {
                    // A printed existing issue is insufficient: only this
                    // session's verified creation can close as an issue.
                    let created: Option<String> = match resolved.issue_number {
                        Some(number) => tx
                            .query_row(
                                "SELECT repo FROM ledger_outputs WHERE device=?1 AND agent=?2 \
                            AND id=?3 AND kind='issue' AND number=?4 AND sighted_at BETWEEN ?5 AND ?6 LIMIT 1",
                                params![
                                    session.device_id,
                                    session.agent,
                                    session.session_id,
                                    number as i64,
                                    start_at as i64,
                                    *at_unix_ms as i64
                                ],
                                |row| row.get(0),
                            )
                            .optional()
                            .map_err(failed)?,
                        None => None,
                    };
                    let status = if created.is_some() {
                        MissedStatus::Issue
                    } else {
                        MissedStatus::Resolved
                    };
                    // Only an answer closes a question the operator was asked
                    // (B12): the label cannot resolve one in a turn none came in.
                    tx.execute(
                        "UPDATE ledger_missed SET status=?5,issue_repo=?6,issue_number=?7,resolved_issue_number=?9,resolved_turn_offset=?10 \
                        WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND status=?8 AND (kind!=?11 OR ?12)",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            resolved.id,
                            encoded(&status)?,
                            created,
                            created
                                .as_ref()
                                .and(resolved.issue_number)
                                .map(|n| n as i64),
                            encoded(&MissedStatus::Open)?,
                            resolved.issue_number.map(|number| number as i64),
                            *offset as i64,
                            encoded(&MissedKind::Ask)?,
                            *asks_answered
                        ],
                    )
                    .map_err(failed)?;
                }
                cap(&tx, session, "ledger_missed", MISSED_LIMIT)?;
                tx.execute("DELETE FROM ledger_issue_attempts WHERE device=?1 AND agent=?2 AND id=?3 AND state!='creating' AND NOT EXISTS(SELECT 1 FROM ledger_missed m WHERE m.device=ledger_issue_attempts.device AND m.agent=ledger_issue_attempts.agent AND m.id=ledger_issue_attempts.id AND m.missed=ledger_issue_attempts.missed)",params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
                // Turns use their end timestamp for bounded retention.
                tx.execute("DELETE FROM ledger_turns WHERE rowid IN (SELECT rowid FROM ledger_turns \
                    WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY end_at DESC LIMIT -1 OFFSET ?4)",
                    params![session.device_id,session.agent,session.session_id,REQUEST_LIMIT as i64]).map_err(failed)?;
            }
            LedgerUpdate::MissedStatus {
                session,
                id,
                status,
            } => {
                if !matches!(status, MissedStatus::Open | MissedStatus::Dismissed) {
                    return Err("ledger_status_invalid".to_owned());
                }
                let changed=tx.execute("UPDATE ledger_missed SET status=?5,issue_repo=NULL,issue_number=NULL,resolved_issue_number=NULL,resolved_turn_offset=NULL \
                    WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND status!=?6 AND status!=?7 \
                    AND NOT EXISTS(SELECT 1 FROM ledger_issue_attempts a WHERE a.device=?1 AND a.agent=?2 \
                    AND a.id=?3 AND a.missed=?4 AND a.state='creating')",
                    params![session.device_id,session.agent,session.session_id,id,encoded(status)?,encoded(&MissedStatus::Issue)?,encoded(&MissedStatus::Backlog)?]).map_err(failed)?;
                if changed == 0 {
                    return Err("ledger_missed_unavailable".to_owned());
                }
            }
        }
        tx.commit().map_err(failed)
    }

    /// Every saved plan, for the core to hold (PRD hide-plan D-30). A row
    /// this build cannot read is left out with a diagnostic, never guessed.
    pub fn ledger_plans(&self) -> Result<Vec<crate::plans::Plan>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT device,agent,id,plan FROM ledger_plans ORDER BY updated_at")
            .map_err(failed)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        Ok(rows
            .into_iter()
            .filter_map(|(device, agent, id, plan)| {
                match serde_json::from_str::<crate::plans::Plan>(&plan) {
                    Ok(plan)
                        if plan.session.device_id == device
                            && plan.session.agent == agent
                            && plan.session.session_id == id =>
                    {
                        Some(plan)
                    }
                    _ => {
                        crate::diagnostic!(serde_json::json!({"component":"plans","kind":"plans.row_unreadable","session_id":id}));
                        None
                    }
                }
            })
            .collect())
    }

    fn session_plan(
        &self,
        session: &SessionIdentity,
    ) -> Result<Option<crate::plans::Plan>, String> {
        let plan: Option<String> = self
            .connection
            .query_row(
                "SELECT plan FROM ledger_plans WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(failed)?;
        Ok(plan.and_then(|plan| serde_json::from_str(&plan).ok()))
    }

    pub fn ledger_record(&self, session: &SessionIdentity) -> Result<SessionRecord, String> {
        let mut visited = BTreeSet::new();
        let mut budget = REQUEST_LIMIT;
        self.record_tree(session, &mut visited, 0, &mut budget, true)
    }

    fn record_tree(
        &self,
        session: &SessionIdentity,
        visited: &mut BTreeSet<SessionIdentity>,
        depth: usize,
        request_budget: &mut usize,
        include_requests: bool,
    ) -> Result<SessionRecord, String> {
        if depth > LEDGER_DEPTH_LIMIT || visited.len() >= SUMMARY_SESSION_LIMIT {
            return Err("ledger_descendant_capacity".to_owned());
        }
        if !visited.insert(session.clone()) {
            return Err("ledger_parent_cycle".to_owned());
        }
        let row = self
            .session(&key(session))?
            .ok_or("ledger_session_missing")?;
        let args = params![session.device_id, session.agent, session.session_id];
        let mut record = SessionRecord {
            session: session.clone(),
            fallback_title: crate::sidebar::provider_name(Some(&session.agent)),
            cwd: row.cwd,
            path: row.path,
            started_at_unix_ms: row.started_at,
            ended_at_unix_ms: row.ended_at,
            ..SessionRecord::default()
        };
        let origin: Option<(Option<String>, Option<String>)> = self.connection.query_row(
            "SELECT project,repository FROM ledger_session_origins WHERE device=?1 AND agent=?2 AND id=?3",
            args, |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(failed)?;
        if let Some((project, repository)) = origin {
            record.project = project;
            record.repository = repository;
        }
        record.plan = self
            .session_plan(session)?
            .map(|plan| Box::new(crate::plans::view_of(&plan, "")));
        if let Some((native, manual, state, omitted)) = self
            .connection
            .query_row(
                "SELECT native_title,manual_title,state,omitted_requests FROM ledger_titles WHERE device=?1 AND agent=?2 AND id=?3",
                args,
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, u64>(3)?)),
            )
            .optional()
            .map_err(failed)?
        {
            record.native_title = native;
            record.manual_title = manual;
            record.state = state;
            record.omitted_requests = omitted;
        }
        let turns = self.turn_rows(session)?;
        record.status = status_of(&turns);
        let mut statement = self
            .connection
            .prepare(
                "SELECT chapter,goal,at,closed_result FROM ledger_chapters \
            WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at,rowid",
            )
            .map_err(failed)?;
        let chapters: Vec<WorkChapter> = statement
            .query_map(args, |r| {
                Ok(WorkChapter {
                    id: r.get(0)?,
                    goal: r.get(1)?,
                    at_unix_ms: r.get(2)?,
                    closed_result: r.get(3)?,
                })
            })
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)?;
        record.title = chapters
            .last()
            .map(|chapter| chapter.goal.clone())
            .or_else(|| record.native_title.clone())
            .unwrap_or_else(|| crate::sidebar::provider_name(Some(&session.agent)));
        if include_requests {
            let mut statement=self.connection.prepare("SELECT offset,at,text,author,summary,answers,letter_from,images \
            FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,offset DESC LIMIT ?4").map_err(failed)?;
            let rows = statement
                .query_map(
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        *request_budget as i64 + 1
                    ],
                    |r| {
                        Ok((
                            r.get::<_, u64>(0)?,
                            r.get::<_, u64>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, Option<String>>(6)?,
                            r.get::<_, u32>(7)?,
                        ))
                    },
                )
                .map_err(failed)?;
            let mut requests = Vec::new();
            for row in rows {
                if *request_budget == 0 {
                    return Err("ledger_record_request_capacity".into());
                }
                *request_budget -= 1;
                let (offset, at, text, author, summary, answers, letter_from, images) =
                    row.map_err(failed)?;
                requests.push(shown_request(
                    &session.agent,
                    RecordedRequest {
                        offset,
                        at_unix_ms: at,
                        text,
                        author: decoded(author)?,
                        images,
                        summary,
                        answers,
                        letter_from,
                        count: 1,
                        repeats: Vec::new(),
                        goal_command: false,
                    },
                ));
            }
            requests.reverse();
            record.flows = place_flows(&chapters, requests, &turns);
            // The record is named after the flow the operator asked most of,
            // the later one on a tie (D-06, B9); the pane keeps the current
            // flow's name.
            if !chapters.is_empty()
                && let Some(largest) = record.flows.iter().max_by_key(|flow| flow.request_count)
            {
                record.title.clone_from(&largest.name);
            }
        }
        record.missed = self.ledger_missed(session)?;
        let current_start = chapters.last().map(|chapter| chapter.at_unix_ms);
        record.asks = record
            .missed
            .iter()
            .filter(|missed| {
                missed.kind == MissedKind::Ask
                    && missed.status == MissedStatus::Open
                    && waits(missed, current_start)
            })
            .cloned()
            .collect();
        let mut statement = self
            .connection
            .prepare(
                "SELECT o.kind,o.repo,o.number,o.title,o.url,o.state,o.created_at,COALESCE(f.stale,1),f.read_at,o.sighted_at, \
            (SELECT COALESCE(p.merged_at,p.closed_at) FROM prs p WHERE o.kind='pr' AND p.number=o.number \
            AND p.repo=COALESCE((SELECT n.repo FROM repo_names n WHERE n.name=o.repo),o.repo)) \
            FROM ledger_outputs o LEFT JOIN ledger_output_facts f ON f.repo=o.repo AND f.kind=o.kind AND f.number=o.number \
            WHERE o.device=?1 AND o.agent=?2 AND o.id=?3 ORDER BY o.created_at DESC LIMIT 2001",
            )
            .map_err(failed)?;
        record.outputs = statement
            .query_map(args, |r| {
                Ok(WorkOutput {
                    kind: r.get(0)?,
                    repository: r.get(1)?,
                    number: r.get(2)?,
                    title: r.get(3)?,
                    url: r.get(4)?,
                    state: r.get(5)?,
                    created_at_unix_ms: r.get(6)?,
                    stale: r.get(7)?,
                    last_read_at_unix_ms: r.get(8)?,
                    source: session.clone(),
                    source_name: record.title.clone(),
                    sighted_at_unix_ms: r.get(9)?,
                    settled_at_unix_ms: r.get(10)?,
                })
            })
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)?;
        if record.outputs.len() > REQUEST_LIMIT {
            return Err("ledger_output_capacity".into());
        }
        let own_prs: Vec<WorkOutput> = record
            .outputs
            .iter()
            .filter(|output| output.kind == "pr")
            .cloned()
            .collect();
        if !own_prs.is_empty() {
            record.pr_groups.push(PrGroup {
                child: None,
                name: record.title.clone(),
                outputs: own_prs,
            });
        }
        record.artifacts = turns
            .iter()
            .rev()
            .flat_map(|turn| turn.artifacts.iter().cloned())
            .collect();
        // A child is part of the record once the ledger records it: a spawn
        // is known before its file is read, and a Pi or omp child's edge is
        // under its file until the reader names it.
        let mut statement = self
            .connection
            .prepare(
                "SELECT p.device,p.agent,p.id FROM session_parents p JOIN sessions s \
            ON s.device=p.device AND s.agent=p.agent AND s.id=p.id \
            WHERE p.parent_device=?1 AND p.parent_agent=?2 AND p.parent_id=?3 \
            ORDER BY s.started_at,p.device,p.agent,p.id LIMIT ?4",
            )
            .map_err(failed)?;
        let children = statement
            .query_map(
                params![
                    session.device_id,
                    session.agent,
                    session.session_id,
                    SUMMARY_SESSION_LIMIT as i64
                ],
                |r| {
                    Ok(SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    })
                },
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        for child in children {
            let child =
                self.record_tree(&child, visited, depth + 1, request_budget, include_requests)?;
            // A grandchild's pull requests sit under the direct child that
            // started it (B18).
            let prs: Vec<WorkOutput> = child
                .outputs
                .iter()
                .filter(|output| output.kind == "pr")
                .cloned()
                .collect();
            if !prs.is_empty() {
                record.pr_groups.push(PrGroup {
                    child: Some(child.session.clone()),
                    name: child.title.clone(),
                    outputs: prs,
                });
            }
            record.outputs.extend(child.outputs.iter().cloned());
            record.artifacts.extend(child.artifacts.iter().cloned());
            if record.outputs.len() > REQUEST_LIMIT {
                return Err("ledger_output_capacity".into());
            }
            record.children.push(child);
        }
        let mut outputs_seen = BTreeSet::new();
        record.outputs.retain(|output| {
            outputs_seen.insert((
                output.kind.clone(),
                output.repository.clone(),
                output.number,
            ))
        });
        let mut groups_seen = BTreeSet::new();
        for group in &mut record.pr_groups {
            group
                .outputs
                .retain(|output| groups_seen.insert((output.repository.clone(), output.number)));
        }
        record.pr_groups.retain(|group| !group.outputs.is_empty());
        let mut artifacts_seen = BTreeSet::new();
        record
            .artifacts
            .retain(|artifact| artifacts_seen.insert(artifact.clone()));
        record.artifacts.truncate(ARTIFACT_LIMIT);
        settle(&mut record);
        Ok(record)
    }

    /// The session's judged turns, oldest first.
    fn turn_rows(&self, session: &SessionIdentity) -> Result<Vec<TurnRow>, String> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT start_at,end_at,end,line,artifacts FROM ledger_turns \
                 WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY end_at,offset",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map(
                params![session.device_id, session.agent, session.session_id],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        rows.into_iter()
            .map(|(start_at, end_at, end, line, artifacts)| {
                Ok(TurnRow {
                    start_at,
                    end_at,
                    end: decoded(end)?,
                    line,
                    artifacts: decoded(artifacts)?,
                })
            })
            .collect()
    }

    fn ledger_missed(&self, session: &SessionIdentity) -> Result<Vec<MissedWork>, String> {
        ledger_missed_rows(&self.connection, session)
    }

    /// When the session's current flow opened, if it has flows.
    fn current_flow_start(&self, session: &SessionIdentity) -> Result<Option<u64>, String> {
        self.connection
            .query_row(
                "SELECT MAX(at) FROM ledger_chapters WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id],
                |r| r.get(0),
            )
            .map_err(failed)
    }

    /// What a later turn of `session` needs from the ledger: who delegated
    /// it, its open missed lines and the requests still owed a summary. A
    /// session the ledger has not recorded, one whose file no read has
    /// reached yet or one retention removed, has only the parent that
    /// started it.
    pub fn ledger_context(&self, session: &SessionIdentity) -> Result<LedgerContext, String> {
        let parent_name = self
            .connection
            .query_row(
                "SELECT parent_name FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(failed)?;
        if self.session(&key(session))?.is_none() {
            return Ok(LedgerContext {
                parent_name,
                ..LedgerContext::default()
            });
        }
        let since: u64 = self
            .meta("summaries_owed_since")?
            .ok_or("ledger_summaries_since_missing")?
            .parse()
            .map_err(|_| "ledger_summaries_since_invalid")?;
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT offset,at,text FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 \
                 AND summary IS NULL AND letter_from IS NULL AND at>=?4 ORDER BY at,offset LIMIT ?5",
            )
            .map_err(failed)?;
        let owed = statement
            .query_map(
                params![
                    session.device_id,
                    session.agent,
                    session.session_id,
                    since as i64,
                    OWED_IN_CONTEXT as i64
                ],
                |r| {
                    Ok(OwedRequest {
                        offset: r.get(0)?,
                        at_unix_ms: r.get(1)?,
                        text: r.get(2)?,
                    })
                },
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let newest_offset = self
            .connection
            .query_row(
                "SELECT MAX(offset) FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3",
                params![session.device_id, session.agent, session.session_id],
                |r| r.get(0),
            )
            .map_err(failed)?;
        Ok(LedgerContext {
            parent_name,
            open_missed: {
                let since = self.current_flow_start(session)?;
                self.ledger_missed(session)?
                    .into_iter()
                    .filter(|missed| missed.status == MissedStatus::Open && waits(missed, since))
                    .collect()
            },
            owed,
            newest_offset,
            artifacts: self.recorded_artifacts(session)?,
        })
    }

    fn recorded_artifacts(&self, session: &SessionIdentity) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare_cached(
                "SELECT artifacts FROM ledger_turns WHERE device=?1 AND agent=?2 AND id=?3 \
                 AND artifacts!='[]' ORDER BY end_at DESC LIMIT ?4",
            )
            .map_err(failed)?;
        let rows = statement
            .query_map(
                params![
                    session.device_id,
                    session.agent,
                    session.session_id,
                    ARTIFACTS_IN_CONTEXT as i64
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut seen = BTreeSet::new();
        let mut artifacts = Vec::new();
        for row in rows {
            for artifact in decoded::<Vec<String>>(row)? {
                if seen.insert(artifact.clone()) {
                    artifacts.push(artifact);
                }
            }
        }
        artifacts.truncate(ARTIFACTS_IN_CONTEXT);
        Ok(artifacts)
    }

    /// Closed issues also need creation facts; feed sightings to the existing
    /// bounded GitHub reader, never fetch from record/search code.
    ///
    /// A sighting is the project's only when its session works inside the
    /// project and its issue is in the project's repository, the one GitHub
    /// answered for the project's checkout: the same two facts that join a
    /// session's pull requests to a project (`project_paths`). An address an
    /// agent only printed, such as a test fixture's `acme/repo`, is no issue
    /// of the project's, and one that does not resolve once failed every issue
    /// read of the project. A project whose repository is not known yet has
    /// none.
    pub fn ledger_issue_refs(
        &self,
        project: &str,
    ) -> Result<Vec<crate::issues::IssueReference>, String> {
        let pass = self.pass(project)?.ok_or("ledger_project_missing")?;
        self.ledger_issue_refs_in(&pass)
    }

    /// [`LinkStore::ledger_issue_refs`] within a publish pass for one project.
    pub(in crate::links) fn ledger_issue_refs_in(
        &self,
        pass: &ProjectPass,
    ) -> Result<Vec<crate::issues::IssueReference>, String> {
        let project = &pass.project;
        let Some(repo) = project.repo.as_deref() else {
            return Ok(Vec::new());
        };
        let paths = &pass.paths;
        let mut statement = self.connection.prepare("SELECT DISTINCT x.device,x.agent,x.id,x.repo,x.number FROM ledger_sightings x LEFT JOIN repo_names r ON r.name=x.repo WHERE x.kind='issue' AND COALESCE(r.repo,x.repo)=?2 ORDER BY x.at DESC LIMIT ?1").map_err(failed)?;
        let rows = statement
            .query_map(params![SUMMARY_SESSION_LIMIT as i64 + 1, repo], |r| {
                Ok((
                    SessionIdentity {
                        device_id: r.get(0)?,
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        if rows.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_sighting_capacity".into());
        }
        let mut refs = BTreeSet::new();
        for (session, repo, number) in rows {
            if self
                .session(&key(&session))?
                .is_some_and(|row| row.inside(paths))
            {
                let reference = crate::issues::IssueReference::parse(
                    &format!("https://github.com/{repo}/issues/{number}"),
                    None,
                )
                .map_err(|_| "ledger_issue_reference_invalid")?;
                refs.insert(reference);
            }
        }
        if refs.len() > crate::issues::ISSUE_LIMIT {
            if self.issue_capacity.borrow_mut().enter(&pass.project.key) {
                crate::diagnostic!(
                    serde_json::json!({"component":"links","kind":"ledger.issue_read_capacity","limit":crate::issues::ISSUE_LIMIT,"omitted":refs.len()-crate::issues::ISSUE_LIMIT})
                );
            }
        } else {
            self.issue_capacity.borrow_mut().leave(&pass.project.key);
        }
        Ok(refs.into_iter().take(crate::issues::ISSUE_LIMIT).collect())
    }

    pub fn ledger_summaries(&self, project: &str) -> Result<Vec<SessionWorkSummary>, String> {
        let pass = self.pass(project)?.ok_or("ledger_project_missing")?;
        self.ledger_summaries_in(&pass)
    }

    /// [`LinkStore::ledger_summaries`] within a publish pass for one project.
    pub(in crate::links) fn ledger_summaries_in(
        &self,
        pass: &ProjectPass,
    ) -> Result<Vec<SessionWorkSummary>, String> {
        let paths = &pass.paths;
        // A summary never loads request bodies or record trees. It reads
        // bounded counts once, then rolls each count along stored lineage.
        // An ask waits on the operator in its session's record and is no
        // missed work of the backlog's (D-12, D-13).
        let mut statement = self
            .connection
            .prepare(
                "SELECT device,agent,id,count(*) FROM ledger_missed \
            WHERE status=?1 AND kind!=?3 GROUP BY device,agent,id LIMIT ?2",
            )
            .map_err(failed)?;
        let counts = statement
            .query_map(
                params![
                    encoded(&MissedStatus::Open)?,
                    SUMMARY_SESSION_LIMIT as i64 + 1,
                    encoded(&MissedKind::Ask)?
                ],
                |r| {
                    Ok((
                        SessionIdentity {
                            device_id: r.get(0)?,
                            agent: r.get(1)?,
                            session_id: r.get(2)?,
                        },
                        r.get::<_, usize>(3)?,
                    ))
                },
            )
            .map_err(failed)?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(failed)?;
        if counts.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_summary_capacity".into());
        }
        let mut statement = self
            .connection
            .prepare("SELECT device,agent,id,parent_agent,parent_id,parent_device FROM session_parents LIMIT ?1")
            .map_err(failed)?;
        let parents = statement
            .query_map([SUMMARY_SESSION_LIMIT as i64 + 1], |r| {
                let device: String = r.get(0)?;
                Ok((
                    SessionIdentity {
                        device_id: device.clone(),
                        agent: r.get(1)?,
                        session_id: r.get(2)?,
                    },
                    SessionIdentity {
                        device_id: r.get(5)?,
                        agent: r.get(3)?,
                        session_id: r.get(4)?,
                    },
                ))
            })
            .map_err(failed)?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(failed)?;
        if parents.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_summary_capacity".into());
        }
        let mut subtree = counts.clone();
        for (child, count) in &counts {
            let mut held = child;
            let mut visited = BTreeSet::new();
            while let Some(parent) = parents.get(held) {
                if visited.len() >= 32 || !visited.insert(parent.clone()) {
                    return Err("ledger_parent_cycle".into());
                }
                *subtree.entry(parent.clone()).or_default() += count;
                held = parent;
            }
        }
        // The open asks by session, for the Project tool's 할 일 (PRD
        // lane-two-columns B34): one bounded grouped read, like the counts.
        let asks = self
            .connection
            .prepare(
                "SELECT device,agent,id,count(*) FROM ledger_missed \
            WHERE status=?1 AND kind=?3 GROUP BY device,agent,id LIMIT ?2",
            )
            .map_err(failed)?
            .query_map(
                params![
                    encoded(&MissedStatus::Open)?,
                    SUMMARY_SESSION_LIMIT as i64 + 1,
                    encoded(&MissedKind::Ask)?
                ],
                |r| {
                    Ok((
                        SessionIdentity {
                            device_id: r.get(0)?,
                            agent: r.get(1)?,
                            session_id: r.get(2)?,
                        },
                        r.get::<_, i64>(3)? as usize,
                    ))
                },
            )
            .map_err(failed)?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(failed)?;
        if asks.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_summary_capacity".into());
        }
        let sessions = self
            .all_sessions()?
            .into_iter()
            .filter(|session| session.inside(paths))
            .take(SUMMARY_SESSION_LIMIT + 1)
            .collect::<Vec<_>>();
        if sessions.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_summary_capacity".into());
        }
        let mut statement=self.connection.prepare("SELECT \
            (SELECT goal FROM ledger_chapters WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC LIMIT 1), \
            (SELECT native_title FROM ledger_titles WHERE device=?1 AND agent=?2 AND id=?3), \
            (SELECT manual_title FROM ledger_titles WHERE device=?1 AND agent=?2 AND id=?3)").map_err(failed)?;
        let mut summaries = Vec::new();
        let mut missed_count = 0;
        for session in sessions {
            let identity = identity(&session.key);
            let (goal, native, manual) = statement
                .query_row(
                    params![identity.device_id, identity.agent, identity.session_id],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .map_err(failed)?;
            let missed = if counts.get(&identity).copied().unwrap_or(0) > 0 {
                self.ledger_missed(&identity)?
                    .into_iter()
                    .filter(|missed| {
                        missed.status == MissedStatus::Open && missed.kind != MissedKind::Ask
                    })
                    .collect()
            } else {
                Vec::new()
            };
            missed_count += missed.len();
            if missed_count > SUMMARY_SESSION_LIMIT {
                return Err("ledger_summary_capacity".into());
            }
            summaries.push(SessionWorkSummary {
                fallback_title: crate::sidebar::provider_name(Some(&identity.agent)),
                title: goal
                    .or_else(|| native.clone())
                    .unwrap_or_else(|| crate::sidebar::provider_name(Some(&identity.agent))),
                native_title: native,
                manual_title: manual,
                cwd: session.cwd,
                ended_at_unix_ms: session.ended_at,
                open_missed: counts.get(&identity).copied().unwrap_or(0),
                subtree_open_missed: subtree.get(&identity).copied().unwrap_or(0),
                missed,
                open_asks: asks.get(&identity).copied().unwrap_or(0),
                session: identity,
            });
        }
        Ok(summaries)
    }

    #[cfg(test)]
    pub fn ledger_search(
        &self,
        query: &str,
        summaries: bool,
    ) -> Result<Vec<SessionSearchHit>, String> {
        self.ledger_search_page(
            query,
            &SessionSearchOptions {
                include_flows: summaries,
                ..Default::default()
            },
            summaries,
            &[],
        )
        .map(|page| page.results)
    }

    /// The retained ledger only. Every scope predicate precedes ranking and
    /// the page limit; the cursor includes the whole native session identity.
    pub fn ledger_search_page(
        &self,
        query: &str,
        options: &SessionSearchOptions,
        summaries: bool,
        presence: &[SessionSearchPresence],
    ) -> Result<SessionSearchPage, String> {
        let query = query.trim();
        if query.chars().count() > 200 || !options.bounded() {
            return Err("ledger_query_capacity".to_owned());
        }
        if presence.len() > SUMMARY_SESSION_LIMIT {
            return Err("ledger_search_presence_capacity".into());
        }
        if options
            .since_unix_ms
            .zip(options.until_unix_ms)
            .is_some_and(|(since, until)| since > until)
            || options.since_unix_ms.is_some_and(|at| at > i64::MAX as u64)
            || options.until_unix_ms.is_some_and(|at| at > i64::MAX as u64)
            || options
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.at_unix_ms > i64::MAX as u64)
        {
            return Err("ledger_search_range_invalid".into());
        }
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut values = "SELECT r.device,r.agent,r.id,r.text,r.at,'request' AS kind,r.offset FROM ledger_requests r JOIN scoped s USING(device,agent,id) WHERE r.author='\"operator\"' AND r.letter_from IS NULL".to_owned();
        // A flow's name is matched where a task's was (B32).
        if options.include_flows && summaries {
            values.push_str(" UNION ALL SELECT r.device,r.agent,r.id,r.goal,r.at,'flow',NULL FROM ledger_chapters r JOIN scoped s USING(device,agent,id)");
        }
        if options.include_titles {
            values.push_str(" UNION ALL SELECT t.device,t.agent,t.id,t.manual_title,COALESCE(s.last_request_at,s.ended_at,s.started_at,0),'title',NULL FROM ledger_titles t JOIN scoped s USING(device,agent,id) WHERE t.manual_title IS NOT NULL UNION ALL SELECT t.device,t.agent,t.id,t.native_title,COALESCE(s.last_request_at,s.ended_at,s.started_at,0),'title',NULL FROM ledger_titles t JOIN scoped s USING(device,agent,id) WHERE t.native_title IS NOT NULL AND t.native_title IS NOT t.manual_title");
        }
        let goal = if summaries {
            "(SELECT goal FROM ledger_chapters c WHERE c.device=s.device AND c.agent=s.agent AND c.id=s.id ORDER BY at DESC LIMIT 1)"
        } else {
            "NULL"
        };
        let sql = format!(
            r#"
            WITH raw_presence AS (
                SELECT json_extract(value,'$.session.device_id') device,json_extract(value,'$.session.agent') agent,
                       json_extract(value,'$.session.session_id') id,json_extract(value,'$.status') status,CAST(key AS INTEGER) priority FROM json_each(?9)
            ), named_presence AS (
                SELECT p.device,p.agent,COALESCE((SELECT session FROM cursors c WHERE c.device=p.device AND c.agent=p.agent AND c.path=p.id AND c.session IS NOT NULL),p.id) id,p.status,p.priority FROM raw_presence p
            ), presence AS (
                SELECT device,agent,id,status FROM (SELECT *,row_number() OVER(PARTITION BY device,agent,id ORDER BY CASE WHEN status='archived' THEN 0 ELSE 1 END DESC,priority DESC) rank FROM named_presence) WHERE rank=1
            ), scoped AS MATERIALIZED (
                SELECT s.*,o.project,COALESCE(p.status,'retained') status FROM sessions s
                LEFT JOIN ledger_session_origins o USING(device,agent,id)
                LEFT JOIN presence p USING(device,agent,id)
                WHERE (?3 IS NULL OR s.device=?3) AND (?4 IS NULL OR o.project=?4)
                  AND (?5 IS NULL OR s.agent=?5)
                  AND (?8 IS NULL OR COALESCE(p.status,'retained')=?8
                    OR (?8='current' AND p.status IS NOT NULL AND p.status!='archived'))
            ), matched AS MATERIALIZED (
                SELECT * FROM ({values}) WHERE (?2 OR text LIKE ?1 ESCAPE '\')
                  AND (?2 OR ((?6 IS NULL OR at>=?6) AND (?7 IS NULL OR at<=?7)))
            ), ranked AS MATERIALIZED (
                SELECT *,row_number() OVER(PARTITION BY device,agent,id ORDER BY at DESC,kind,offset DESC,text) rank FROM matched
            ), totals AS (
                SELECT device,agent,id,MAX(at) at,COUNT(*) count FROM matched GROUP BY device,agent,id
            ), candidates AS (
                SELECT s.device,s.agent,s.id,COALESCE(t.manual_title,{goal},t.native_title) title,
                    s.cwd,s.project,s.status,
                    CASE WHEN ?2 THEN MAX(COALESCE(s.last_request_at,0),COALESCE(s.ended_at,0),COALESCE(s.started_at,0),COALESCE(m.at,0)) ELSE m.at END activity,
                    CASE WHEN ?2 THEN 0 ELSE COALESCE(m.count,0) END match_count,
                    (SELECT COUNT(*) FROM ledger_requests r WHERE r.device=s.device AND r.agent=s.agent AND r.id=s.id AND r.author='"operator"' AND r.letter_from IS NULL) request_count
                FROM scoped s LEFT JOIN totals m USING(device,agent,id) LEFT JOIN ledger_titles t USING(device,agent,id)
                WHERE ?2 OR m.count>0
            ) SELECT c.*,
                (SELECT json_group_array(json_object('kind',kind,'text',text,'at_unix_ms',at,'request_offset',offset))
                 FROM (SELECT kind,text,at,offset FROM ranked r WHERE r.device=c.device AND r.agent=c.agent AND r.id=c.id AND rank<=2 ORDER BY rank)) matches
            FROM candidates c WHERE (?6 IS NULL OR activity>=?6) AND (?7 IS NULL OR activity<=?7)
                AND (?10 IS NULL OR activity<?10 OR (activity=?10 AND (device,agent,id)>(?11,?12,?13)))
            ORDER BY activity DESC,device,agent,id LIMIT ?14
        "#
        );
        let status = options.status.map(|status| match status {
            SessionSearchStatus::Current => "current",
            SessionSearchStatus::Working => "working",
            SessionSearchStatus::Done => "done",
            SessionSearchStatus::NeedsYou => "needs_you",
            SessionSearchStatus::Archived => "archived",
            SessionSearchStatus::Retained => "retained",
        });
        let cursor = options.cursor.as_ref();
        let mut statement = self.connection.prepare(&sql).map_err(failed)?;
        let rows = statement
            .query_map(
                params![
                    pattern,
                    query.is_empty(),
                    options.device_id,
                    options.project_root,
                    options.agent,
                    options.since_unix_ms.map(|n| n as i64),
                    options.until_unix_ms.map(|n| n as i64),
                    status,
                    encoded(&presence)?,
                    cursor.map(|c| c.at_unix_ms as i64),
                    cursor.map(|c| &c.device_id),
                    cursor.map(|c| &c.agent),
                    cursor.map(|c| &c.session_id),
                    SEARCH_LIMIT as i64 + 1
                ],
                |r| {
                    let agent: String = r.get(1)?;
                    let title = r
                        .get::<_, Option<String>>(3)?
                        .unwrap_or_else(|| crate::sidebar::provider_name(Some(&agent)));
                    Ok((
                        SessionSearchHit {
                            session: SessionIdentity {
                                device_id: r.get(0)?,
                                agent,
                                session_id: r.get(2)?,
                            },
                            title,
                            matched: String::new(),
                            at_unix_ms: r.get(7)?,
                            cwd: r.get(4)?,
                            project_root: r.get(5)?,
                            status: r.get(6)?,
                            matches: Vec::new(),
                            match_count: r.get(8)?,
                            request_count: r.get(9)?,
                        },
                        r.get::<_, String>(10)?,
                    ))
                },
            )
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let mut results = rows
            .into_iter()
            .map(|(mut hit, matches)| {
                hit.matches = serde_json::from_str(&matches)
                    .map_err(|_| "ledger_search_result_invalid".to_owned())?;
                hit.matched = hit
                    .matches
                    .first()
                    .map(|m| m.text.clone())
                    .unwrap_or_default();
                Ok(hit)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let has_more = results.len() > SEARCH_LIMIT;
        results.truncate(SEARCH_LIMIT);
        let next_cursor = results
            .last()
            .filter(|_| has_more)
            .map(|hit| SessionSearchCursor {
                at_unix_ms: hit.at_unix_ms,
                device_id: hit.session.device_id.clone(),
                agent: hit.session.agent.clone(),
                session_id: hit.session.session_id.clone(),
            });
        Ok(SessionSearchPage {
            results,
            next_cursor,
        })
    }

    pub fn ledger_creators(
        &self,
        work: &[SessionWorkSummary],
    ) -> Result<Vec<OutputCreator>, String> {
        let sessions = encoded(
            &work
                .iter()
                .map(|record| &record.session)
                .collect::<Vec<_>>(),
        )?;
        let mut statement = self.connection.prepare("SELECT o.device,o.agent,o.id,o.kind,o.repo,o.number FROM ledger_outputs o \
            JOIN json_each(?1) s ON o.device=json_extract(s.value,'$.device_id') AND o.agent=json_extract(s.value,'$.agent') AND o.id=json_extract(s.value,'$.session_id') \
            ORDER BY o.created_at DESC,o.kind,o.number LIMIT ?2").map_err(failed)?;
        let creators = statement
            .query_map(params![sessions, REQUEST_LIMIT as i64 + 1], |row| {
                Ok(OutputCreator {
                    source: SessionIdentity {
                        device_id: row.get(0)?,
                        agent: row.get(1)?,
                        session_id: row.get(2)?,
                    },
                    kind: row.get(3)?,
                    repository: row.get(4)?,
                    number: row.get(5)?,
                })
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        if creators.len() > REQUEST_LIMIT {
            return Err("ledger_creator_capacity".into());
        }
        Ok(creators)
    }

    /// Apply verified PR creation sightings after each project refresh.
    pub fn refresh_ledger_outputs(&mut self) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(failed)?;
        refresh_outputs(&tx)?;
        tx.commit().map_err(failed)
    }
}

/// Either side of the creation join can arrive last: the repository's
/// creation fact or a newly read tool sighting. Commit its derived output
/// with the read rather than waiting for another repository refresh.
pub(super) fn refresh_outputs(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute("INSERT INTO ledger_outputs(device,agent,id,kind,repo,number,title,url,state,created_at,sighted_at) \
            SELECT s.device,s.agent,s.id,'pr',s.repo_name,p.number,p.title,p.url,CASE WHEN p.merged_at IS NOT NULL THEN 'merged' \
            WHEN p.closed_at IS NOT NULL THEN 'closed' ELSE 'open' END,p.created_at,s.at \
            FROM session_prs s JOIN repo_names n ON n.name=s.repo_name JOIN prs p ON p.repo=n.repo AND p.number=s.number \
            WHERE p.created_at IS NOT NULL AND s.at>=p.created_at-?1 AND s.at<=p.created_at+?2 \
            ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET title=excluded.title,url=excluded.url,state=excluded.state",
            params![CREATED_BEFORE_MS as i64,CREATED_AFTER_MS as i64]).map_err(failed)?;
    // The label judged these pull requests made by the session against
    // GitHub's creation time (`SessionFacts::judge_created`), so a sighting
    // the link reader's window missed still finds its session (D-14, B21).
    tx.execute("INSERT INTO ledger_outputs(device,agent,id,kind,repo,number,title,url,state,created_at,sighted_at) \
            SELECT s.device,s.agent,s.id,'pr',s.repo,p.number,p.title,p.url,CASE WHEN p.merged_at IS NOT NULL THEN 'merged' \
            WHEN p.closed_at IS NOT NULL THEN 'closed' ELSE 'open' END,p.created_at,s.at \
            FROM ledger_sightings s JOIN repo_names n ON n.name=s.repo JOIN prs p ON p.repo=n.repo AND p.number=s.number \
            WHERE s.kind='pr' AND p.created_at IS NOT NULL \
            ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET title=excluded.title,url=excluded.url,state=excluded.state",
            []).map_err(failed)?;
    tx.execute("INSERT INTO ledger_outputs(device,agent,id,kind,repo,number,title,url,state,created_at,sighted_at) \
            SELECT s.device,s.agent,s.id,s.kind,s.repo,s.number,f.title,f.url,f.state,f.created_at,s.at \
            FROM ledger_sightings s JOIN ledger_output_facts f ON f.repo=s.repo AND f.kind=s.kind AND f.number=s.number \
            WHERE f.created_at IS NOT NULL AND s.at>=f.created_at-?1 AND s.at<=f.created_at+?2 \
            ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET title=excluded.title,url=excluded.url,state=excluded.state",
            params![CREATED_BEFORE_MS as i64,CREATED_AFTER_MS as i64]).map_err(failed)?;
    tx.execute("UPDATE ledger_outputs SET state=COALESCE((SELECT f.state FROM ledger_output_facts f WHERE f.kind=ledger_outputs.kind AND f.number=ledger_outputs.number AND (f.repo=ledger_outputs.repo OR f.repo=(SELECT name FROM repo_names n WHERE n.repo=ledger_outputs.repo LIMIT 1))),state)",[]).map_err(failed)?;
    // GitHub may arrive after END. Retain the proposed number and the
    // resolving turn, then attach only that turn's verified creation.
    tx.execute("UPDATE ledger_missed AS m SET status=?1,issue_number=resolved_issue_number,issue_repo=(SELECT repo FROM ledger_outputs o JOIN ledger_turns t ON t.device=o.device AND t.agent=o.agent AND t.id=o.id AND t.offset=m.resolved_turn_offset WHERE o.device=m.device AND o.agent=m.agent AND o.id=m.id AND o.kind='issue' AND o.number=m.resolved_issue_number AND o.sighted_at BETWEEN t.start_at AND t.end_at LIMIT 1) WHERE m.status=?2 AND m.resolved_issue_number IS NOT NULL AND EXISTS(SELECT 1 FROM ledger_outputs o JOIN ledger_turns t ON t.device=o.device AND t.agent=o.agent AND t.id=o.id AND t.offset=m.resolved_turn_offset WHERE o.device=m.device AND o.agent=m.agent AND o.id=m.id AND o.kind='issue' AND o.number=m.resolved_issue_number AND o.sighted_at BETWEEN t.start_at AND t.end_at)",params![encoded(&MissedStatus::Issue)?,encoded(&MissedStatus::Resolved)?]).map_err(failed)?;
    Ok(())
}

/// One judged turn as the record reads it.
struct TurnRow {
    start_at: u64,
    end_at: u64,
    end: LabelEnd,
    line: String,
    artifacts: Vec<String>,
}

impl TurnRow {
    fn outcome(&self) -> TurnOutcome {
        TurnOutcome {
            end: self.end,
            line: self.line.clone(),
            at_unix_ms: self.end_at,
        }
    }
}

/// The top state (D-10, B10): the newest turn a usage limit did not stop,
/// with the limit's line as a badge when the newest turn was stopped by one;
/// a session every turn of which was stopped reads as blocked.
fn status_of(turns: &[TurnRow]) -> Option<RecordStatus> {
    let newest = turns.last()?;
    let limited = (newest.end == LabelEnd::Limited).then(|| newest.line.clone());
    match turns
        .iter()
        .rev()
        .find(|turn| turn.end != LabelEnd::Limited)
    {
        Some(turn) => Some(RecordStatus {
            end: turn.end,
            line: turn.line.clone(),
            at_unix_ms: turn.end_at,
            limited,
            settled: None,
            started_at_unix_ms: turn.start_at,
        }),
        None => Some(RecordStatus {
            end: LabelEnd::Blocked,
            line: String::new(),
            at_unix_ms: newest.end_at,
            limited,
            settled: None,
            started_at_unix_ms: newest.start_at,
        }),
    }
}

/// Whether an open question still waits on the operator: one asked in the
/// current flow, since moving to the next flow answers the ones before it
/// (PRD session-record-fact-status D-03, B6).
fn waits(missed: &MissedWork, current_start: Option<u64>) -> bool {
    missed.kind != MissedKind::Ask || current_start.is_none_or(|start| missed.at_unix_ms >= start)
}

/// Hide's facts over the labels' lines (PRD session-record-fact-status
/// D-01, B1-B3): a flow whose pull requests all merged or closed after its
/// result reads as that. The top state does the same with the pull requests
/// of the flow its turn belongs to that were not settled when it ended, a
/// child's printed in that flow among them; a turn that asks the operator, is blocked or
/// hit a limit stays on top.
fn settle(record: &mut SessionRecord) {
    if record.flows.is_empty() {
        return;
    }
    let starts: Vec<u64> = record.flows.iter().map(|flow| flow.at_unix_ms).collect();
    let mut made: Vec<Vec<&WorkOutput>> = vec![Vec::new(); starts.len()];
    for output in record.outputs.iter().filter(|output| output.kind == "pr") {
        let index = starts
            .iter()
            .rposition(|start| *start <= output.sighted_at_unix_ms)
            .unwrap_or(0);
        made[index].push(output);
    }
    let settlements: Vec<Option<PrSettlement>> = made.iter().map(|prs| settlement(prs)).collect();
    for (flow, settled) in record.flows.iter_mut().zip(&settlements) {
        let Some(settled) = settled else { continue };
        let line_at = match &flow.result {
            Some(FlowResult::Turn(turn)) => Some(turn.at_unix_ms),
            Some(
                FlowResult::Closing { at_unix_ms, .. } | FlowResult::MovedOn { at_unix_ms, .. },
            ) => Some(*at_unix_ms),
            Some(FlowResult::Settled(_)) | None => None,
        };
        if line_at.is_none_or(|at| settled.at_unix_ms > at) {
            flow.result = Some(FlowResult::Settled(settled.clone()));
        }
    }
    if let Some(status) = record.status.as_mut()
        && status.limited.is_none()
        && !matches!(status.end, LabelEnd::Question | LabelEnd::Blocked)
    {
        let flow = starts
            .iter()
            .rposition(|start| *start <= status.started_at_unix_ms)
            .unwrap_or(0);
        let pending: Vec<&WorkOutput> = made[flow]
            .iter()
            .copied()
            .filter(|output| {
                output
                    .settled_at_unix_ms
                    .is_none_or(|at| at > status.at_unix_ms)
            })
            .collect();
        status.settled = settlement(&pending);
    }
}

/// The settlement of a flow's pull requests, when every one of them is
/// merged or closed at a known time; one still open, or a time the link
/// reader has not read, settles nothing.
fn settlement(prs: &[&WorkOutput]) -> Option<PrSettlement> {
    if prs.is_empty() {
        return None;
    }
    let mut merged = Vec::new();
    let mut closed = Vec::new();
    let mut at = 0;
    for pr in prs {
        let settled_at = pr.settled_at_unix_ms?;
        match pr.state.as_str() {
            "merged" => merged.push(pr.number),
            "closed" => closed.push(pr.number),
            _ => return None,
        }
        at = at.max(settled_at);
    }
    let (state, mut numbers) = if merged.is_empty() {
        (SettledState::Closed, closed)
    } else {
        (SettledState::Merged, merged)
    };
    numbers.sort_unstable();
    numbers.dedup();
    Some(PrSettlement {
        state,
        numbers,
        at_unix_ms: at,
    })
}

/// A request's words as the record shows them (D-18, B15, B25): attachment
/// paths become a count, and a `/goal` command keeps its words and a mark,
/// in the agents that have the command (Claude Code, Codex).
fn shown_request(agent: &str, mut request: RecordedRequest) -> RecordedRequest {
    let (text, found) = without_attachments(&request.text);
    if found > 0 {
        request.text = text;
        request.images = request.images.max(found);
    }
    if matches!(agent, "claude" | "codex")
        && let Some(rest) = request.text.trim_start().strip_prefix("/goal")
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        request.goal_command = true;
        request.text = rest.trim().to_owned();
    }
    request
}

/// The same words from the same sender again within [`REPEAT_WINDOW_MS`]
/// read as one line with a count (D-18, B23).
fn merge_repeats(requests: Vec<RecordedRequest>) -> Vec<RecordedRequest> {
    let mut kept: Vec<(RecordedRequest, u64)> = Vec::with_capacity(requests.len());
    for request in requests {
        if let Some((held, last_at)) = kept.last_mut()
            && held.text == request.text
            && held.images == request.images
            && held.author == request.author
            && held.letter_from == request.letter_from
            && held.goal_command == request.goal_command
            && request.at_unix_ms.saturating_sub(*last_at) <= REPEAT_WINDOW_MS
        {
            held.count += 1;
            held.repeats.push(request.offset);
            *last_at = request.at_unix_ms;
            if held.summary.is_none() {
                held.summary = request.summary;
                held.answers = request.answers;
            }
            continue;
        }
        let at = request.at_unix_ms;
        kept.push((request, at));
    }
    kept.into_iter().map(|(request, _)| request).collect()
}

/// The longest name a flow takes from its first request.
const FLOW_NAME_CHARS: usize = 40;

/// Requests and turns placed by time into the goal flows (D-05, B4, B13):
/// a flow holds what arrived from its start until the next flow's, and the
/// first holds what came before any goal too. With no goal, one flow is
/// named after its first request (B17).
fn place_flows(
    chapters: &[WorkChapter],
    requests: Vec<RecordedRequest>,
    turns: &[TurnRow],
) -> Vec<WorkFlow> {
    let starts: Vec<(String, Option<String>, u64, Option<String>)> = if chapters.is_empty() {
        let Some(first) = requests.first() else {
            return Vec::new();
        };
        vec![("flow-start".to_owned(), None, first.at_unix_ms, None)]
    } else {
        chapters
            .iter()
            .map(|chapter| {
                (
                    chapter.id.clone(),
                    Some(chapter.goal.clone()),
                    chapter.at_unix_ms,
                    chapter.closed_result.clone(),
                )
            })
            .collect()
    };
    let flow_of = |at: u64| {
        starts
            .iter()
            .rposition(|(_, _, start, _)| *start <= at)
            .unwrap_or(0)
    };
    let next_starts: Vec<Option<u64>> = (0..starts.len())
        .map(|index| starts.get(index + 1).map(|(_, _, at, _)| *at))
        .collect();
    let mut placed: Vec<Vec<RecordedRequest>> = vec![Vec::new(); starts.len()];
    for request in requests {
        placed[flow_of(request.at_unix_ms)].push(request);
    }
    let mut flow_turns: Vec<Vec<&TurnRow>> = vec![Vec::new(); starts.len()];
    for turn in turns {
        flow_turns[flow_of(turn.start_at)].push(turn);
    }
    starts
        .into_iter()
        .zip(placed)
        .zip(flow_turns)
        .zip(next_starts)
        .map(|((((id, goal, at, closed), requests), turns), next)| {
            // Repeats merge within a flow, so the message that opened one is
            // never folded into the flow before it.
            let requests = merge_repeats(requests);
            let name = goal.unwrap_or_else(|| {
                requests
                    .iter()
                    .find(|request| request.letter_from.is_none())
                    .map(|request| {
                        request
                            .summary
                            .clone()
                            .unwrap_or_else(|| head(&request.text, FLOW_NAME_CHARS))
                    })
                    .unwrap_or_default()
            });
            // The flow's result (D-11): its last finished turn, else its last
            // turn a limit did not stop, else its last turn, so a flow with a
            // judged turn always has one (D-04, B7).
            let last = turns
                .iter()
                .rev()
                .find(|turn| turn.end == LabelEnd::Done)
                .or_else(|| {
                    turns
                        .iter()
                        .rev()
                        .find(|turn| turn.end != LabelEnd::Limited)
                })
                .or_else(|| turns.last())
                .map(|turn| turn.outcome());
            // A past flow ends where the next begins: the line the label that
            // opened it gave, else a question or a wait the operator moved
            // on from reads as moved on (D-02, B4, B5).
            let result = match (next, closed, last) {
                (Some(next), Some(line), _) => Some(FlowResult::Closing {
                    line,
                    at_unix_ms: next,
                }),
                (Some(_), None, Some(turn))
                    if matches!(turn.end, LabelEnd::Question | LabelEnd::Waiting) =>
                {
                    Some(FlowResult::MovedOn {
                        line: turn.line,
                        at_unix_ms: turn.at_unix_ms,
                    })
                }
                (_, _, last) => last.map(FlowResult::Turn),
            };
            WorkFlow {
                id,
                name,
                at_unix_ms: at,
                request_count: requests
                    .iter()
                    .filter(|request| request.letter_from.is_none())
                    .count(),
                requests,
                result,
            }
        })
        .collect()
}

/// The first `max` characters of `text` on one line, ending in `…` when cut.
fn head(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut kept: String = line.chars().take(max - 1).collect();
    kept.push('…');
    kept
}

#[cfg(test)]
#[path = "ledger_store_tests.rs"]
mod tests;
