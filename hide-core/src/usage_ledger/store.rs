//! Calls, daily totals and checkpoints are one transaction. After the first
//! backfill, days outside the event horizon are frozen and never added again.
//! A damaged store is refused, never rebuilt: its old daily totals may be
//! the only remaining record after an agent deletes its source history.
//!
//! A call row holds each value its readers need once (with-hide/hide-private#672):
//! its key as 32 bytes, its node by number, its tokens as numbers and its
//! session and folder by the number of a `places` row, so a call costs about
//! a fifth of the whole-event JSON a version 3 store kept. A version 3 store
//! is converted in place, a batch per step ([`Store::convert`]), and then
//! compacted once ([`Store::compact`]).
//!
//! A build from before this format opens a converted store as it opens its
//! own: the version stays 3 and its `events` and `devices` tables stay,
//! empty, so it goes on counting its calls into `events` and Hide AI's into
//! `ai_daily`. Back on this build, those rows are converted like a version
//! 3 store's, and a call the older build counted a second time, which it
//! could not find in `events`, is taken back from its day ([`Totals::remove`])
//! and taken as the later view of the call kept here, as ingest takes one.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use hide_session::{
    Agent,
    usage::{Candidate, Checkpoint, Event, Step, Tokens},
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{DAILY_DAYS, DAILY_LIMIT, DAY_MS, EVENT_DAYS, EVENT_LIMIT, local_day};
mod ai;
pub use ai::{AiPart, AiRecorded, AiRow, AiUnknown};
mod stream;
#[cfg(test)]
pub(crate) mod v3_fixture;

/// The oldest layout a build reads and writes this store correctly in. The
/// calls format did not raise it: every table a version 3 build uses is
/// still here, and `calls` beside them marks a converted store.
const VERSION: i64 = 3;
const DEVICE_LIMIT: usize = 32;
const BATCH_LIMIT: usize = 8192;
const CHECKPOINT_BYTES: usize = 16 * 1024;
/// The calls and what they name. `places` holds each (node, agent, session,
/// folder) a call ran in once, with the newest call's time, so the place
/// leaves with its last call ([`Store::prune`]); every place has had a call
/// of its own, so a node's places never outnumber its calls. A call's folder
/// is `''` when the native record named none, which no folder can be
/// (`valid_name`). `calls_node_at` serves the projects' costs and pruning;
/// beside a table without rowids it carries the primary key too, ascending
/// after the newest-first time, so a read ordered by time (newest first),
/// agent and key sorts nothing.
const CALLS_SCHEMA: &str = "
CREATE TABLE nodes(id INTEGER PRIMARY KEY, device TEXT NOT NULL UNIQUE,
    calls INTEGER NOT NULL DEFAULT 0);
CREATE TABLE places(id INTEGER PRIMARY KEY, node INTEGER NOT NULL, agent TEXT NOT NULL,
    session TEXT NOT NULL, cwd TEXT NOT NULL, last_at INTEGER NOT NULL,
    UNIQUE(node,agent,session,cwd));
CREATE TABLE calls(node INTEGER NOT NULL, agent TEXT NOT NULL, key BLOB NOT NULL,
    at INTEGER NOT NULL, day TEXT NOT NULL, model TEXT NOT NULL, place INTEGER NOT NULL,
    provider TEXT, input INTEGER NOT NULL, output INTEGER NOT NULL,
    cache_read INTEGER NOT NULL, cache_write INTEGER NOT NULL,
    cache_write_1h INTEGER NOT NULL, reasoning INTEGER, cost INTEGER,
    incomplete INTEGER NOT NULL, PRIMARY KEY(node,agent,key)) WITHOUT ROWID;
CREATE INDEX calls_node_at ON calls(node,at DESC);
";
/// The tables a version 3 build made, kept for one that opens this store:
/// a version 3 store has them, and a new store is made with them.
const SCHEMA: &str = "
CREATE TABLE events(device TEXT NOT NULL, agent TEXT NOT NULL, key TEXT NOT NULL,
    day TEXT NOT NULL, fingerprint BLOB NOT NULL, at INTEGER NOT NULL,
    event TEXT NOT NULL, PRIMARY KEY(device,agent,key));
CREATE INDEX events_at ON events(at);
CREATE TABLE backfills(device TEXT NOT NULL, agent TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0, frozen_at INTEGER NOT NULL DEFAULT 0,
    limited INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(device,agent));
CREATE TABLE daily(device TEXT NOT NULL, agent TEXT NOT NULL, day TEXT NOT NULL,
    model TEXT NOT NULL, totals TEXT NOT NULL, PRIMARY KEY(device,agent,day,model));
CREATE INDEX daily_day ON daily(day);
CREATE TABLE cursors(device TEXT NOT NULL, agent TEXT NOT NULL, path TEXT NOT NULL,
    stamp TEXT NOT NULL, checkpoint TEXT NOT NULL, PRIMARY KEY(device,agent,path));
CREATE TABLE devices(device TEXT PRIMARY KEY, calls INTEGER NOT NULL DEFAULT 0);
CREATE TABLE price_cache(slot INTEGER PRIMARY KEY CHECK(slot=1), value TEXT NOT NULL);
PRAGMA user_version=3;
";
/// A version 3 store gains the calls tables in one transaction that moves
/// no call: its `events` stay where they are, without the two indexes that
/// would slow each move, until [`Store::convert`] has moved them all. Each
/// device becomes a node with no calls yet; a move counts its calls there
/// and takes them off the device, whose count an older build still reads.
const START_CONVERSION: &str = "
INSERT INTO nodes(device) SELECT device FROM devices;
DROP INDEX IF EXISTS events_at;
DROP INDEX IF EXISTS events_device_at;
";
/// The calls one conversion step moves, in one transaction: about half a
/// second on a loaded Mac, so 640,000 calls take one to two minutes of
/// steps while a window is open.
const CONVERT_BATCH: usize = 4096;
/// A conversion step's own budget: the start drops two indexes and every
/// step reads and writes thousands of calls, past an ordinary step's 250 ms
/// on a machine under load.
const CONVERT_BUDGET: Duration = Duration::from_secs(30);
/// Compacting rewrites the whole file once: four seconds for a converted
/// 640,000-call store on a loaded Mac.
const COMPACT_BUDGET: Duration = Duration::from_secs(120);

