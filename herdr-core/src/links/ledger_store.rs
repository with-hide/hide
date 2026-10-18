//! The work ledger shares the link writer, version, size cap and retention.

use super::{LinkStore, SessionKey, failed};
use crate::labels::analysis::LabelEnd;
use crate::links::ledger::*;
use crate::links::{CREATED_AFTER_MS, CREATED_BEFORE_MS, SUMMARY_SESSION_LIMIT};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

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
        tx.execute(
            &format!(
                "DELETE FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3 AND {GROUP_SQL}=?4"
            ),
            params![
                session.device_id,
                session.agent,
                session.session_id,
                format!("task-{offset}")
            ],
        )
        .map_err(failed)?;
    }
    if invalid.is_empty() {
        return Ok(());
    }
    // Remove references to retracted tasks, including later continuations.
    for table in ["ledger_turns", "ledger_requests"] {
        tx.execute(&format!("UPDATE {table} SET task_ids=(SELECT json_group_array(j.value) FROM json_each({table}.task_ids) j WHERE EXISTS(SELECT 1 FROM ledger_tasks t WHERE t.device={table}.device AND t.agent={table}.agent AND t.id={table}.id AND t.task=j.value)) WHERE device=?1 AND agent=?2 AND id=?3"),params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
    }
    tx.execute("UPDATE ledger_tasks SET (end,last_at)=(SELECT t.end,t.end_at FROM ledger_turns t,json_each(t.task_ids) j WHERE t.device=ledger_tasks.device AND t.agent=ledger_tasks.agent AND t.id=ledger_tasks.id AND j.value=ledger_tasks.task ORDER BY t.end_at DESC,t.offset DESC LIMIT 1) WHERE device=?1 AND agent=?2 AND id=?3",params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
    tx.execute("UPDATE ledger_turns SET output_task=NULL WHERE device=?1 AND agent=?2 AND id=?3 AND NOT EXISTS(SELECT 1 FROM ledger_tasks t WHERE t.device=ledger_turns.device AND t.agent=ledger_turns.agent AND t.id=ledger_turns.id AND t.task=ledger_turns.output_task)",params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
    assign_outputs(tx, session)?;
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
        let counts = match parent.as_deref() {
            Some(parent) => match request.sender.as_deref() {
                Some(sender) => {
                    sender == parent && request.letter_kind.as_deref() == Some("request")
                }
                None => Some(request.offset) == first,
            },
            None => request.sender.is_none(),
        };
        if !counts {
            continue;
        }
        tx.execute("INSERT INTO ledger_requests(device,agent,id,offset,at,text,author) \
            SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE NOT EXISTS(SELECT 1 FROM ledger_excluded WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4) \
            ON CONFLICT(device,agent,id,offset) DO UPDATE SET text=excluded.text,at=excluded.at",
            params![device,agent,id,request.offset as i64,request.at_unix_ms as i64,hide_session::links::cut(&request.text),encoded(&if parent.is_some() {WorkAuthor::Parent} else {WorkAuthor::Operator})?]).map_err(failed)?;
    }
    for (kind, marks) in [("issue", &facts.issues)] {
        for mark in marks {
            tx.execute("INSERT INTO ledger_sightings VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET at=MIN(at,excluded.at)",
                params![device,agent,id,kind,mark.repository.to_ascii_lowercase(),mark.number as i64,mark.at_unix_ms as i64]).map_err(failed)?;
        }
    }
    cap(tx, &session, "ledger_requests", REQUEST_LIMIT)?;
    cap(tx, &session, "ledger_sightings", TASK_LIMIT)?;
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

/// `task_group` in SQL: the task id up to its first `.`.
const GROUP_SQL: &str =
    "(CASE WHEN instr(task,'.')>0 THEN substr(task,1,instr(task,'.')-1) ELSE task END)";

/// The task cap counts groups, so a split request is kept or dropped whole.
fn cap_task_groups(tx: &Transaction<'_>, session: &SessionIdentity) -> Result<(), String> {
    let count: usize = tx
        .query_row(
            &format!("SELECT count(DISTINCT {GROUP_SQL}) FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3"),
            params![session.device_id, session.agent, session.session_id],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if count <= TASK_LIMIT {
        return Ok(());
    }
    tx.execute(
        &format!(
            "DELETE FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3 AND {GROUP_SQL} IN \
            (SELECT {GROUP_SQL} FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3 \
            GROUP BY {GROUP_SQL} ORDER BY min(at),min(rowid) LIMIT ?4)"
        ),
        params![
            session.device_id,
            session.agent,
            session.session_id,
            (count - TASK_LIMIT) as i64
        ],
    )
    .map_err(failed)?;
    crate::diagnostic!(serde_json::json!({
        "component":"links", "kind":"ledger.capacity", "table":"ledger_tasks",
        "device_id":session.device_id, "agent":session.agent,
        "session_id":session.session_id, "removed":count-TASK_LIMIT,
    }));
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

impl LinkStore {
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
        if let LedgerCall::Record(session) = &call {
            return self
                .ledger_record(session)
                .map(Box::new)
                .map(LedgerAnswer::Record);
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
            LedgerCall::Record(_) => unreachable!(),
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
            LedgerCall::ReleaseBacklog { backlog_key } => {
                // A typed item's issue was closed: its create no longer
                // names an issue `되돌리기` may close.
                if let Some(reference) = crate::tasks::parse_github_key(&backlog_key) {
                    tx.execute("UPDATE ledger_backlog SET state='undone' WHERE state='complete' AND lower(repo)=lower(?1) AND number=?2",
                        params![reference.repository,reference.number]).map_err(failed)?;
                }
                tx.execute("DELETE FROM ledger_issue_attempts WHERE EXISTS(SELECT 1 FROM ledger_missed m WHERE m.device=ledger_issue_attempts.device AND m.agent=ledger_issue_attempts.agent AND m.id=ledger_issue_attempts.id AND m.missed=ledger_issue_attempts.missed AND m.backlog_key=?1)",
                    params![backlog_key]).map_err(failed)?;
                tx.execute("UPDATE ledger_missed SET status=?2,issue_repo=NULL,issue_number=NULL,backlog_key=NULL WHERE backlog_key=?1",
                    params![backlog_key,encoded(&MissedStatus::Open)?]).map_err(failed)?;
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
            LedgerCall::BacklogOrigin { backlog_key } => {
                let found: Option<(String, String, String)> = tx
                    .query_row(
                        "SELECT device,agent,id FROM ledger_missed WHERE backlog_key=?1 LIMIT 1",
                        params![backlog_key],
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
                    .prepare("SELECT backlog_key FROM ledger_missed WHERE backlog_key IS NOT NULL GROUP BY backlog_key ORDER BY MAX(at) DESC LIMIT ?1")
                    .map_err(failed)?;
                let missed = statement
                    .query_map(params![BACKLOG_KEYS_LIMIT as i64], |r| r.get(0))
                    .map_err(failed)?
                    .collect::<Result<Vec<String>, _>>()
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
        // A descendant's letter continues the task that spawned it, read
        // before the write starts: the record is the same rule the screen
        // reads, so a finished task is never reopened by a letter (B5).
        let letter_task = match update {
            LedgerUpdate::TurnEnd {
                session,
                offset,
                end,
                work_author: false,
                letter: Some(sender),
                ..
            } if !self.turn_recorded(session, *offset)? => {
                // A store failure is retried whole; any other failure to
                // place the turn records it with no task rather than losing it.
                match self.letter_task(session, sender, *end) {
                    Ok(group) => group,
                    Err(error) if error.starts_with("links_store_") => return Err(error),
                    Err(error) => {
                        crate::diagnostic!(serde_json::json!({
                            "component":"links", "kind":"ledger.letter_task_unavailable",
                            "device_id":session.device_id, "session_id":session.session_id,
                            "offset":offset, "error":error,
                        }));
                        None
                    }
                }
            }
            _ => None,
        };
        let tx = self.connection.transaction().map_err(failed)?;
        match update {
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
                    "INSERT INTO ledger_titles VALUES(?1,?2,?3,?4,?5,?6) \
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
                    tx.execute(
                        "INSERT INTO ledger_requests(device,agent,id,offset,at,text,author) \
                        SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE NOT EXISTS(SELECT 1 FROM ledger_excluded \
                        WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4) ON CONFLICT(device,agent,id,offset) DO UPDATE \
                        SET text=excluded.text,at=excluded.at",
                        params![
                            session.device_id,
                            session.agent,
                            session.session_id,
                            request.offset as i64,
                            request.at_unix_ms as i64,
                            text,
                            encoded(&author)?
                        ],
                    )
                    .map_err(failed)?;
                }
                cap(&tx, session, "ledger_requests", REQUEST_LIMIT)?;
            }
            LedgerUpdate::Chapter {
                session,
                goal,
                at_unix_ms,
            } => {
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT goal FROM ledger_chapters \
                    WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,rowid DESC LIMIT 1",
                        params![session.device_id, session.agent, session.session_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(failed)?;
                if previous.as_deref() != Some(goal) {
                    tx.execute(
                        "INSERT OR IGNORE INTO ledger_chapters VALUES(?1,?2,?3,?4,?5,?6)",
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
                    cap(&tx, session, "ledger_chapters", TASK_LIMIT)?;
                }
            }
            LedgerUpdate::TurnEnd {
                session,
                offset,
                started_at_unix_ms,
                at_unix_ms,
                end,
                work_author,
                letter: _,
                turn_witness,
                judgment,
            } => {
                if turn_recorded(&tx, session, *offset)? {
                    return Ok(());
                }
                if judgment.missed.len() > NEW_MISSED_LIMIT
                    || judgment.resolved.len() > MISSED_LIMIT
                {
                    return Err("ledger_judgment_capacity".to_owned());
                }
                let chapter: Option<String> = tx
                    .query_row(
                        "SELECT chapter FROM ledger_chapters \
                    WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,rowid DESC LIMIT 1",
                        params![session.device_id, session.agent, session.session_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(failed)?;
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
                let mut tasks = Vec::new();
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
                let lineage = letter_task.map(|group| TaskDecision::Continue { ids: vec![group] });
                let decision = if *work_author {
                    Some(&judgment.task)
                } else {
                    lineage.as_ref()
                };
                if let Some(decision) = decision {
                    match decision {
                        TaskDecision::New { names } => {
                            let chapter = chapter.as_deref().ok_or("ledger_chapter_missing")?;
                            if names.is_empty() || names.len() > NEW_TASK_LIMIT {
                                return Err("ledger_task_capacity".to_owned());
                            }
                            let mut seen = BTreeSet::new();
                            for name in names {
                                if name.trim().is_empty()
                                    || name.chars().count() > MISSED_CHARS
                                    || !seen.insert(name.trim())
                                {
                                    return Err("ledger_task_invalid".to_owned());
                                }
                            }
                            // The request's tasks are one group: the first
                            // keeps `task-<offset>`, so stored rows and
                            // retraction keep reading it.
                            for (index, name) in names.iter().enumerate() {
                                let task = match index {
                                    0 => format!("task-{offset}"),
                                    n => format!("task-{offset}.{}", n + 1),
                                };
                                tx.execute(
                                    "INSERT INTO ledger_tasks VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                                    params![
                                        session.device_id,
                                        session.agent,
                                        session.session_id,
                                        task,
                                        chapter,
                                        name,
                                        start_at as i64,
                                        *at_unix_ms as i64,
                                        encoded(end)?
                                    ],
                                )
                                .map_err(failed)?;
                                tasks.push(task);
                            }
                        }
                        TaskDecision::Continue { ids } => {
                            let groups =
                                ids.iter().map(|id| task_group(id)).collect::<BTreeSet<_>>();
                            if ids.is_empty() || groups.len() > CONTINUED_TASK_LIMIT {
                                return Err("ledger_task_capacity".to_owned());
                            }
                            // Continuing one task continues its request's
                            // whole group: its tasks share every END.
                            for task in ids {
                                let group = task_group(task);
                                let exists: bool = tx
                                    .query_row(
                                        "SELECT EXISTS(SELECT 1 FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3 AND task=?4)",
                                        params![session.device_id, session.agent, session.session_id, task],
                                        |row| row.get(0),
                                    )
                                    .map_err(failed)?;
                                if !exists {
                                    return Err("ledger_task_unknown".to_owned());
                                }
                                if tasks.iter().any(|held: &String| task_group(held) == group) {
                                    continue;
                                }
                                tx.execute(
                                    &format!(
                                        "UPDATE ledger_tasks SET end=?5,last_at=?6 \
                                    WHERE device=?1 AND agent=?2 AND id=?3 AND {GROUP_SQL}=?4"
                                    ),
                                    params![
                                        session.device_id,
                                        session.agent,
                                        session.session_id,
                                        group,
                                        encoded(end)?,
                                        *at_unix_ms as i64
                                    ],
                                )
                                .map_err(failed)?;
                                let mut members = tx
                                    .prepare(&format!("SELECT task FROM ledger_tasks WHERE device=?1 AND agent=?2 AND id=?3 AND {GROUP_SQL}=?4 ORDER BY rowid"))
                                    .map_err(failed)?;
                                let members = members
                                    .query_map(
                                        params![
                                            session.device_id,
                                            session.agent,
                                            session.session_id,
                                            group
                                        ],
                                        |row| row.get::<_, String>(0),
                                    )
                                    .map_err(failed)?
                                    .collect::<Result<Vec<_>, _>>()
                                    .map_err(failed)?;
                                tasks.extend(members);
                            }
                        }
                        TaskDecision::None => {}
                    }
                }
                // An output belongs to a group, named by its first task: one
                // group's turn needs no choice, two groups' take the model's.
                let groups = tasks
                    .iter()
                    .map(|id| task_group(id))
                    .collect::<BTreeSet<_>>();
                let output_task = if groups.len() == 1 {
                    groups.first().map(|group| (*group).to_owned())
                } else {
                    judgment
                        .output_task_id
                        .as_deref()
                        .filter(|id| tasks.iter().any(|task| task == id))
                        .map(|id| task_group(id).to_owned())
                };
                tx.execute(
                    "INSERT INTO ledger_turns VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        *offset as i64,
                        start_at as i64,
                        *at_unix_ms as i64,
                        encoded(end)?,
                        encoded(&tasks)?,
                        output_task
                    ],
                )
                .map_err(failed)?;
                tx.execute(
                    "UPDATE ledger_requests SET judged=1,chapter=?5,task_ids=?6 \
                    WHERE device=?1 AND agent=?2 AND id=?3 AND offset=?4",
                    params![
                        session.device_id,
                        session.agent,
                        session.session_id,
                        *offset as i64,
                        chapter,
                        encoded(&tasks)?
                    ],
                )
                .map_err(failed)?;
                for (index, missed) in judgment.missed.iter().enumerate() {
                    if missed.text.trim().is_empty() || missed.text.chars().count() > MISSED_CHARS {
                        return Err("ledger_missed_invalid".to_owned());
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
                    tx.execute(
                        "UPDATE ledger_missed SET status=?5,issue_repo=?6,issue_number=?7,resolved_issue_number=?9,resolved_turn_offset=?10 \
                        WHERE device=?1 AND agent=?2 AND id=?3 AND missed=?4 AND status=?8",
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
                            *offset as i64
                        ],
                    )
                    .map_err(failed)?;
                }
                cap_task_groups(&tx, session)?;
                cap(&tx, session, "ledger_missed", MISSED_LIMIT)?;
                tx.execute("DELETE FROM ledger_issue_attempts WHERE device=?1 AND agent=?2 AND id=?3 AND state!='creating' AND NOT EXISTS(SELECT 1 FROM ledger_missed m WHERE m.device=ledger_issue_attempts.device AND m.agent=ledger_issue_attempts.agent AND m.id=ledger_issue_attempts.id AND m.missed=ledger_issue_attempts.missed)",params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
                // Turns use their end timestamp for bounded retention.
                tx.execute("DELETE FROM ledger_turns WHERE rowid IN (SELECT rowid FROM ledger_turns \
                    WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY end_at DESC LIMIT -1 OFFSET ?4)",
                    params![session.device_id,session.agent,session.session_id,REQUEST_LIMIT as i64]).map_err(failed)?;
                assign_outputs(&tx, session)?;
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
            params![session.device_id,session.agent,session.session_id], |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(failed)?;
        if let Some((project, repository)) = origin {
            record.project = project;
            record.repository = repository;
        }
        let titles:Option<(Option<String>,Option<String>,Option<String>)>=self.connection.query_row(
            "SELECT native_title,manual_title,state FROM ledger_titles WHERE device=?1 AND agent=?2 AND id=?3",
            params![session.device_id,session.agent,session.session_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(failed)?;
        if let Some((native, manual, state)) = titles {
            record.native_title = native;
            record.manual_title = manual;
            record.state = state;
        }
        let last_end: Option<String> = self
            .connection
            .query_row(
                "SELECT end FROM ledger_turns \
            WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY end_at DESC LIMIT 1",
                params![session.device_id, session.agent, session.session_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(failed)?;
        record.last_end = last_end.map(decoded).transpose()?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT chapter,goal,at FROM ledger_chapters \
            WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at",
            )
            .map_err(failed)?;
        record.chapters = statement
            .query_map(args, |r| {
                Ok(WorkChapter {
                    id: r.get(0)?,
                    goal: r.get(1)?,
                    at_unix_ms: r.get(2)?,
                })
            })
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)?;
        record.title = record
            .chapters
            .last()
            .map(|chapter| chapter.goal.clone())
            .or_else(|| record.native_title.clone())
            .unwrap_or_else(|| crate::sidebar::provider_name(Some(&session.agent)));
        if include_requests {
            let mut statement=self.connection.prepare("SELECT offset,at,text,author,task_ids,chapter,judged \
            FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,offset DESC LIMIT ?4").map_err(failed)?;
            let requests = statement
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
                            r.get::<_, String>(4)?,
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, bool>(6)?,
                        ))
                    },
                )
                .map_err(failed)?;
            for request in requests {
                if *request_budget == 0 {
                    return Err("ledger_record_request_capacity".into());
                }
                *request_budget -= 1;
                let (offset, at, text, author, tasks, chapter, judged) = request.map_err(failed)?;
                record.requests.push(RecordedRequest {
                    offset,
                    at_unix_ms: at,
                    text,
                    author: decoded(author)?,
                    task_ids: decoded(tasks)?,
                    chapter_id: chapter,
                    judged,
                });
            }
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT task,chapter,name,at,last_at,end FROM ledger_tasks \
            WHERE device=?1 AND agent=?2 AND id=?3 ORDER BY at DESC,rowid",
            )
            .map_err(failed)?;
        let tasks = statement
            .query_map(
                params![session.device_id, session.agent, session.session_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, u64>(3)?,
                        r.get::<_, u64>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .map_err(failed)?;
        for task in tasks {
            let (id, chapter, name, at, last, end) = task.map_err(failed)?;
            record.tasks.push(WorkTask {
                id,
                chapter_id: chapter,
                name,
                at_unix_ms: at,
                last_at_unix_ms: last,
                end: decoded(end)?,
                done: false,
                unjudged: false,
            });
        }
        record.missed = self.ledger_missed(session)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT o.kind,o.repo,o.number,o.title,o.url,o.state,o.created_at,o.task,COALESCE(f.stale,1),f.read_at \
            FROM ledger_outputs o LEFT JOIN ledger_output_facts f ON f.repo=o.repo AND f.kind=o.kind AND f.number=o.number \
            WHERE o.device=?1 AND o.agent=?2 AND o.id=?3 ORDER BY o.created_at DESC LIMIT 2001",
            )
            .map_err(failed)?;
        record.outputs = statement
            .query_map(
                params![session.device_id, session.agent, session.session_id],
                |r| {
                    Ok(WorkOutput {
                        kind: r.get(0)?,
                        repository: r.get(1)?,
                        number: r.get(2)?,
                        title: r.get(3)?,
                        url: r.get(4)?,
                        state: r.get(5)?,
                        created_at_unix_ms: r.get(6)?,
                        task_id: r.get(7)?,
                        stale: r.get(8)?,
                        last_read_at_unix_ms: r.get(9)?,
                        source: session.clone(),
                        source_name: record.title.clone(),
                    })
                },
            )
            .map_err(failed)?
            .collect::<Result<_, _>>()
            .map_err(failed)?;
        if record.outputs.len() > REQUEST_LIMIT {
            return Err("ledger_output_capacity".into());
        }
        // A child is part of the record once the ledger records it: a spawn
        // is known before its file is read, and a Pi or omp child's edge is
        // under its file until the reader names it.
        let mut statement = self
            .connection
            .prepare(
                "SELECT p.device,p.agent,p.id FROM session_parents p JOIN sessions s \
            ON s.device=p.device AND s.agent=p.agent AND s.id=p.id \
            WHERE p.parent_device=?1 AND p.parent_agent=?2 AND p.parent_id=?3 LIMIT ?4",
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
        let mut child_work = Vec::with_capacity(children.len());
        // Task groups a spawned child holds open (D-05, B23).
        let mut busy_groups = BTreeSet::new();
        for child in children {
            let mut child =
                self.record_tree(&child, visited, depth + 1, request_budget, include_requests)?;
            let spawn_task: Option<String> = self.connection.query_row("SELECT output_task FROM ledger_turns WHERE device=?1 AND agent=?2 AND id=?3 AND ?4 BETWEEN start_at AND end_at ORDER BY start_at DESC LIMIT 1",
                params![session.device_id,session.agent,session.session_id,child.started_at_unix_ms.map(|n|n as i64)],|r|r.get(0)).optional().map_err(failed)?.flatten();
            child.spawn_task = spawn_task.clone();
            let facts = self.child_facts(&child.session)?;
            if facts.holds_task(&child)
                && let Some(task) = &spawn_task
            {
                busy_groups.insert(task_group(task).to_owned());
            }
            child_work.push(ChildWork::of(&child, facts));
            record
                .outputs
                .extend(child.outputs.iter().cloned().map(|mut output| {
                    output.task_id = spawn_task.clone();
                    output
                }));
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
        // A request's tasks finish together: they share every END and every
        // pull request assigned to any of them (`task_done`). Done is read
        // from facts and the last judged END; an unjudged newest turn only
        // takes the word from a task left open (D-09, B19).
        let latest_unjudged = self.latest_unjudged(session)?;
        for task in &mut record.tasks {
            let group = task_group(&task.id);
            task.done = task_done(
                task.end,
                record.outputs.iter().filter(|o| {
                    o.kind == "pr" && o.task_id.as_deref().map(task_group) == Some(group)
                }),
                busy_groups.contains(group),
            );
            task.unjudged = !task.done && latest_unjudged;
        }
        record.child_progress = ChildProgress::of(child_work);
        Ok(record)
    }

    /// Whether `update` is a descendant letter's turn that must wait for the
    /// runtime's first lineage facts after a start: before them no sender can
    /// be matched, and a turn recorded with no task is never placed again.
    pub fn awaits_lineage(&self, update: &LedgerUpdate) -> bool {
        self.lineage.is_none()
            && matches!(
                update,
                LedgerUpdate::TurnEnd {
                    work_author: false,
                    letter: Some(_),
                    ..
                }
            )
    }

    fn turn_recorded(&self, session: &SessionIdentity, offset: u64) -> Result<bool, String> {
        turn_recorded(&self.connection, session, offset)
    }

    /// Whether the session's newest turn went unjudged: its transcript
    /// offset is past every judged turn's.
    fn latest_unjudged(&self, session: &SessionIdentity) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT COALESCE((SELECT MAX(offset) FROM ledger_unjudged WHERE device=?1 AND agent=?2 AND id=?3),-1) \
                 > COALESCE((SELECT MAX(offset) FROM ledger_turns WHERE device=?1 AND agent=?2 AND id=?3),-1)",
                params![session.device_id, session.agent, session.session_id],
                |row| row.get(0),
            )
            .map_err(failed)
    }

    /// What decides a child beside its record (D-08, D-09): a report to its
    /// parent with no turn started after it, and an unjudged newest turn.
    fn child_facts(&self, child: &SessionIdentity) -> Result<ChildFacts, String> {
        let lineage = self.lineage.as_ref().and_then(|known| known.get(child));
        let reported_idle = match lineage.and_then(|lineage| lineage.reported_at_unix_ms) {
            Some(at) => !self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ledger_turns WHERE device=?1 AND agent=?2 AND id=?3 AND start_at>?4) \
                     OR EXISTS(SELECT 1 FROM ledger_requests WHERE device=?1 AND agent=?2 AND id=?3 AND at>?4)",
                    params![child.device_id, child.agent, child.session_id, at as i64],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(failed)?,
            None => false,
        };
        Ok(ChildFacts {
            reported_idle,
            latest_unjudged: self.latest_unjudged(child)?,
            live: lineage.is_some_and(|lineage| lineage.live),
        })
    }

    /// The task group a letter from `sender` continues (PRD
    /// task-continuation D-02): the spawn task of the direct child whose
    /// subtree holds the descendant of that name. A sender outside the
    /// lineage or a name two groups share continues nothing, and a finished
    /// task takes the turn only when its `end` keeps it finished: the
    /// question of the turn that merged its PR keeps its `!` (B3, D-07), and
    /// no letter reopens it (B5).
    fn letter_task(
        &self,
        session: &SessionIdentity,
        sender: &str,
        end: LabelEnd,
    ) -> Result<Option<String>, String> {
        if self.session(&key(session))?.is_none() {
            return Ok(None);
        }
        let record = self.record_tree(session, &mut BTreeSet::new(), 0, &mut 0, false)?;
        let lineage = self.lineage.as_ref().ok_or("ledger_lineage_pending")?;
        fn named(
            record: &SessionRecord,
            sender: &str,
            lineage: &BTreeMap<SessionIdentity, super::ChildLineage>,
        ) -> bool {
            lineage
                .get(&record.session)
                .is_some_and(|child| hide_session::envelope_names(sender, &child.name))
                || record
                    .children
                    .iter()
                    .any(|child| named(child, sender, lineage))
        }
        let groups = record
            .children
            .iter()
            .filter(|child| named(child, sender, lineage))
            .map(|child| child.spawn_task.clone())
            .collect::<BTreeSet<_>>();
        if groups.len() > 1 {
            crate::diagnostic!(serde_json::json!({
                "component":"links", "kind":"ledger.letter_sender_ambiguous",
                "device_id":session.device_id, "session_id":session.session_id,
                "groups":groups.len(),
            }));
            return Ok(None);
        }
        if groups.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component":"links", "kind":"ledger.letter_sender_unknown",
                "device_id":session.device_id, "session_id":session.session_id,
            }));
        } else if groups.contains(&None) {
            // The child is known but no judged turn of this session holds its
            // start, so there is no task to continue.
            crate::diagnostic!(serde_json::json!({
                "component":"links", "kind":"ledger.letter_spawn_task_missing",
                "device_id":session.device_id, "session_id":session.session_id,
            }));
        }
        let Some(group) = groups.into_iter().next().flatten() else {
            return Ok(None);
        };
        let mut tasks = record
            .tasks
            .iter()
            .filter(|task| task_group(&task.id) == group)
            .peekable();
        if tasks.peek().is_none() {
            return Ok(None);
        }
        // Tasks of one group finish together, so they are all done or none.
        let reopens = tasks.all(|task| task.done)
            && !task_done(
                end,
                record.outputs.iter().filter(|o| {
                    o.kind == "pr" && o.task_id.as_deref().map(task_group) == Some(group.as_str())
                }),
                false,
            );
        Ok((!reopens).then_some(group))
    }

    /// A live pane's chip and checklist: `None` for a session the ledger does
    /// not hold or one with no task yet.
    pub fn ledger_progress(
        &self,
        session: &SessionIdentity,
    ) -> Result<Option<WorkProgress>, String> {
        if self.session(&key(session))?.is_none() {
            return Ok(None);
        }
        let record = self.record_tree(session, &mut BTreeSet::new(), 0, &mut 0, false)?;
        Ok(WorkProgress::of(&record))
    }

    fn ledger_missed(&self, session: &SessionIdentity) -> Result<Vec<MissedWork>, String> {
        ledger_missed_rows(&self.connection, session)
    }

    /// What a later turn of `session` continues. A session the ledger has
    /// not recorded, one whose file no read has reached yet or one retention
    /// removed, has no work to continue, only the parent that started it.
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
        // The same descendant-to-spawn-turn attribution decides both the
        // visible verdict and which tasks a later turn may continue.
        let record = self.record_tree(session, &mut BTreeSet::new(), 0, &mut 0, false)?;
        let (mut finished_tasks, open_tasks): (Vec<_>, Vec<_>) =
            record.tasks.into_iter().partition(|task| task.done);
        // The newest finished requests, each with all its tasks: a split
        // request is one candidate, and continuing any of its tasks reopens
        // the whole group (PRD task-split-checklist D-03).
        finished_tasks.sort_by_key(|task| std::cmp::Reverse(task.last_at_unix_ms));
        let mut groups = Vec::new();
        finished_tasks.retain(|task| {
            let group = task_group(&task.id).to_owned();
            if !groups.contains(&group) {
                if groups.len() == FINISHED_TASKS_IN_CONTEXT {
                    return false;
                }
                groups.push(group);
            }
            true
        });
        Ok(LedgerContext {
            parent_name,
            open_tasks,
            finished_tasks,
            open_missed: record
                .missed
                .into_iter()
                .filter(|missed| missed.status == MissedStatus::Open)
                .collect(),
        })
    }

    pub fn ledger_parent(
        &self,
        session: &SessionIdentity,
    ) -> Result<Option<SessionIdentity>, String> {
        self.connection.query_row("SELECT parent_device,parent_agent,parent_id FROM session_parents WHERE device=?1 AND agent=?2 AND id=?3", params![session.device_id,session.agent,session.session_id], |r| Ok(SessionIdentity { device_id:r.get(0)?,agent:r.get(1)?,session_id:r.get(2)? })).optional().map_err(failed)
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
        let project = self.project(project)?.ok_or("ledger_project_missing")?;
        let Some(repo) = project.repo.as_deref() else {
            return Ok(Vec::new());
        };
        let paths = self.project_paths(&project)?;
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
                .is_some_and(|row| row.inside(&paths))
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
            crate::diagnostic!(
                serde_json::json!({"component":"links","kind":"ledger.issue_read_capacity","limit":crate::issues::ISSUE_LIMIT,"omitted":refs.len()-crate::issues::ISSUE_LIMIT})
            );
        }
        Ok(refs.into_iter().take(crate::issues::ISSUE_LIMIT).collect())
    }

    pub fn ledger_summaries(&self, project: &str) -> Result<Vec<SessionWorkSummary>, String> {
        let project = self.project(project)?.ok_or("ledger_project_missing")?;
        let paths = self.project_paths(&project)?;
        // A summary never loads request bodies or record trees. It reads
        // bounded counts once, then rolls each count along stored lineage.
        let mut statement = self
            .connection
            .prepare(
                "SELECT device,agent,id,count(*) FROM ledger_missed \
            WHERE status=?1 GROUP BY device,agent,id LIMIT ?2",
            )
            .map_err(failed)?;
        let counts = statement
            .query_map(
                params![
                    encoded(&MissedStatus::Open)?,
                    SUMMARY_SESSION_LIMIT as i64 + 1
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
        let sessions = self
            .all_sessions()?
            .into_iter()
            .filter(|session| session.inside(&paths))
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
                    .filter(|missed| missed.status == MissedStatus::Open)
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
                include_tasks: summaries,
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
        let mut values = "SELECT r.device,r.agent,r.id,r.text,r.at,'request' AS kind,r.offset FROM ledger_requests r JOIN scoped s USING(device,agent,id) WHERE r.author='\"operator\"'".to_owned();
        if options.include_tasks && summaries {
            values.push_str(" UNION ALL SELECT r.device,r.agent,r.id,r.name,r.at,'task',NULL FROM ledger_tasks r JOIN scoped s USING(device,agent,id) UNION ALL SELECT r.device,r.agent,r.id,r.goal,r.at,'task',NULL FROM ledger_chapters r JOIN scoped s USING(device,agent,id)");
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
                    (SELECT COUNT(*) FROM ledger_requests r WHERE r.device=s.device AND r.agent=s.agent AND r.id=s.id AND r.author='"operator"') request_count
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
    tx.execute("UPDATE ledger_outputs SET task=(SELECT output_task FROM ledger_turns t WHERE t.device=ledger_outputs.device \
            AND t.agent=ledger_outputs.agent AND t.id=ledger_outputs.id AND ledger_outputs.sighted_at BETWEEN t.start_at AND t.end_at \
            ORDER BY t.start_at DESC LIMIT 1) WHERE kind='pr'",[]).map_err(failed)?;
    tx.execute("INSERT INTO ledger_outputs(device,agent,id,kind,repo,number,title,url,state,created_at,sighted_at) \
            SELECT s.device,s.agent,s.id,s.kind,s.repo,s.number,f.title,f.url,f.state,f.created_at,s.at \
            FROM ledger_sightings s JOIN ledger_output_facts f ON f.repo=s.repo AND f.kind=s.kind AND f.number=s.number \
            WHERE f.created_at IS NOT NULL AND s.at>=f.created_at-?1 AND s.at<=f.created_at+?2 \
            ON CONFLICT(device,agent,id,kind,repo,number) DO UPDATE SET title=excluded.title,url=excluded.url,state=excluded.state",
            params![CREATED_BEFORE_MS as i64,CREATED_AFTER_MS as i64]).map_err(failed)?;
    tx.execute("UPDATE ledger_outputs SET state=COALESCE((SELECT f.state FROM ledger_output_facts f WHERE f.kind=ledger_outputs.kind AND f.number=ledger_outputs.number AND (f.repo=ledger_outputs.repo OR f.repo=(SELECT name FROM repo_names n WHERE n.repo=ledger_outputs.repo LIMIT 1))),state)",[]).map_err(failed)?;
    tx.execute("UPDATE ledger_outputs SET task=(SELECT output_task FROM ledger_turns t WHERE t.device=ledger_outputs.device AND t.agent=ledger_outputs.agent AND t.id=ledger_outputs.id AND ledger_outputs.sighted_at BETWEEN t.start_at AND t.end_at ORDER BY t.start_at DESC LIMIT 1) WHERE kind='issue'",[]).map_err(failed)?;
    // GitHub may arrive after END. Retain the proposed number and the
    // resolving turn, then attach only that turn's verified creation.
    tx.execute("UPDATE ledger_missed AS m SET status=?1,issue_number=resolved_issue_number,issue_repo=(SELECT repo FROM ledger_outputs o JOIN ledger_turns t ON t.device=o.device AND t.agent=o.agent AND t.id=o.id AND t.offset=m.resolved_turn_offset WHERE o.device=m.device AND o.agent=m.agent AND o.id=m.id AND o.kind='issue' AND o.number=m.resolved_issue_number AND o.sighted_at BETWEEN t.start_at AND t.end_at LIMIT 1) WHERE m.status=?2 AND m.resolved_issue_number IS NOT NULL AND EXISTS(SELECT 1 FROM ledger_outputs o JOIN ledger_turns t ON t.device=o.device AND t.agent=o.agent AND t.id=o.id AND t.offset=m.resolved_turn_offset WHERE o.device=m.device AND o.agent=m.agent AND o.id=m.id AND o.kind='issue' AND o.number=m.resolved_issue_number AND o.sighted_at BETWEEN t.start_at AND t.end_at)",params![encoded(&MissedStatus::Issue)?,encoded(&MissedStatus::Resolved)?]).map_err(failed)?;
    Ok(())
}

fn assign_outputs(tx: &Transaction<'_>, session: &SessionIdentity) -> Result<(), String> {
    tx.execute("UPDATE ledger_outputs SET task=(SELECT output_task FROM ledger_turns t WHERE t.device=?1 \
        AND t.agent=?2 AND t.id=?3 AND ledger_outputs.sighted_at BETWEEN t.start_at AND t.end_at ORDER BY t.start_at DESC LIMIT 1) \
        WHERE device=?1 AND agent=?2 AND id=?3",params![session.device_id,session.agent,session.session_id]).map_err(failed)?;
    Ok(())
}

#[cfg(test)]
#[path = "ledger_store_tests.rs"]
mod tests;
