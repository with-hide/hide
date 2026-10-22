//! One owned writer, reading only the core's own node. No runtime lock is
//! held while listing, parsing, transacting, pruning or building a report.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hide_node_link::{NodeLink, call_as, protocol::Call};
use hide_session::{
    Agent,
    usage::{Candidate, Candidates, Checkpoint, Step},
};

use super::ai_usage::AiUsageQueue;
use super::prices::{Cache, Fetch};
use super::project_cost::{self, ProjectCost, ProjectRoots};
use super::query::{self, Projection};
use super::store::{AiRow, Daily, Store};

const REST: Duration = Duration::from_millis(20);
/// Hide AI's queue is written at most this often, in one transaction.
const AI_BATCH: Duration = Duration::from_secs(1);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(30);
/// New calls make the projects' costs again at most this often.
const COST_REBUILD: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub agent: Agent,
    pub reading: bool,
    pub files_done: usize,
    pub files_total: usize,
    pub limited: bool,
    pub skipped: u64,
    pub failure: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub projection: Option<Arc<Projection>>,
    /// Each project's last seven days, for the projects the core named.
    pub projects: Arc<Vec<ProjectCost>>,
    pub progress: Vec<Progress>,
    pub failure: Option<&'static str>,
    pub price_failure: Option<&'static str>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    pub visible: bool,
    pub open: bool,
}

#[derive(Default)]
struct Mailbox {
    stop: bool,
    activity: Activity,
    /// The checkouts of each of this machine's projects, replaced whole.
    projects: Arc<Vec<ProjectRoots>>,
}

#[derive(Clone)]
pub struct Client(Arc<(Mutex<Mailbox>, Condvar)>);

impl Client {
    /// Two booleans replace prior intent; input and session-sync events do
    /// no ledger work and cannot grow a queue.
    pub fn activity(&self, activity: Activity) {
        let (lock, wake) = &*self.0;
        if let Ok(mut mailbox) = lock.lock()
            && mailbox.activity != activity
        {
            mailbox.activity = activity;
            wake.notify_one();
        }
    }

    /// The projects whose costs the report carries, replacing the last
    /// ones; an unchanged list wakes nothing.
    pub fn projects(&self, projects: Vec<ProjectRoots>) {
        let (lock, wake) = &*self.0;
        if let Ok(mut mailbox) = lock.lock()
            && *mailbox.projects != projects
        {
            mailbox.projects = Arc::new(projects);
            wake.notify_one();
        }
    }

    fn named_projects(&self) -> Arc<Vec<ProjectRoots>> {
        let (lock, _) = &*self.0;
        lock.lock()
            .map(|mailbox| Arc::clone(&mailbox.projects))
            .unwrap_or_default()
    }

    fn wait(&self, duration: Duration) -> Option<Activity> {
        let (lock, wake) = &*self.0;
        let guard = lock.lock().ok()?;
        if guard.stop {
            return None;
        }
        let (guard, _) = wake.wait_timeout(guard, duration).ok()?;
        (!guard.stop).then_some(guard.activity)
    }
}