/// Hide's own background AI, counted by day, provider, model and feature.
/// Added beside the session tables without a version bump, so an older build
/// opens the same file and ignores them (PRD hide-ai-usage-split D-10).
/// `limited_day` is the earliest day whose calls found the row cap full.
const AI_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ai_daily(day TEXT NOT NULL, provider TEXT NOT NULL,
    model TEXT NOT NULL, feature TEXT NOT NULL, totals TEXT NOT NULL,
    unknown INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(day,provider,model,feature));
CREATE INDEX IF NOT EXISTS ai_daily_day ON ai_daily(day);
CREATE TABLE IF NOT EXISTS ai_state(slot INTEGER PRIMARY KEY CHECK(slot=1), limited_day TEXT);
";
/// The own node's calls of a window, newest first. `(node, agent, key)` is
/// the primary key, so the order is total: past the limit the same calls are
/// read whichever plan SQLite chooses, and by `calls_node_at` none is sorted.
const RECENT_CALLS: &str = "SELECT c.agent,c.day,c.key,c.at,c.model,c.input,c.output,c.cache_read,\
    c.cache_write,c.cache_write_1h,c.reasoning,c.cost,c.incomplete,c.provider,p.session,p.cwd \
    FROM calls c JOIN places p ON p.id=c.place \
    WHERE c.node=(SELECT id FROM nodes WHERE device=?1) AND c.at>=?2 \
    ORDER BY c.at DESC,c.agent,c.key LIMIT ?3";
/// One stored call by its identity, with the day it was counted on.
const STORED_CALL: &str = "SELECT c.day,c.key,c.at,c.model,c.input,c.output,c.cache_read,\
    c.cache_write,c.cache_write_1h,c.reasoning,c.cost,c.incomplete,c.provider,p.session,p.cwd \
    FROM calls c JOIN places p ON p.id=c.place WHERE c.node=?1 AND c.agent=?2 AND c.key=?3";
/// Distinct (day, provider, model, feature) rows kept for Hide AI. About 400
/// days of four features, two providers and a few models each.
pub const AI_ROW_LIMIT: usize = 40_000;

/// No error returned by this boundary includes source content or paths.
fn failed(error: rusqlite::Error) -> &'static str {
    match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => "usage_store_full",
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "usage_store_busy",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "usage_store_corrupt",
        Some(ErrorCode::OperationInterrupted) => "usage_store_budget",
        _ => "usage_store_failed",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub tokens: Tokens,
    pub calls: u64,
    pub recorded_cost_micro_usd: u64,
    pub missing_cost_calls: u64,
    pub incomplete_calls: u64,
    /// Native provider ids only. Bounded by the model/provider vocabulary.
    pub providers: Vec<String>,
    /// Fixed token-only bands preserve whole-call tier pricing after event
    /// expiry. There are no call identities or source paths in these bands.
    #[serde(default)]
    pub price_bands: [Tokens; 4],
}

fn plus(a: &mut u64, b: u64) -> Result<(), &'static str> {
    *a = a.checked_add(b).ok_or("usage_total_overflow")?;
    Ok(())
}

