//! A version 3 store as every build before with-hide/hide-private#672 wrote
//! it, for the tests of its conversion: written from calls, or read from a
//! copy, with the answers the converted store must give again.

use std::path::Path;

use hide_session::Agent;
use hide_session::usage::{Candidate, Checkpoint, Event};
use rusqlite::{Connection, OptionalExtension, params};

use super::{Daily, Totals, decode, encode};
use crate::usage_ledger::local_day;

/// The version 3 schema, as every build before with-hide/hide-private#672
/// made it.
pub(crate) const V3_SCHEMA: &str = "
CREATE TABLE events(device TEXT NOT NULL, agent TEXT NOT NULL, key TEXT NOT NULL,
    day TEXT NOT NULL, fingerprint BLOB NOT NULL, at INTEGER NOT NULL,
    event TEXT NOT NULL, PRIMARY KEY(device,agent,key));
CREATE INDEX events_at ON events(at);
CREATE INDEX events_device_at ON events(device,at);
CREATE TABLE backfills(device TEXT NOT NULL, agent TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0, frozen_at INTEGER NOT NULL DEFAULT 0,
    limited INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(device,agent));
CREATE TABLE daily(device TEXT NOT NULL, agent TEXT NOT NULL, day TEXT NOT NULL,
    model TEXT NOT NULL, totals TEXT NOT NULL, PRIMARY KEY(device,agent,day,model));
CREATE INDEX daily_day ON daily(day);
CREATE TABLE cursors(device TEXT NOT NULL, agent TEXT NOT NULL, path TEXT NOT NULL,
    stamp TEXT NOT NULL, checkpoint TEXT NOT NULL, PRIMARY KEY(device,agent,path));
CREATE TABLE devices(device TEXT PRIMARY KEY, calls INTEGER NOT NULL DEFAULT 0);
CREATE TABLE price_cache(slot INTEGER PRIMARY KEY CHECK(slot=1), value TEXT NOT NULL);
PRAGMA user_version=3;
";

/// What a version 3 store holds, read before it is converted: the answers
/// the converted store must give again.
pub(crate) struct V3 {
    pub(crate) daily: Vec<Daily>,
    /// Each node's calls as `recent_calls` hands them on: agent, day and
    /// event, newest first, then by agent and key.
    pub(crate) calls: std::collections::BTreeMap<String, Vec<(String, String, Event)>>,
    /// Every call with its node and agent, in the order it was stored.
    pub(crate) stored: Vec<(String, Agent, Event)>,
    /// The tables a conversion leaves alone, row by row.
    pub(crate) kept: Vec<String>,
}

impl V3 {
    /// Writes `calls` as a build before the calls format stored them: the whole
    /// event as JSON beside its key, its day's totals, the node's count, a
    /// completed backfill and a checkpoint at the start of `file`.
    pub(crate) fn write(path: &Path, calls: &[(&str, Agent, Event)], file: &Candidate) -> Self {
        let db = Connection::open(path).unwrap();
        db.execute_batch(&format!("BEGIN;{V3_SCHEMA}")).unwrap();
        for (device, agent, event) in calls {
            count_as_version_3(&db, device, *agent, event).unwrap();
            let agent = agent.as_str();
            db.execute(
                "INSERT OR IGNORE INTO backfills(device,agent,completed) VALUES(?1,?2,1)",
                params![device, agent],
            )
            .unwrap();
            db.execute(
                "INSERT OR IGNORE INTO cursors VALUES(?1,?2,?3,?4,?5)",
                params![
                    device,
                    agent,
                    file.path,
                    encode(file).unwrap(),
                    encode(&Checkpoint::default()).unwrap()
                ],
            )
            .unwrap();
        }
        db.execute_batch("COMMIT;").unwrap();
        drop(db);
        Self::read(path)
    }

