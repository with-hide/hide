//! Events, daily totals and checkpoints are one transaction. After the first
//! backfill, days outside the event horizon are frozen and never added again.
//! A damaged store is refused, never rebuilt: its old daily totals may be
//! the only remaining record after an agent deletes its source history.

use std::path::Path;
use std::time::{Duration, Instant};

use hide_session::{
    Agent,
    usage::{Candidate, Checkpoint, Event, Step, Tokens},
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{DAILY_DAYS, DAILY_LIMIT, DAY_MS, EVENT_DAYS, EVENT_LIMIT, local_day};
mod ai;
pub use ai::{AiPart, AiRecorded, AiRow, AiUnknown};
mod stream;

const VERSION: i64 = 3;
const DEVICE_LIMIT: usize = 32;
const BATCH_LIMIT: usize = 8192;
const CHECKPOINT_BYTES: usize = 16 * 1024;
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
}

pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, &'static str> {
        hide_platform::fs::private::open_own_file(path, true)
            .map_err(|_| "usage_store_unavailable")?;
        let connection = Connection::open(path).map_err(failed)?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(failed)?;
        let store = Self { connection };
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
                .execute_batch(&format!("BEGIN;{SCHEMA}COMMIT;"))
                .map_err(failed)?;
        } else if version == 1 {
            store.connection.execute_batch("BEGIN; CREATE TABLE price_cache(slot INTEGER PRIMARY KEY CHECK(slot=1), value TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(failed)?;
        }
        if version == 1 || version == 2 {
            store.connection.execute_batch("BEGIN; ALTER TABLE backfills ADD COLUMN limited INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=3; COMMIT;").map_err(failed)?;
        }
        store.connection.execute_batch(AI_SCHEMA).map_err(failed)?;
        Ok(store)
    }

    fn budget(&self) {
        let start = Instant::now();
        self.connection.progress_handler(
            1000,
            Some(move || start.elapsed() >= Duration::from_millis(250)),
        );
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
        let calls: Option<usize> = tx
            .query_row("SELECT calls FROM devices WHERE device=?1", [device], |r| {
                r.get(0)
            })
            .optional()
            .map_err(failed)?;
        let mut calls = match calls {
            Some(n) => n,
            None => {
                let n: usize = tx
                    .query_row("SELECT count(*) FROM devices", [], |r| r.get(0))
                    .map_err(failed)?;
                if n >= DEVICE_LIMIT {
                    return Err("usage_device_capacity");
                }
                tx.execute("INSERT INTO devices(device) VALUES(?1)", [device])
                    .map_err(failed)?;
                0
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
            let fingerprint = fingerprint(event)?;
            if event.at_ms > now.saturating_add(DAY_MS) {
                outcome.invalid += 1;
                continue;
            }
            let previous: Option<(Vec<u8>, String)> = tx
                .query_row(
                    "SELECT fingerprint,event FROM events WHERE device=?1 AND agent=?2 AND key=?3",
                    params![device, agent.as_str(), event.key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(failed)?;
            if let Some((previous, original)) = previous {
                if previous == fingerprint {
                    outcome.repeated += 1;
                } else if agent == Agent::Claude
                    && stream::merge(&tx, device, &decode(&original)?, event, frozen_at)?
                {
                    outcome.updated += 1;
                } else {
                    outcome.changed += 1;
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
            tx.execute(
                "INSERT INTO events VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    device,
                    agent.as_str(),
                    event.key,
                    day,
                    fingerprint,
                    event.at_ms,
                    encode(event)?
                ],
            )
            .map_err(failed)?;
            calls += 1;
            outcome.added += 1;
        }
        tx.execute(
            "UPDATE devices SET calls=?2 WHERE device=?1",
            params![device, calls],
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

    /// The own node's calls made at `since_ms` or later, newest first, for
    /// the projects' costs (PRD lane-two-columns B31), and whether more than
    /// `limit` were there. It reads only the 45-day `events` table by its
    /// time index, with a longer budget than a step's because it decodes
    /// every call; the caller keeps the days it wants.
    pub fn recent_calls(
        &self,
        device: &str,
        since_ms: u64,
        limit: usize,
    ) -> Result<(Vec<super::project_cost::Call>, bool), &'static str> {
        let start = Instant::now();
        self.connection.progress_handler(
            1000,
            Some(move || start.elapsed() >= Duration::from_secs(2)),
        );
        let mut query = self
            .connection
            .prepare("SELECT agent,day,event FROM events WHERE device=?1 AND at>=?2 ORDER BY at DESC LIMIT ?3")
            .map_err(failed)?;
        let since = i64::try_from(since_ms).map_err(|_| "usage_timestamp_invalid")?;
        let mut rows = query
            .query(params![device, since, limit + 1])
            .map_err(failed)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            if out.len() == limit {
                return Ok((out, true));
            }
            let text: String = row.get(2).map_err(failed)?;
            out.push(super::project_cost::Call {
                agent: row.get(0).map_err(failed)?,
                day: row.get(1).map_err(failed)?,
                event: decode(&text)?,
            });
        }
        Ok((out, false))
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
        // A maintenance turn deletes at most 512 calls and 512 rollups.
        // Adjust the retained counter from those exact rows, never COUNT an
        // entire million-call device on each wake.
        let expired = {
            let mut query=tx.prepare("SELECT e.rowid,e.device FROM events e WHERE e.at<?1 AND EXISTS(SELECT 1 FROM backfills b WHERE b.device=e.device AND b.agent=e.agent AND b.completed=1 AND e.at<b.frozen_at) LIMIT 512").map_err(failed)?;
            query
                .query_map([frozen], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?
        };
        for (id, device) in &expired {
            tx.execute("DELETE FROM events WHERE rowid=?1", [id])
                .map_err(failed)?;
            tx.execute("UPDATE devices SET calls=calls-1 WHERE device=?1", [device])
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
        Ok(expired.len() == 512 || days == 512 || ai_days == 512)
    }
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

fn fingerprint(event: &Event) -> Result<Vec<u8>, &'static str> {
    // A fork may change the containing session or cwd; neither changes the
    // billed call. A completed call's accounting stays pinned while its
    // event exists; the frozen-day boundary protects it after expiry.
    Ok(Sha256::digest(
        encode(&(
            event.at_ms,
            &event.model,
            &event.tokens,
            event.recorded_cost_micro_usd,
            &event.provider,
            event.incomplete,
        ))?
        .as_bytes(),
    )
    .to_vec())
}

#[cfg(test)]
mod tests;