impl Totals {
    fn add(&mut self, event: &Event) -> Result<(), &'static str> {
        self.add_call(
            &event.tokens,
            event.recorded_cost_micro_usd,
            event.incomplete,
            event.provider.as_deref(),
        )
    }

    /// One call's tokens into the totals and into the price band its prompt
    /// size selects. Hide AI's own calls come through here too, with no
    /// recorded cost, so the same price table reads them.
    pub(super) fn add_call(
        &mut self,
        tokens: &Tokens,
        recorded_cost_micro_usd: Option<u64>,
        incomplete: bool,
        provider: Option<&str>,
    ) -> Result<(), &'static str> {
        plus(&mut self.tokens.input, tokens.input)?;
        plus(&mut self.tokens.output, tokens.output)?;
        plus(&mut self.tokens.cache_read, tokens.cache_read)?;
        plus(&mut self.tokens.cache_write, tokens.cache_write)?;
        plus(&mut self.tokens.cache_write_1h, tokens.cache_write_1h)?;
        if let Some(n) = tokens.reasoning {
            plus(self.tokens.reasoning.get_or_insert(0), n)?;
        }
        let prompt = tokens
            .input
            .checked_add(tokens.cache_read)
            .and_then(|n| n.checked_add(tokens.cache_write))
            .ok_or("usage_total_overflow")?;
        let band = &mut self.price_bands[super::prices::prompt_band(prompt)];
        plus(&mut band.input, tokens.input)?;
        plus(&mut band.output, tokens.output)?;
        plus(&mut band.cache_read, tokens.cache_read)?;
        plus(&mut band.cache_write, tokens.cache_write)?;
        plus(&mut band.cache_write_1h, tokens.cache_write_1h)?;
        if let Some(n) = tokens.reasoning {
            plus(band.reasoning.get_or_insert(0), n)?;
        }
        plus(&mut self.calls, 1)?;
        if let Some(cost) = recorded_cost_micro_usd.filter(|_| !incomplete) {
            plus(&mut self.recorded_cost_micro_usd, cost)?;
        } else {
            plus(&mut self.missing_cost_calls, 1)?;
        }
        plus(&mut self.incomplete_calls, u64::from(incomplete))?;
        if let Some(provider) = provider
            && !self.providers.iter().any(|known| known == provider)
        {
            if self.providers.len() == 32 {
                return Err("usage_provider_capacity");
            }
            self.providers.push(provider.to_owned());
            self.providers.sort();
        }
        Ok(())
    }

    /// Takes one call back out, as [`Self::add`] put it in; its provider
    /// stays named, as the same call's own count names it.
    fn remove(&mut self, event: &Event) -> Result<(), &'static str> {
        fn minus(a: &mut u64, b: u64) -> Result<(), &'static str> {
            *a = a.checked_sub(b).ok_or("usage_total_inconsistent")?;
            Ok(())
        }
        let tokens = &event.tokens;
        let prompt = tokens
            .input
            .checked_add(tokens.cache_read)
            .and_then(|n| n.checked_add(tokens.cache_write))
            .ok_or("usage_total_overflow")?;
        for total in [
            &mut self.tokens,
            &mut self.price_bands[super::prices::prompt_band(prompt)],
        ] {
            minus(&mut total.input, tokens.input)?;
            minus(&mut total.output, tokens.output)?;
            minus(&mut total.cache_read, tokens.cache_read)?;
            minus(&mut total.cache_write, tokens.cache_write)?;
            minus(&mut total.cache_write_1h, tokens.cache_write_1h)?;
            if let Some(n) = tokens.reasoning {
                minus(total.reasoning.as_mut().ok_or("usage_total_inconsistent")?, n)?;
            }
        }
        minus(&mut self.calls, 1)?;
        match event.recorded_cost_micro_usd.filter(|_| !event.incomplete) {
            Some(cost) => minus(&mut self.recorded_cost_micro_usd, cost)?,
            None => minus(&mut self.missing_cost_calls, 1)?,
        }
        minus(&mut self.incomplete_calls, u64::from(event.incomplete))
    }

    pub fn new_tokens(&self) -> Result<u64, &'static str> {
        self.tokens
            .input
            .checked_add(self.tokens.output)
            .and_then(|n| n.checked_add(self.tokens.cache_write))
            .ok_or("usage_total_overflow")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Daily {
    pub device: String,
    pub agent: String,
    pub day: String,
    pub model: String,
    pub totals: Totals,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ingested {
    pub added: usize,
    pub repeated: usize,
    pub changed: usize,
    pub updated: usize,
    pub invalid: usize,
    pub expired: usize,
    pub limited: usize,
    /// The newest call this ingest added or rewrote, so a reader of recent
    /// calls only can tell whether it moved.
    pub newest_ms: Option<u64>,
}

pub struct Store {
    connection: Connection,
    /// The clock every step's budget is measured on.
    clock: fn() -> Instant,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, &'static str> {
        Self::open_with_clock(path, Instant::now)
    }

    /// A test that is not about the budgets holds this clock still.
    fn open_with_clock(path: &Path, clock: fn() -> Instant) -> Result<Self, &'static str> {
        hide_platform::fs::private::open_own_file(path, true)
            .map_err(|_| "usage_store_unavailable")?;
        let connection = Connection::open(path).map_err(failed)?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(failed)?;
        let store = Self { connection, clock };
        store.budget();
        let version: i64 = store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(failed)?;
        if !(0..=VERSION).contains(&version) {
            return Err("usage_store_newer");
        }
        store
            .connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; \
             PRAGMA max_page_count=131072; PRAGMA wal_autocheckpoint=256;",
            )
            .map_err(failed)?;
        if version == 0 {
            let tables: u64 = store
                .connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .map_err(failed)?;
            if tables != 0 {
                return Err("usage_store_schema");
            }
            store
                .connection
                .execute_batch(&format!("BEGIN;{SCHEMA}{CALLS_SCHEMA}COMMIT;"))
                .map_err(failed)?;
        } else if version == 1 {
            store.connection.execute_batch("BEGIN; CREATE TABLE price_cache(slot INTEGER PRIMARY KEY CHECK(slot=1), value TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(failed)?;
        }
        if version == 1 || version == 2 {
            store.connection.execute_batch("BEGIN; ALTER TABLE backfills ADD COLUMN limited INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=3; COMMIT;").map_err(failed)?;
        }
        let converted: bool = store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='calls')",
                [],
                |r| r.get(0),
            )
            .map_err(failed)?;
        if !converted {
            store.budget_of(CONVERT_BUDGET);
            store
                .connection
                .execute_batch(&format!("BEGIN;{CALLS_SCHEMA}{START_CONVERSION}COMMIT;"))
                .map_err(failed)?;
            store.budget();
        }
        store.connection.execute_batch(AI_SCHEMA).map_err(failed)?;
        Ok(store)
    }

    fn budget(&self) {
        self.budget_of(Duration::from_millis(250));
    }

    fn budget_of(&self, limit: Duration) {
        let clock = self.clock;
        let start = clock();
        self.connection
            .progress_handler(1000, Some(move || clock().duration_since(start) >= limit));
    }

    pub fn prices(&self) -> Result<super::prices::Cache, &'static str> {
        self.budget();
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM price_cache WHERE slot=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(failed)?;
        let cache: super::prices::Cache = match value {
            Some(value) if value.len() <= super::prices::CACHE_LIMIT => decode(&value)?,
            Some(_) => return Err("usage_prices_capacity"),
            None => return Ok(Default::default()),
        };
        cache.validate()?;
        Ok(cache)
    }

    pub fn save_prices(&mut self, cache: &super::prices::Cache) -> Result<(), &'static str> {
        cache.validate()?;
        let value = encode(cache)?;
        if value.len() > super::prices::CACHE_LIMIT {
            return Err("usage_prices_capacity");
        }
        self.budget();
        self.connection.execute("INSERT INTO price_cache VALUES(1,?1) ON CONFLICT(slot) DO UPDATE SET value=excluded.value", [value]).map_err(failed)?;
        Ok(())
    }

    /// A discarded event cannot be recovered from an advanced cursor. Keep
    /// that loss visible across discovery and restart, with the same commit.
    pub fn limited(&self, device: &str, agent: Agent) -> Result<bool, &'static str> {
        self.budget();
        self.connection
            .query_row(
                "SELECT limited FROM backfills WHERE device=?1 AND agent=?2",
                params![device, agent.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(failed)
    }

    pub fn checkpoint(
        &self,
        device: &str,
        agent: Agent,
        path: &str,
    ) -> Result<Option<(Candidate, Checkpoint)>, &'static str> {
        self.budget();
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT stamp,checkpoint FROM cursors WHERE device=?1 AND agent=?2 AND path=?3",
                params![device, agent.as_str(), path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(failed)?;
        row.map(|(stamp, checkpoint)| Ok((decode(&stamp)?, decode(&checkpoint)?)))
            .transpose()
    }

    /// Deleted native files need no cursor; their totals and dedup receipts
    /// remain. A capped or incomplete listing must never call this method.
    pub fn retire_cursors(
        &mut self,
        device: &str,
        agent: Agent,
        live: &[Candidate],
    ) -> Result<(), &'static str> {
        if live.len() > hide_session::usage::CANDIDATE_LIMIT {
            return Err("usage_candidate_capacity");
        }
        self.budget();
        let live: std::collections::HashSet<_> = live
            .iter()
            .map(|candidate| candidate.path.as_str())
            .collect();
        let tx = self.connection.transaction().map_err(failed)?;
        let paths = {
            let mut query = tx
                .prepare("SELECT path FROM cursors WHERE device=?1 AND agent=?2 LIMIT ?3")
                .map_err(failed)?;
            query
                .query_map(
                    params![
                        device,
                        agent.as_str(),
                        hide_session::usage::CANDIDATE_LIMIT + 1
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?
        };
        if paths.len() > hide_session::usage::CANDIDATE_LIMIT {
            return Err("usage_candidate_capacity");
        }
        for path in paths {
            if !live.contains(path.as_str()) {
                tx.execute(
                    "DELETE FROM cursors WHERE device=?1 AND agent=?2 AND path=?3",
                    params![device, agent.as_str(), path],
                )
                .map_err(failed)?;
            }
        }
        tx.commit().map_err(failed)
    }

    /// The checkpoint advances in the same commit as the dedup receipt and
    /// rollup. Losing a reply or replaying a copied file cannot add twice.
    pub fn ingest(
        &mut self,
        device: &str,
        agent: Agent,
        candidate: &Candidate,
        step: &Step,
        now: u64,
        zone: &jiff::tz::TimeZone,
    ) -> Result<Ingested, &'static str> {
        valid_name(device, 128)?;
        if agent == Agent::Cursor || step.events.len() > BATCH_LIMIT {
            return Err("usage_batch_invalid");
        }
        let path = candidate.path.as_str();
        valid_name(path, 4096)?;
        let stamp = encode(candidate)?;
        let checkpoint = encode(&step.checkpoint)?;
        if checkpoint.len() > CHECKPOINT_BYTES {
            return Err("usage_checkpoint_capacity");
        }
        self.budget();
        let tx = self.connection.transaction().map_err(failed)?;
        let known: Option<(i64, usize)> = tx
            .query_row(
                "SELECT id,calls FROM nodes WHERE device=?1",
                [device],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(failed)?;
        let (node, mut calls) = match known {
            Some(known) => known,
            None => {
                let n: usize = tx
                    .query_row("SELECT count(*) FROM nodes", [], |r| r.get(0))
                    .map_err(failed)?;
                if n >= DEVICE_LIMIT {
                    return Err("usage_device_capacity");
                }
                tx.execute("INSERT INTO nodes(device) VALUES(?1)", [device])
                    .map_err(failed)?;
                (tx.last_insert_rowid(), 0)
            }
        };
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cursors WHERE device=?1 AND agent=?2 AND path=?3)",
                params![device, agent.as_str(), path],
                |r| r.get(0),
            )
            .map_err(failed)?;
        if !known {
            let count: usize = tx
                .query_row(
                    "SELECT count(*) FROM cursors WHERE device=?1 AND agent=?2",
                    params![device, agent.as_str()],
                    |r| r.get(0),
                )
                .map_err(failed)?;
            if count >= hide_session::usage::CANDIDATE_LIMIT {
                return Err("usage_candidate_capacity");
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO backfills(device,agent) VALUES(?1,?2)",
            params![device, agent.as_str()],
        )
        .map_err(failed)?;
        let (completed, frozen_at): (bool, u64) = tx
            .query_row(
                "SELECT completed,frozen_at FROM backfills WHERE device=?1 AND agent=?2",
                params![device, agent.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(failed)?;
        let event_horizon = freeze_at(now, zone)?;
        let frozen_at = if completed {
            frozen_at.max(event_horizon)
        } else {
            frozen_at
        };
        tx.execute(
            "UPDATE backfills SET frozen_at=?3 WHERE device=?1 AND agent=?2",
            params![device, agent.as_str(), frozen_at],
        )
        .map_err(failed)?;
        let oldest = horizon(now, zone, DAILY_DAYS - 1)?;
        let mut outcome = Ingested::default();
        for event in &step.events {
            validate(event)?;
            if event.at_ms > now.saturating_add(DAY_MS) {
                outcome.invalid += 1;
                continue;
            }
            let key = key_bytes(&event.key)?;
            let previous = tx
                .query_row(STORED_CALL, params![node, agent.as_str(), key], |row| {
                    Ok((row.get::<_, String>(0)?, stored_call(row, 1)))
                })
                .optional()
                .map_err(failed)?;
            if let Some((day, original)) = previous {
                let original = original?;
                if same_accounting(&original, event) {
                    outcome.repeated += 1;
                    continue;
                }
                let merged = if agent == Agent::Claude {
                    stream::merge(&tx, device, node, &day, &original, event, frozen_at)?
                } else {
                    None
                };
                match merged {
                    // The stored call keeps its own time.
                    Some(stream::Merged::Rewrote) => {
                        outcome.updated += 1;
                        outcome.newest_ms = outcome.newest_ms.max(Some(original.at_ms));
                    }
                    // A later view holding no new maximum changes nothing.
                    Some(stream::Merged::Same) => outcome.repeated += 1,
                    None => outcome.changed += 1,
                }
                continue;
            }
            let day = local_day(event.at_ms, zone)?;
            if day < oldest || event.at_ms < frozen_at {
                outcome.expired += 1;
                continue;
            }
            if calls >= EVENT_LIMIT {
                outcome.limited += 1;
                continue;
            }
            let totals: Option<String> = tx
                .query_row(
                    "SELECT totals FROM daily WHERE device=?1 AND agent=?2 AND day=?3 AND model=?4",
                    params![device, agent.as_str(), day, event.model],
                    |r| r.get(0),
                )
                .optional()
                .map_err(failed)?;
            if totals.is_none() {
                let count: usize = tx
                    .query_row("SELECT count(*) FROM daily", [], |r| r.get(0))
                    .map_err(failed)?;
                if count >= DAILY_LIMIT {
                    outcome.limited += 1;
                    continue;
                }
            }
            let mut totals: Totals = totals.map(|v| decode(&v)).transpose()?.unwrap_or_default();
            totals.add(event)?;
            tx.execute("INSERT INTO daily VALUES(?1,?2,?3,?4,?5) ON CONFLICT(device,agent,day,model) DO UPDATE SET totals=excluded.totals",
                params![device,agent.as_str(),day,event.model,encode(&totals)?]).map_err(failed)?;
            insert_call(&tx, node, agent.as_str(), &day, event)?;
            calls += 1;
            outcome.added += 1;
            outcome.newest_ms = outcome.newest_ms.max(Some(event.at_ms));
        }
        tx.execute(
            "UPDATE nodes SET calls=?2 WHERE id=?1",
            params![node, calls],
        )
        .map_err(failed)?;
        if outcome.limited > 0 {
            tx.execute(
                "UPDATE backfills SET limited=1 WHERE device=?1 AND agent=?2",
                params![device, agent.as_str()],
            )
            .map_err(failed)?;
        }
        tx.execute("INSERT INTO cursors VALUES(?1,?2,?3,?4,?5) ON CONFLICT(device,agent,path) DO UPDATE SET stamp=excluded.stamp,checkpoint=excluded.checkpoint",
            params![device,agent.as_str(),path,stamp,checkpoint]).map_err(failed)?;
        tx.commit().map_err(failed)?;
        Ok(outcome)
    }

    pub fn daily(&self) -> Result<Vec<Daily>, &'static str> {
        self.budget();
        let mut query = self
            .connection
            .prepare(
                "SELECT device,agent,day,model,totals FROM daily ORDER BY day,agent,model LIMIT ?1",
            )
            .map_err(failed)?;
        let mut rows = query.query([DAILY_LIMIT + 1]).map_err(failed)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            let text: String = row.get(4).map_err(failed)?;
            out.push(Daily {
                device: row.get(0).map_err(failed)?,
                agent: row.get(1).map_err(failed)?,
                day: row.get(2).map_err(failed)?,
                model: row.get(3).map_err(failed)?,
                totals: decode(&text)?,
            });
            if out.len() > DAILY_LIMIT {
                return Err("usage_daily_capacity");
            }
        }
        Ok(out)
    }

    /// Runs `sql` on the ledger, for a test that breaks or mends a table.
    #[cfg(test)]
    pub(crate) fn execute_for_test(&self, sql: &str) {
        self.connection.execute_batch(sql).unwrap();
    }

    /// Hands `each` the own node's calls made at `since_ms` or later in
    /// `RECENT_CALLS` order, with their agent and assigned day, for the
    /// projects' costs (PRD lane-two-columns B31), and says whether more than
    /// `limit` were there. Each call is decoded, handed on and dropped, so nothing here
    /// grows with the calls read. It reads the 45-day `calls` table by
    /// `calls_node_at`, with a longer budget than a step's because it
    /// decodes every call, `each` included; the caller keeps the days it
    /// wants.
    pub fn recent_calls(
        &self,
        device: &str,
        since_ms: u64,
        limit: usize,
        mut each: impl FnMut(&str, &str, Event) -> Result<(), &'static str>,
    ) -> Result<bool, &'static str> {
        self.budget_of(Duration::from_secs(2));
        let mut query = self.connection.prepare(RECENT_CALLS).map_err(failed)?;
        let since = i64::try_from(since_ms).map_err(|_| "usage_timestamp_invalid")?;
        let mut rows = query
            .query(params![device, since, limit + 1])
            .map_err(failed)?;
        let mut read = 0;
        while let Some(row) = rows.next().map_err(failed)? {
            if read == limit {
                return Ok(true);
            }
            let text = |index| {
                row.get_ref(index)
                    .map_err(failed)?
                    .as_str()
                    .map_err(|_| "usage_store_decode")
            };
            each(text(0)?, text(1)?, stored_call(row, 2)?)?;
            read += 1;
        }
        Ok(false)
    }

    /// Completion is durable even when no events were found. Once completed,
    /// an old day is immutable even if every source checkpoint is lost.
    pub fn complete_backfill(
        &mut self,
        device: &str,
        agent: Agent,
        now: u64,
        zone: &jiff::tz::TimeZone,
    ) -> Result<(), &'static str> {
        valid_name(device, 128)?;
        self.budget();
        self.connection.execute("INSERT INTO backfills(device,agent,completed,frozen_at) VALUES(?1,?2,1,?3) ON CONFLICT(device,agent) DO UPDATE SET completed=1,frozen_at=max(frozen_at,excluded.frozen_at)", params![device,agent.as_str(),freeze_at(now,zone)?]).map_err(failed)?;
        self.prune(now, zone).map(|_| ())
    }

    pub fn backfill_complete(&self, device: &str, agent: Agent) -> Result<bool, &'static str> {
        self.budget();
        self.connection
            .query_row(
                "SELECT completed FROM backfills WHERE device=?1 AND agent=?2",
                params![device, agent.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(failed)
    }

    /// Expire whole days so a midnight boundary never loses only some of a
    /// day's dedup keys. First-backfill keys stay until completion.
    pub fn prune(&mut self, now: u64, zone: &jiff::tz::TimeZone) -> Result<bool, &'static str> {
        let oldest = horizon(now, zone, DAILY_DAYS - 1)?;
        let frozen = freeze_at(now, zone)?;
        self.budget();
        let tx = self.connection.transaction().map_err(failed)?;
        tx.execute(
            "UPDATE backfills SET frozen_at=max(frozen_at,?1) WHERE completed=1",
            [&frozen],
        )
        .map_err(failed)?;
        // A maintenance turn deletes at most 512 calls, 512 places and 512
        // rollups. Adjust the retained counter from those exact rows, never
        // COUNT an entire million-call node on each wake. A completed
        // backfill's `frozen_at` is at least `frozen` now, so its calls before
        // `frozen` are the expired ones.
        let nodes: Vec<(i64, String)> = {
            let mut query = tx
                .prepare("SELECT id,device FROM nodes ORDER BY id")
                .map_err(failed)?;
            query
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(failed)?
                .collect::<Result<_, _>>()
                .map_err(failed)?
        };
        let (mut expired, mut places) = (0, 0);
        for (node, device) in &nodes {
            if expired == 512 || places == 512 {
                break;
            }
            let keys: Vec<(String, Vec<u8>)> = {
                let mut query = tx
                    .prepare(
                        "SELECT agent,key FROM calls WHERE node=?1 AND at<?2 AND agent IN \
                         (SELECT agent FROM backfills WHERE device=?3 AND completed=1) LIMIT ?4",
                    )
                    .map_err(failed)?;
                query
                    .query_map(params![node, frozen, device, 512 - expired], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .map_err(failed)?
                    .collect::<Result<_, _>>()
                    .map_err(failed)?
            };
            for (agent, key) in &keys {
                tx.execute(
                    "DELETE FROM calls WHERE node=?1 AND agent=?2 AND key=?3",
                    params![node, agent, key],
                )
                .map_err(failed)?;
            }
            tx.execute(
                "UPDATE nodes SET calls=calls-?2 WHERE id=?1",
                params![node, keys.len()],
            )
            .map_err(failed)?;
            expired += keys.len();
            // A place whose newest call expired has no call left once its
            // agent's expired calls are all gone.
            places += tx
                .execute(
                    "DELETE FROM places WHERE id IN(SELECT p.id FROM places p WHERE p.node=?1 \
                     AND p.last_at<?2 AND p.agent IN(SELECT agent FROM backfills \
                     WHERE device=?3 AND completed=1) AND NOT EXISTS(SELECT 1 FROM calls c \
                     WHERE c.node=?1 AND c.agent=p.agent AND c.at<?2) LIMIT ?4)",
                    params![node, frozen, device, 512 - places],
                )
                .map_err(failed)?;
        }
        let days = tx
            .execute(
                "DELETE FROM daily WHERE rowid IN(SELECT rowid FROM daily WHERE day<?1 LIMIT 512)",
                [&oldest],
            )
            .map_err(failed)?;
        let ai_days = tx
            .execute(
                "DELETE FROM ai_daily WHERE rowid IN(SELECT rowid FROM ai_daily WHERE day<?1 LIMIT 512)",
                [&oldest],
            )
            .map_err(failed)?;
        // A call lost to the row cap on a day that is now gone no longer
        // makes the remaining days look incomplete.
        tx.execute(
            "UPDATE ai_state SET limited_day=NULL WHERE limited_day<?1",
            [&oldest],
        )
        .map_err(failed)?;
        tx.commit().map_err(failed)?;
        Ok(expired == 512 || places == 512 || days == 512 || ai_days == 512)
    }
}