    /// The answers a version 3 store at `path` gives, read without
    /// changing it.
    pub(crate) fn read(path: &Path) -> Self {
        let db =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let daily = db
            .prepare("SELECT device,agent,day,model,totals FROM daily ORDER BY day,agent,model")
            .unwrap()
            .query_map([], |r| {
                Ok(Daily {
                    device: r.get(0)?,
                    agent: r.get(1)?,
                    day: r.get(2)?,
                    model: r.get(3)?,
                    totals: decode(&r.get::<_, String>(4)?).unwrap(),
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut calls = std::collections::BTreeMap::<String, Vec<_>>::new();
        let mut stored = Vec::new();
        let mut query = db
            .prepare("SELECT device,agent,day,event,key FROM events ORDER BY rowid")
            .unwrap();
        let mut rows = query.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let device: String = row.get(0).unwrap();
            let agent: String = row.get(1).unwrap();
            let mut event: Event = decode(&row.get::<_, String>(3).unwrap()).unwrap();
            event.key = row.get(4).unwrap();
            stored.push((
                device.clone(),
                Agent::supported()
                    .find(|known| known.as_str() == agent)
                    .unwrap(),
                event.clone(),
            ));
            calls
                .entry(device)
                .or_default()
                .push((agent, row.get(2).unwrap(), event));
        }
        for list in calls.values_mut() {
            list.sort_by(|a, b| (b.2.at_ms, &a.0, &a.2.key).cmp(&(a.2.at_ms, &b.0, &b.2.key)));
        }
        Self {
            daily,
            calls,
            stored,
            kept: kept_rows(&db),
        }
    }

    pub(crate) fn agents(&self) -> Vec<(String, Agent)> {
        let mut agents: Vec<(String, Agent)> = Vec::new();
        for (device, agent, _) in &self.stored {
            if !agents.iter().any(|(d, a)| d == device && a == agent) {
                agents.push((device.clone(), *agent));
            }
        }
        agents
    }
}

/// Counts `event` as a version 3 build counts a call it finds no row of in
/// `events`, in its order: its day's totals, then the whole event as JSON
/// beside its key, then the device's count. What a build before
/// with-hide/hide-private#672 asks of a store this build converted, which
/// refuses the first write.
pub(crate) fn count_as_version_3(
    db: &Connection,
    device: &str,
    agent: Agent,
    event: &Event,
) -> rusqlite::Result<()> {
    let agent = agent.as_str();
    let day = local_day(event.at_ms, &jiff::tz::TimeZone::UTC).unwrap();
    let json = encode(event).unwrap();
    let fingerprint = sha2::Digest::finalize(sha2::Digest::chain_update(
        <sha2::Sha256 as sha2::Digest>::new(),
        json.as_bytes(),
    ))
    .to_vec();
    let totals: Option<String> = db
        .query_row(
            "SELECT totals FROM daily WHERE device=?1 AND agent=?2 AND day=?3 AND model=?4",
            params![device, agent, day, event.model],
            |r| r.get(0),
        )
        .optional()?;
    let mut totals: Totals = totals.map(|t| decode(&t).unwrap()).unwrap_or_default();
    totals.add(event).unwrap();
    db.execute(
        "INSERT INTO daily VALUES(?1,?2,?3,?4,?5) ON CONFLICT(device,agent,day,model) \
         DO UPDATE SET totals=excluded.totals",
        params![device, agent, day, event.model, encode(&totals).unwrap()],
    )?;
    db.execute(
        "INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            device,
            agent,
            event.key,
            day,
            fingerprint,
            event.at_ms,
            json
        ],
    )?;
    db.execute(
        "INSERT INTO devices VALUES(?1,1) ON CONFLICT(device) DO UPDATE SET calls=calls+1",
        [device],
    )?;
    Ok(())
}

/// Prunes as a version 3 build's maintenance turn prunes calls, in its
/// order: each completed backfill's horizon moves up to `frozen`, then at
/// most 512 calls before it leave `events`, each taken off its device's
/// count. Answers how many left.
pub(crate) fn prune_as_version_3(db: &Connection, frozen: u64) -> usize {
    db.execute(
        "UPDATE backfills SET frozen_at=max(frozen_at,?1) WHERE completed=1",
        [frozen],
    )
    .unwrap();
    let mut query = db
        .prepare(
            "SELECT e.rowid,e.device FROM events e WHERE e.at<?1 AND EXISTS(SELECT 1 FROM \
             backfills b WHERE b.device=e.device AND b.agent=e.agent AND b.completed=1 AND \
             e.at<b.frozen_at) LIMIT 512",
        )
        .unwrap();
    let expired: Vec<(i64, String)> = query
        .query_map([frozen], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (id, device) in &expired {
        db.execute("DELETE FROM events WHERE rowid=?1", [id])
            .unwrap();
        db.execute("UPDATE devices SET calls=calls-1 WHERE device=?1", [device])
            .unwrap();
    }
    expired.len()
}

/// Every row of the tables a conversion must leave as they were.
pub(crate) fn kept_rows(db: &Connection) -> Vec<String> {
    let mut rows = Vec::new();
    for table in ["backfills", "daily", "cursors", "devices", "price_cache"] {
        let mut query = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2,3"))
            .or_else(|_| db.prepare(&format!("SELECT * FROM {table}")))
            .unwrap();
        let width = query.column_count();
        let mut found = query.query([]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let cells = (0..width)
                .map(|at| format!("{:?}", row.get_ref(at).unwrap()))
                .collect::<Vec<_>>();
            rows.push(format!("{table}:{}", cells.join("|")));
        }
    }
    rows
}