pub struct Worker {
    client: Client,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        path: PathBuf,
        device: String,
        own: Arc<dyn NodeLink>,
        ai: Arc<AiUsageQueue>,
        mut publish: impl FnMut(Report) + Send + 'static,
    ) -> Result<Self, &'static str> {
        let client = Client(Arc::new((Mutex::new(Mailbox::default()), Condvar::new())));
        let worker_client = client.clone();
        let join = thread::Builder::new()
            .name("hide-usage-ledger".into())
            .spawn(move || {
                let mut previous = None;
                while let Some(activity) = worker_client.wait(Duration::from_millis(1)) {
                    match Engine::open(&path, device.clone(), Arc::clone(&ai)) {
                        Ok(mut engine) => {
                            let mut activity = activity;
                            let mut published = None;
                            loop {
                                // Stored whether or not a window is open; only
                                // a visible window makes it a new report.
                                engine.flush_ai(false);
                                if activity.visible {
                                    engine.prices_step(&own);
                                    engine.step(own.as_ref(), activity);
                                }
                                let interval =
                                    Duration::from_secs(if activity.open { 5 } else { 30 });
                                if published.is_none_or(|at: Instant| at.elapsed() >= interval)
                                    || (activity.visible && engine.ai_dirty)
                                {
                                    let report = engine
                                        .report(activity.visible, &worker_client.named_projects());
                                    if previous.as_ref() != Some(&report) {
                                        publish(report.clone());
                                        previous = Some(report);
                                    }
                                    published = Some(Instant::now());
                                }
                                let Some(next) = worker_client.wait(if activity.visible {
                                    REST
                                } else {
                                    Duration::from_secs(1)
                                }) else {
                                    engine.flush_ai(true);
                                    return;
                                };
                                activity = next;
                            }
                        }
                        Err(code) => {
                            let report = Report {
                                projection: None,
                                projects: Arc::default(),
                                progress: Vec::new(),
                                failure: Some(code),
                                price_failure: None,
                            };
                            if previous.as_ref() != Some(&report) {
                                diagnostic(code, None, 0);
                                publish(report.clone());
                                previous = Some(report);
                            }
                            if worker_client.wait(RETRY).is_none() {
                                return;
                            }
                        }
                    }
                }
            })
            .map_err(|_| "usage_worker_start_failed")?;
        Ok(Self {
            client,
            join: Some(join),
        })
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.client.0;
        if let Ok(mut guard) = lock.lock() {
            guard.stop = true;
            wake.notify_one();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct AgentWork {
    progress: Progress,
    queue: VecDeque<Candidate>,
    active: Option<(Candidate, Option<Checkpoint>)>,
    listed: Option<Instant>,
    failed: bool,
}

struct Engine {
    store: Store,
    device: String,
    agents: Vec<AgentWork>,
    next: usize,
    dirty: bool,
    daily: Arc<Vec<Daily>>,
    ai: Arc<AiUsageQueue>,
    ai_flushed: Instant,
    /// What the report was last built from. The store may hold newer rows
    /// (`ai_dirty`), which a visible window reads and a hidden one does not.
    ai_rows: Arc<Vec<AiRow>>,
    ai_limited: bool,
    ai_dirty: bool,
    failure: Option<&'static str>,
    pruned: Instant,
    pruning: bool,
    prune_retry: Option<Instant>,
    prices: Cache,
    price_fetch: Option<Fetch>,
    price_failure: Option<&'static str>,
    price_retry: Option<Instant>,
    costs: Costs,
}

/// The projects' costs as last built, and what they were built from.
#[derive(Default)]
struct Costs {
    value: Arc<Vec<ProjectCost>>,
    projects: Arc<Vec<ProjectRoots>>,
    today: String,
    built: Option<Instant>,
    /// New calls, a prune or a new price table since the last build.
    dirty: bool,
    failure: Option<&'static str>,
}

impl Engine {
    fn open(
        path: &std::path::Path,
        device: String,
        ai: Arc<AiUsageQueue>,
    ) -> Result<Self, &'static str> {
        let store = Store::open(path)?;
        let agents = Agent::supported()
            .filter(|agent| *agent != Agent::Cursor)
            .map(|agent| {
                Ok(AgentWork {
                    progress: Progress {
                        agent,
                        reading: !store.backfill_complete(&device, agent)?,
                        files_done: 0,
                        files_total: 0,
                        limited: store.limited(&device, agent)?,
                        skipped: 0,
                        failure: None,
                    },
                    queue: VecDeque::new(),
                    active: None,
                    listed: None,
                    failed: false,
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        let daily = Arc::new(store.daily()?);
        // A Hide AI table that cannot be read must not stop the session
        // ledger: it is named once and read again at the next write.
        let (ai_rows, ai_limited, ai_unread) = match (store.ai_rows(), store.ai_limited()) {
            (Ok(rows), Ok(limited)) => (rows, limited, false),
            (Err(code), _) | (_, Err(code)) => {
                diagnostic(code, None, 0);
                (Vec::new(), false, true)
            }
        };
        let ai_rows = Arc::new(ai_rows);
        let (prices, price_failure) = match store.prices() {
            Ok(prices) => (prices, None),
            Err(code) => {
                diagnostic(code, None, 0);
                (Cache::default(), Some(code))
            }
        };
        Ok(Self {
            store,
            device,
            agents,
            next: 0,
            dirty: false,
            daily,
            ai,
            ai_flushed: Instant::now(),
            ai_rows,
            ai_limited,
            ai_dirty: ai_unread,
            failure: None,
            pruned: Instant::now(),
            pruning: true,
            prune_retry: None,
            prices,
            price_fetch: None,
            price_failure,
            price_retry: None,
            costs: Costs {
                dirty: true,
                ..Costs::default()
            },
        })
    }

    fn prices_step(&mut self, own: &Arc<dyn NodeLink>) {
        if let Some(outcome) = self.price_fetch.as_mut().and_then(Fetch::take) {
            self.price_fetch = None;
            let outcome = outcome.and_then(|table| {
                let mut cache = self.prices.clone();
                cache.table = Some(table);
                self.store.save_prices(&cache)?;
                self.prices = cache;
                self.costs.dirty = true;
                Ok(())
            });
            self.price_failure = outcome.err();
            if let Some(code) = self.price_failure {
                diagnostic(code, None, 0);
            }
        }
        let now = crate::links::now_ms();
        if self.price_fetch.is_some()
            || !self.prices.due(now)
            || self.price_retry.is_some_and(|at| at.elapsed() < RETRY)
        {
            return;
        }
        // Persist the attempt before any network activity, including failed
        // attempts, so restarts cannot retry more than once per day.
        let mut attempted = self.prices.clone();
        attempted.attempted_at = Some(now);
        match self.store.save_prices(&attempted) {
            Ok(()) => {
                self.prices = attempted;
                match Fetch::start(Arc::clone(own)) {
                    Ok(fetch) => self.price_fetch = Some(fetch),
                    Err(code) => {
                        self.price_failure = Some(code);
                        diagnostic(code, None, 0);
                    }
                }
            }
            Err(code) => {
                self.price_retry = Some(Instant::now());
                if self.price_failure != Some(code) {
                    diagnostic(code, None, 0);
                }
                self.price_failure = Some(code);
            }
        }
    }

    /// One node read and one atomic store step per turn, followed by a rest.
    /// Round-robin selection prevents a large agent history starving another.
    fn step(&mut self, own: &dyn NodeLink, activity: Activity) {
        let now = crate::links::now_ms();
        let zone = jiff::tz::TimeZone::system();
        if (self.pruning || self.pruned.elapsed() >= Duration::from_secs(30 * 60))
            && self.prune_retry.is_none_or(|at| at.elapsed() >= RETRY)
        {
            match self.store.prune(now, &zone) {
                Ok(more) => {
                    self.pruning = more;
                    self.pruned = Instant::now();
                    self.dirty = true;
                    self.costs.dirty = true;
                    // Pruning also ages Hide AI rows out.
                    self.ai_dirty = true;
                    self.prune_retry = None;
                }
                Err(code) => {
                    self.prune_retry = Some(Instant::now());
                    self.failed(code);
                }
            }
            return;
        }
        let index = self.next;
        self.next = (self.next + 1) % self.agents.len();
        let work = &mut self.agents[index];
        let agent = work.progress.agent;
        let interval = Duration::from_secs(if activity.open { 3 } else { 30 });
        if work.active.is_none() && work.queue.is_empty() {
            if work
                .listed
                .is_some_and(|at| at.elapsed() < if work.failed { RETRY } else { interval })
            {
                return;
            }
            work.listed = Some(Instant::now());
            let listed: Result<Candidates, _> = call_as(
                own,
                Call::UsageFiles {
                    agent,
                    since_unix_ms: 0,
                },
                CALL_TIMEOUT,
            );
            match listed {
                Ok(listed) if listed.files.len() <= hide_session::usage::CANDIDATE_LIMIT => {
                    if !listed.capped
                        && listed.skipped == 0
                        && let Err(code) =
                            self.store
                                .retire_cursors(&self.device, agent, &listed.files)
                    {
                        work.fail(code);
                        return;
                    }
                    work.progress.files_done = 0;
                    work.progress.files_total = listed.files.len();
                    work.progress.limited = match self.store.limited(&self.device, agent) {
                        Ok(limited) => listed.capped || limited,
                        Err(code) => {
                            work.fail(code);
                            return;
                        }
                    };
                    work.progress.skipped = listed.skipped;
                    work.progress.failure = None;
                    work.failed = listed.capped || listed.skipped > 0;
                    if listed.capped {
                        diagnostic(
                            "usage_candidate_capacity",
                            Some(agent),
                            listed.files.len() as u64,
                        );
                    }
                    if listed.skipped > 0 {
                        diagnostic("usage_discovery_skipped", Some(agent), listed.skipped);
                    }
                    work.queue = listed.files.into();
                }
                Ok(_) => {
                    work.fail("usage_list_failed");
                    return;
                }
                Err(error) => {
                    work.fail_because("usage_list_failed", &error);
                    return;
                }
            }
            if work.queue.is_empty() {
                self.finish(index, now, &zone);
            }
            return;
        }
        if work.active.is_none() {
            let Some(candidate) = work.queue.pop_front() else {
                return;
            };
            let path = candidate.path.as_str();
            match self.store.checkpoint(&self.device, agent, path) {
                Ok(saved) => {
                    if agent != Agent::OpenCode
                        && saved.as_ref().is_some_and(|(stamp, checkpoint)| {
                            stamp == &candidate && checkpoint.is_finished()
                        })
                    {
                        work.progress.files_done += 1;
                        if work.queue.is_empty() {
                            self.finish(index, now, &zone);
                        }
                        return;
                    }
                    work.active = Some((candidate, saved.map(|(_, checkpoint)| checkpoint)));
                }
                Err(code) => {
                    work.fail(code);
                    return;
                }
            }
        }
        let (candidate, checkpoint) = work.active.as_ref().unwrap();
        let step: Result<Step, _> = call_as(
            own,
            Call::UsageRead {
                agent,
                path: candidate.path.clone(),
                checkpoint: checkpoint.clone(),
            },
            CALL_TIMEOUT,
        );
        match step {
            Ok(step) => match self
                .store
                .ingest(&self.device, agent, candidate, &step, now, &zone)
            {
                Ok(ingested) => {
                    self.dirty |= ingested.added > 0 || ingested.updated > 0;
                    self.costs.dirty |= ingested.added > 0 || ingested.updated > 0;
                    work.progress.limited |= ingested.limited > 0;
                    let skipped = step
                        .skipped
                        .values()
                        .copied()
                        .sum::<u64>()
                        .saturating_add(ingested.changed as u64)
                        .saturating_add(ingested.invalid as u64)
                        .saturating_add(ingested.expired as u64);
                    work.progress.skipped = work.progress.skipped.saturating_add(skipped);
                    if skipped > 0 {
                        diagnostic("usage_records_skipped", Some(agent), skipped);
                    }
                    if ingested.limited > 0 {
                        diagnostic("usage_event_capacity", Some(agent), ingested.limited as u64);
                    }
                    if step.has_more {
                        work.active.as_mut().unwrap().1 = Some(step.checkpoint);
                    } else {
                        work.active = None;
                        work.progress.files_done += 1;
                        if work.queue.is_empty() {
                            self.finish(index, now, &zone);
                        }
                    }
                }
                Err(code) => {
                    work.active = None;
                    work.fail(code);
                }
            },
            Err(error) => {
                work.active = None;
                work.fail_because("usage_read_failed", &error);
            }
        }
    }

    fn finish(&mut self, index: usize, now: u64, zone: &jiff::tz::TimeZone) {
        let work = &mut self.agents[index];
        if work.progress.reading && !work.failed {
            match self
                .store
                .complete_backfill(&self.device, work.progress.agent, now, zone)
            {
                Ok(()) => {
                    work.progress.reading = false;
                    self.pruning = true;
                }
                Err(code) => work.fail(code),
            }
        }
    }

    fn failed(&mut self, code: &'static str) {
        if self.failure != Some(code) {
            diagnostic(code, None, 0);
        }
        self.failure = Some(code);
    }

    /// Writes what the routers told since the last write. A failed write is a
    /// diagnostic with the calls it lost, not a screen state: the operator
    /// cannot act on it.
    fn flush_ai(&mut self, force: bool) {
        if !force && self.ai_flushed.elapsed() < AI_BATCH {
            return;
        }
        self.ai_flushed = Instant::now();
        let batch = self.ai.take(&jiff::tz::TimeZone::system());
        if batch.is_empty() {
            return;
        }
        for feature in &batch.unmapped {
            crate::diagnostic!(
                serde_json::json!({ "component": "usage_ledger", "kind": "ai_usage_feature_unmapped", "feature": feature })
            );
        }
        for (code, count) in [
            ("ai_usage_queue_full", batch.dropped),
            ("ai_usage_tokens_invalid", batch.invalid),
            ("ai_usage_day_invalid", batch.lost),
        ] {
            if count > 0 {
                diagnostic(code, None, count);
            }
        }
        if batch.parts.is_empty() && batch.unknown.is_empty() {
            return;
        }
        match self.store.ai_record(&batch.parts, &batch.unknown) {
            Ok(recorded) => {
                self.ai_dirty = true;
                if recorded.refused > 0 {
                    diagnostic("ai_usage_row_capacity", None, recorded.refused as u64);
                }
            }
            Err(code) => {
                // The batch is gone with the failed transaction, so every
                // failure names how many calls it lost.
                let lost = batch
                    .parts
                    .iter()
                    .filter(|part| part.starts_attempt)
                    .count() as u64
                    + batch.unknown.iter().map(|u| u.count).sum::<u64>();
                diagnostic(code, None, lost);
            }
        }
    }

    /// A report for a window that is `visible` or not. A hidden window's
    /// report keeps the Hide AI rows it last read, so a call alone makes no
    /// new report and nothing is published for it (B17); the next visible
    /// report reads the newest.
    /// The projects' costs, built again when the projects or the day
    /// changed, or new calls came at most every `COST_REBUILD`. A failed
    /// build is a diagnostic and keeps the last costs: the operator cannot
    /// act on it.
    fn project_costs(&mut self, projects: &Arc<Vec<ProjectRoots>>) -> Arc<Vec<ProjectCost>> {
        let now = crate::links::now_ms();
        let zone = jiff::tz::TimeZone::system();
        let Ok(today) = super::local_day(now, &zone) else {
            return Arc::clone(&self.costs.value);
        };
        let moved = *self.costs.projects != **projects || self.costs.today != today;
        let due = self.costs.dirty
            && self
                .costs
                .built
                .is_none_or(|at| at.elapsed() >= COST_REBUILD);
        if !moved && !due {
            return Arc::clone(&self.costs.value);
        }
        self.costs.projects = Arc::clone(projects);
        self.costs.today = today.clone();
        self.costs.built = Some(Instant::now());
        self.costs.dirty = false;
        if projects.is_empty() {
            self.costs.value = Arc::default();
            return Arc::default();
        }
        let built = today
            .parse::<jiff::civil::Date>()
            .map_err(|_| "usage_date_invalid")
            .and_then(|date| {
                // A day past the seven reaches back eight, so a time zone
                // change cannot drop the first day's earliest calls.
                let since = now.saturating_sub((project_cost::DAYS as u64 + 1) * super::DAY_MS);
                let (calls, limited) =
                    self.store
                        .recent_calls(&self.device, since, project_cost::EVENT_LIMIT)?;
                project_cost::prepare(
                    &calls,
                    limited,
                    projects,
                    self.prices.table.as_ref(),
                    date,
                    now,
                )
            });
        match built {
            Ok(value) => {
                if value.iter().any(|cost| cost.limited)
                    && self.costs.failure != Some("usage_project_cost_capacity")
                {
                    diagnostic(
                        "usage_project_cost_capacity",
                        None,
                        project_cost::EVENT_LIMIT as u64,
                    );
                    self.costs.failure = Some("usage_project_cost_capacity");
                } else if !value.iter().any(|cost| cost.limited) {
                    self.costs.failure = None;
                }
                self.costs.value = Arc::new(value);
            }
            Err(code) => {
                if self.costs.failure != Some(code) {
                    diagnostic(code, None, 0);
                }
                self.costs.failure = Some(code);
            }
        }
        Arc::clone(&self.costs.value)
    }

    fn report(&mut self, visible: bool, projects: &Arc<Vec<ProjectRoots>>) -> Report {
        if self.ai_dirty && visible {
            match (self.store.ai_rows(), self.store.ai_limited()) {
                (Ok(rows), Ok(limited)) => {
                    self.ai_rows = Arc::new(rows);
                    self.ai_limited = limited;
                    self.ai_dirty = false;
                }
                (Err(code), _) | (_, Err(code)) => {
                    // Read again at the next write, not on every turn.
                    self.ai_dirty = false;
                    self.failed(code);
                }
            }
        }
        if self.dirty {
            match self.store.daily() {
                Ok(daily) => {
                    self.daily = Arc::new(daily);
                    self.dirty = false;
                    self.failure = None;
                }
                Err(code) => self.failed(code),
            }
        }
        let progress: Vec<_> = self
            .agents
            .iter()
            .map(|work| work.progress.clone())
            .collect();
        let projection = match query::prepare(
            &self.daily,
            &query::Hide {
                rows: &self.ai_rows,
                limited: self.ai_limited,
            },
            &progress,
            self.prices.table.as_ref(),
            &self.device,
            crate::links::now_ms(),
            &jiff::tz::TimeZone::system(),
        ) {
            Ok(projection) => projection.map(Arc::new),
            Err(code) => {
                self.failed(code);
                None
            }
        };
        Report {
            projection,
            projects: self.project_costs(projects),
            progress,
            failure: self.failure,
            price_failure: self.price_failure,
        }
    }
}

impl AgentWork {
    fn fail(&mut self, code: &'static str) {
        self.fail_with(code, None);
    }

    /// A failed node call, with what the node said: a refusal's own code
    /// (`usage_read_Database`) or how the link failed, so the log tells a
    /// database error from a timeout.
    fn fail_because(&mut self, code: &'static str, error: &crate::node_access::LinkError) {
        self.fail_with(code, Some(crate::links::link_code(error, "node")));
    }

    fn fail_with(&mut self, code: &'static str, reason: Option<String>) {
        if self.progress.failure != Some(code) {
            let agent = self.progress.agent.as_str();
            crate::diagnostic!(serde_json::json!({
                "component": "usage_ledger", "kind": code, "agent": agent, "count": 0, "reason": reason,
            }));
        }
        self.progress.failure = Some(code);
        self.failed = true;
    }
}

fn diagnostic(code: &'static str, agent: Option<Agent>, count: u64) {
    crate::diagnostic!(
        serde_json::json!({ "component": "usage_ledger", "kind": code, "agent": agent.map(|agent| agent.as_str()), "count": count })
    );
}

#[cfg(test)]
mod tests;