impl Store {
    /// Whether calls in `events`, a version 3 store's or an older build's
    /// since, still wait to be moved ([`Self::convert`]). Until they are,
    /// nothing else reads or writes calls: a call still in `events` would
    /// not be found as already counted.
    pub fn converting(&self) -> Result<bool, &'static str> {
        self.budget();
        self.connection
            .query_row("SELECT EXISTS(SELECT 1 FROM events)", [], |r| r.get(0))
            .map_err(failed)
    }

    /// Moves the next calls in `events` into `calls`, at most
    /// [`CONVERT_BATCH`], in one transaction, and puts back the index an
    /// older build reads `events` by with the last of them; answers how
    /// many it moved. A step that fails or is ended changes nothing, so the
    /// next one, in this process or after a restart, takes the same calls
    /// again.
    pub fn convert(&mut self) -> Result<usize, &'static str> {
        self.budget_of(CONVERT_BUDGET);
        let moved = self.convert_batch();
        self.budget();
        moved
    }

    fn convert_batch(&mut self) -> Result<usize, &'static str> {
        let tx = self.connection.transaction().map_err(failed)?;
        type Old = (i64, String, String, String, String, String);
        let rows: Vec<Old> = {
            let mut query = tx
                .prepare(
                    "SELECT rowid,device,agent,key,day,event FROM events ORDER BY rowid LIMIT ?1",
                )
                .map_err(failed)?;
            query
                .query_map([CONVERT_BATCH], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(failed)?
                .collect::<Result<_, _>>()
                .map_err(failed)?
        };
        // Freed first, so the batch's new rows reuse the pages its old rows
        // leave in a store at its page limit.
        if let Some((last, ..)) = rows.last() {
            tx.execute("DELETE FROM events WHERE rowid<=?1", [last])
                .map_err(failed)?;
        }
        let mut nodes: HashMap<String, (i64, u64)> = HashMap::new();
        let mut moved: HashMap<String, u64> = HashMap::new();
        for (_, device, agent, key, day, event) in &rows {
            let node = match nodes.get_mut(device) {
                Some((node, _)) => *node,
                None => {
                    let node = node_of(&tx, device)?;
                    nodes.insert(device.clone(), (node, 0));
                    node
                }
            };
            *moved.entry(device.clone()).or_default() += 1;
            let mut event: Event = decode(event)?;
            // The key column was the call's identity; the copy inside the
            // event is the one that goes.
            event.key = key.clone();
            validate(&event)?;
            let held = tx
                .query_row(STORED_CALL, params![node, agent, key_bytes(key)?], |row| {
                    Ok((row.get::<_, String>(0)?, stored_call(row, 1)))
                })
                .optional()
                .map_err(failed)?;
            if let Some((held_day, held)) = held {
                // An older build counted a call this build had already
                // moved, which it could not find in `events`: its day gives
                // the count back, and the call is taken as a later view of
                // the one kept, as ingest takes one.
                take_back(&tx, device, agent, day, &event)?;
                if agent == Agent::Claude.as_str() {
                    let frozen_at: u64 = tx
                        .query_row(
                            "SELECT frozen_at FROM backfills WHERE device=?1 AND agent=?2",
                            params![device, agent],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(failed)?
                        .unwrap_or(0);
                    stream::merge(&tx, device, node, &held_day, &held?, &event, frozen_at)?;
                }
                continue;
            }
            insert_call(&tx, node, agent, day, &event)?;
            if let Some((_, inserted)) = nodes.get_mut(device) {
                *inserted += 1;
            }
        }
        for (device, (node, inserted)) in &nodes {
            tx.execute(
                "UPDATE nodes SET calls=calls+?2 WHERE id=?1",
                params![node, inserted],
            )
            .map_err(failed)?;
            tx.execute(
                "UPDATE devices SET calls=max(calls-?2,0) WHERE device=?1",
                params![device, moved[device]],
            )
            .map_err(failed)?;
        }
        if rows.len() < CONVERT_BATCH {
            tx.execute_batch("CREATE INDEX IF NOT EXISTS events_at ON events(at)")
                .map_err(failed)?;
        }
        tx.commit().map_err(failed)?;
        Ok(rows.len())
    }

    /// Whether more than half the file is free pages, as a conversion or a
    /// long quiet spell of pruning leaves it: SQLite reuses free pages but
    /// never gives them back on its own.
    pub fn compaction_due(&self) -> Result<bool, &'static str> {
        self.budget();
        let pages: u64 = self
            .connection
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(failed)?;
        let free: u64 = self
            .connection
            .pragma_query_value(None, "freelist_count", |r| r.get(0))
            .map_err(failed)?;
        Ok(free * 2 > pages)
    }

    /// Rewrites the file without its free pages and empties the write-ahead
    /// log, so the disk holds what the store keeps. One `VACUUM`: ended or
    /// failed, the store stays as it was.
    pub fn compact(&mut self) -> Result<(), &'static str> {
        self.budget_of(COMPACT_BUDGET);
        let compacted = self
            .connection
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(failed);
        self.budget();
        compacted
    }
}

/// The node `device` is, made when it is new, within [`DEVICE_LIMIT`]: an
/// older build may have counted calls of a device this build never saw.
fn node_of(tx: &rusqlite::Transaction<'_>, device: &str) -> Result<i64, &'static str> {
    if let Some(node) = tx
        .query_row("SELECT id FROM nodes WHERE device=?1", [device], |r| r.get(0))
        .optional()
        .map_err(failed)?
    {
        return Ok(node);
    }
    let n: usize = tx
        .query_row("SELECT count(*) FROM nodes", [], |r| r.get(0))
        .map_err(failed)?;
    if n >= DEVICE_LIMIT {
        return Err("usage_device_capacity");
    }
    tx.execute("INSERT INTO nodes(device) VALUES(?1)", [device])
        .map_err(failed)?;
    Ok(tx.last_insert_rowid())
}

/// Takes back from `day`'s totals what an older build added there for
/// `event`; a day already pruned has nothing to give back, and a day left
/// with no call goes, as this build would never have made it.
fn take_back(
    tx: &rusqlite::Transaction<'_>,
    device: &str,
    agent: &str,
    day: &str,
    event: &Event,
) -> Result<(), &'static str> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT totals FROM daily WHERE device=?1 AND agent=?2 AND day=?3 AND model=?4",
            params![device, agent, day, event.model],
            |r| r.get(0),
        )
        .optional()
        .map_err(failed)?;
    let Some(raw) = raw else {
        return Ok(());
    };
    let mut totals: Totals = decode(&raw)?;
    totals.remove(event)?;
    if totals.calls == 0 {
        tx.execute(
            "DELETE FROM daily WHERE device=?1 AND agent=?2 AND day=?3 AND model=?4",
            params![device, agent, day, event.model],
        )
    } else {
        tx.execute(
            "UPDATE daily SET totals=?5 WHERE device=?1 AND agent=?2 AND day=?3 AND model=?4",
            params![device, agent, day, event.model, encode(&totals)?],
        )
    }
    .map_err(failed)?;
    Ok(())
}

/// A call's key as the 32 bytes its 64 hex digits spell (`validate`
/// checked them).
fn key_bytes(key: &str) -> Result<Vec<u8>, &'static str> {
    (0..key.len())
        .step_by(2)
        .map(|at| {
            key.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or("usage_key_invalid")
        })
        .collect()
}

/// Stores `event` as one call of `node`'s `agent` counted on `day`, and its
/// place, which keeps the newest call's time.
fn insert_call(
    tx: &rusqlite::Transaction<'_>,
    node: i64,
    agent: &str,
    day: &str,
    event: &Event,
) -> Result<(), &'static str> {
    let place: i64 = tx
        .query_row(
            "INSERT INTO places(node,agent,session,cwd,last_at) VALUES(?1,?2,?3,?4,?5) \
             ON CONFLICT(node,agent,session,cwd) DO UPDATE SET last_at=max(last_at,excluded.last_at) \
             RETURNING id",
            params![
                node,
                agent,
                event.session,
                event.cwd.as_deref().unwrap_or(""),
                event.at_ms
            ],
            |r| r.get(0),
        )
        .map_err(failed)?;
    let tokens = &event.tokens;
    tx.execute(
        "INSERT INTO calls VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            node,
            agent,
            key_bytes(&event.key)?,
            event.at_ms,
            day,
            event.model,
            place,
            event.provider,
            tokens.input,
            tokens.output,
            tokens.cache_read,
            tokens.cache_write,
            tokens.cache_write_1h,
            tokens.reasoning,
            event.recorded_cost_micro_usd,
            event.incomplete,
        ],
    )
    .map_err(failed)?;
    Ok(())
}

/// The call whose columns start at `from` in `row`, in `STORED_CALL`'s
/// order: key, time, model, tokens, cost, incomplete, provider, session,
/// folder.
fn stored_call(row: &Row<'_>, from: usize) -> Result<Event, &'static str> {
    let at = |index: usize| from + index;
    let key: Vec<u8> = row.get(at(0)).map_err(failed)?;
    if key.len() != 32 {
        return Err("usage_store_decode");
    }
    let cwd: String = row.get(at(13)).map_err(failed)?;
    Ok(Event {
        key: key.iter().map(|byte| format!("{byte:02x}")).collect(),
        at_ms: row.get(at(1)).map_err(failed)?,
        model: row.get(at(2)).map_err(failed)?,
        tokens: Tokens {
            input: row.get(at(3)).map_err(failed)?,
            output: row.get(at(4)).map_err(failed)?,
            cache_read: row.get(at(5)).map_err(failed)?,
            cache_write: row.get(at(6)).map_err(failed)?,
            cache_write_1h: row.get(at(7)).map_err(failed)?,
            reasoning: row.get(at(8)).map_err(failed)?,
        },
        recorded_cost_micro_usd: row.get(at(9)).map_err(failed)?,
        incomplete: row.get(at(10)).map_err(failed)?,
        provider: row.get(at(11)).map_err(failed)?,
        session: row.get(at(12)).map_err(failed)?,
        cwd: (!cwd.is_empty()).then_some(cwd),
    })
}

/// Whether `next` bills what the stored call billed. A fork may change the
/// containing session or cwd; neither changes the billed call. A completed
/// call's accounting stays pinned while its call exists; the frozen-day
/// boundary protects it after expiry.
fn same_accounting(stored: &Event, next: &Event) -> bool {
    stored.at_ms == next.at_ms
        && stored.model == next.model
        && stored.tokens == next.tokens
        && stored.recorded_cost_micro_usd == next.recorded_cost_micro_usd
        && stored.provider == next.provider
        && stored.incomplete == next.incomplete
}

fn freeze_at(now: u64, zone: &jiff::tz::TimeZone) -> Result<u64, &'static str> {
    let day: jiff::civil::Date = horizon(now, zone, EVENT_DAYS as i64)?
        .parse()
        .map_err(|_| "usage_timestamp_invalid")?;
    let at = day
        .at(0, 0, 0, 0)
        .to_zoned(zone.clone())
        .map_err(|_| "usage_timestamp_invalid")?
        .timestamp()
        .as_millisecond();
    u64::try_from(at).map_err(|_| "usage_timestamp_invalid")
}

fn horizon(now: u64, zone: &jiff::tz::TimeZone, days: i64) -> Result<String, &'static str> {
    let today: jiff::civil::Date = local_day(now, zone)?
        .parse()
        .map_err(|_| "usage_timestamp_invalid")?;
    today
        .checked_sub(jiff::Span::new().days(days))
        .map(|d| d.to_string())
        .map_err(|_| "usage_timestamp_invalid")
}

pub(super) fn valid_name(text: &str, limit: usize) -> Result<(), &'static str> {
    if text.is_empty() || text.len() > limit || text.chars().any(char::is_control) {
        Err("usage_field_invalid")
    } else {
        Ok(())
    }
}

fn validate(event: &Event) -> Result<(), &'static str> {
    if event.key.len() != 64 || !event.key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("usage_key_invalid");
    }
    valid_name(&event.session, 128)?;
    valid_name(&event.model, 128)?;
    if let Some(cwd) = &event.cwd {
        valid_name(cwd, 4096)?;
    }
    if let Some(provider) = &event.provider {
        valid_name(provider, 128)?;
    }
    if event.tokens.cache_write_1h > event.tokens.cache_write
        || event
            .tokens
            .reasoning
            .is_some_and(|n| n > event.tokens.output)
    {
        return Err("usage_tokens_invalid");
    }
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<String, &'static str> {
    serde_json::to_string(value).map_err(|_| "usage_store_encode")
}

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, &'static str> {
    serde_json::from_str(text).map_err(|_| "usage_store_decode")
}

#[cfg(test)]
mod tests;
