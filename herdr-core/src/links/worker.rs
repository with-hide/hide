//! The link worker: one named thread that holds the record's only write
//! connection and does every file and SQLite read and write for it outside
//! `Mutex<Runtime>` (D-27, D-29).
//!
//! Its mailbox holds the newest copy of each core fact (projects, panes,
//! parents) and the newest panel read; older ones are replaced, never queued
//! (D-40). Between them it reads changed session files from the core's own
//! node, one file and one read budget at a time, yielding after 20 ms so a
//! backfill never competes with input, and from every other connected node
//! a batch a minute.

use super::ledger::{
    LEDGER_QUEUE_LIMIT, LEDGER_RPC_LIMIT, LedgerAnswer, LedgerCall, LedgerContext, LedgerUpdate,
    SessionIdentity, SessionRecord, SessionSearchOptions, SessionSearchPage, SessionSearchPresence,
};
use super::observation::{self, DirectFact};
use super::store::{ArchivedSession, IssueLinks, LinkStore, LocalFiles, Opened, PrLinks};
use super::{
    BACKFILL_MS, PaneFact, ParentFact, ProjectFacts, ProjectLinkSummary, files_present, link_code,
    now_ms,
};
use crate::node_access::NodeLink;
use hide_session::links::{self, Candidate, ReadRequest};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How often the session folders are listed for changed files.
const LIST_EVERY: Duration = Duration::from_secs(15);
/// How often while a panel is open, so new activity reaches it in seconds
/// (B29); a listing is a walk of directory entries, not a read of files.
const LIST_OPEN_EVERY: Duration = Duration::from_secs(3);
/// How often the panes' spans are extended while they stay.
const PANES_EVERY: Duration = Duration::from_secs(60);
/// How often each connected device is asked for its changed files (D-21).
const DEVICE_EVERY: Duration = Duration::from_secs(60);
/// How long one device call may take before the next turn.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a device whose call failed is left alone before it is asked
/// again, so an unreachable helper costs one timeout a minute, not one a turn.
const DEVICE_RETRY: Duration = Duration::from_secs(60);
/// How often expired sessions are removed (D-18).
const PRUNE_EVERY: Duration = Duration::from_secs(30 * 60);
/// The longest a summary waits after a write, so a burst publishes once.
const SUMMARY_AFTER: Duration = Duration::from_secs(2);
/// One turn of file reading.
const TURN: Duration = Duration::from_millis(20);
/// The rest between two turns while files wait to be read.
const REST: Duration = Duration::from_millis(30);
/// A listing reaches back this far before the last one started, so a file
/// written while it ran is listed again.
const LIST_OVERLAP_MS: u64 = 60_000;

const BACKFILL_DONE: &str = "backfill_done";
/// When the own node's sessions were last listed; a device's is
/// `listed_at:<device>`.
pub(crate) const LISTED_AT: &str = "listed_at";

/// What the worker hands back to the runtime.
pub trait Sink: Send + 'static {
    fn direct_observations(&self, _facts: Vec<DirectFact>) {}
    fn record(&self, _generation: u64, _answer: Result<SessionRecord, String>) {}
    fn session_search(&self, _generation: u64, _answer: Result<SessionSearchPage, String>) {}
    /// Each project's counts and chips, by workspace id, when one changed.
    fn summaries(&self, summaries: BTreeMap<String, ProjectLinkSummary>);
    /// A panel read's answer; `generation` fences a superseded one.
    fn panel(&self, generation: u64, answer: Result<PanelAnswer, String>);
    /// Whether the first fill or a listing's reads are running (B24).
    fn filling(&self, filling: bool);
    /// The devices whose helper is connected now (D-21).
    fn devices(&self) -> Vec<(String, Arc<dyn NodeLink>)>;
    /// The sessions retention removed, and the archived ones it expired
    /// (PRD session-archive B31).
    fn pruned(&self, _sessions: Vec<SessionIdentity>) {}
    /// How a delete of named sessions' lines went (B29, B30).
    fn deleted(&self, _request: String, _answer: Result<(), String>) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelTarget {
    Pr(u64),
    Issue(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRequest {
    pub generation: u64,
    /// The project's record key.
    pub project: String,
    pub target: PanelTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAnswer {
    Pr(Option<PrLinks>),
    Issue(IssueLinks),
}

#[derive(Default)]
struct Mailbox {
    stop: bool,
    direct_shutdown_deadline: Option<Instant>,
    projects: Option<Arc<Vec<ProjectFacts>>>,
    panes: Option<Arc<Vec<PaneFact>>>,
    parents: Option<Arc<Vec<ParentFact>>>,
    panel: Option<PanelRequest>,
    /// The open panel closed: its reads stop following writes.
    panel_closed: bool,
    ledger: VecDeque<LedgerUpdate>,
    ledger_pending: usize,
    direct: VecDeque<PendingDirect>,
    direct_pending: usize,
    contexts: VecDeque<SessionIdentity>,
    record: Option<(u64, SessionIdentity)>,
    record_closed: bool,
    search: Option<SearchRequest>,
    calls: VecDeque<(
        LedgerCall,
        std::sync::mpsc::SyncSender<Result<LedgerAnswer, String>>,
    )>,
    calls_pending: usize,
    /// The sessions the archive holds, newest set only.
    archived: Option<Arc<Vec<ArchivedSession>>>,
    deletes: VecDeque<(String, Vec<SessionIdentity>)>,
}

struct PendingDirect {
    fact: DirectFact,
    queued: Instant,
}

struct SearchRequest {
    generation: u64,
    query: String,
    options: SessionSearchOptions,
    summaries: bool,
    presence: Vec<SessionSearchPresence>,
}

/// Deletes waiting at once; the archive's own delete is one per row.
const DELETE_QUEUE_LIMIT: usize = 64;

/// The runtime's handle on the worker's mailbox.
#[derive(Clone)]
pub struct LinkClient(
    Arc<(Mutex<Mailbox>, Condvar)>,
    Arc<Mutex<ContextCache>>,
    Arc<std::sync::atomic::AtomicU64>,
);

#[derive(Default)]
struct ContextCache {
    values: BTreeMap<SessionIdentity, LedgerContext>,
    used: VecDeque<SessionIdentity>,
}

impl LinkClient {
    fn request_stop(&self, now: Instant) {
        self.post(|mailbox| {
            mailbox.stop = true;
            mailbox
                .direct_shutdown_deadline
                .get_or_insert(now + observation::SHUTDOWN_DRAIN);
        });
    }

    fn stopping(&self) -> bool {
        self.0.0.lock().map_or(true, |mailbox| mailbox.stop)
    }

    /// Admission is bounded in memory. SQLite and publication belong to the worker.
    pub fn observe(&self, fact: DirectFact) -> Result<(), String> {
        fact.validate(now_ms())?;
        let (lock, wake) = &*self.0;
        let mut mailbox = lock.lock().map_err(|_| "links_worker_closed")?;
        if mailbox.stop {
            return Err("links_worker_closed".into());
        }
        if mailbox.ledger_pending + mailbox.direct_pending >= LEDGER_QUEUE_LIMIT {
            crate::diagnostic!(
                serde_json::json!({"component":"links","kind":"direct_observation.queue_capacity","limit":LEDGER_QUEUE_LIMIT})
            );
            return Err("direct_observation_queue_capacity".into());
        }
        mailbox.direct_pending += 1;
        mailbox.direct.push_back(PendingDirect {
            fact,
            queued: Instant::now(),
        });
        wake.notify_one();
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn take_project_facts_for_test(&self) -> Option<Arc<Vec<ProjectFacts>>> {
        self.0.0.lock().unwrap().projects.take()
    }
    #[cfg(test)]
    pub(crate) fn parent_facts_for_test(&self) -> Option<Arc<Vec<ParentFact>>> {
        self.0.0.lock().unwrap().parents.clone()
    }
    #[cfg(test)]
    pub(crate) fn prime_context_for_test(&self, session: SessionIdentity, context: LedgerContext) {
        let mut cache = self.1.lock().unwrap();
        cache.used.push_back(session.clone());
        cache.values.insert(session, context);
    }
    /// A mailbox that takes every update and keeps it for the test to read,
    /// with the ledger started at the epoch.
    #[cfg(test)]
    pub(crate) fn open_mailbox_for_test() -> Self {
        Self(
            Arc::default(),
            Arc::default(),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        )
    }
    #[cfg(test)]
    pub(crate) fn ledger_updates_for_test(&self) -> Vec<LedgerUpdate> {
        self.0.0.lock().unwrap().ledger.iter().cloned().collect()
    }
    #[cfg(test)]
    pub(crate) fn full_mailbox_for_test() -> Self {
        Self(
            Arc::new((
                Mutex::new(Mailbox {
                    ledger_pending: LEDGER_QUEUE_LIMIT,
                    ..Default::default()
                }),
                Condvar::new(),
            )),
            Arc::default(),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
        )
    }
    /// Only a background worker may wait for a reply. Admission is bounded
    /// and every mutation is committed before its acknowledgement.
    pub fn call(&self, call: LedgerCall) -> Result<LedgerAnswer, String> {
        // Metadata is a local read, never an external-effect acknowledgement.
        // Its failure must not hold other panes' native reads for 15 seconds.
        let timeout = if matches!(
            &call,
            LedgerCall::Metadata(_) | LedgerCall::SaveMetadata { .. }
        ) {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(15)
        };
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        {
            let (lock, wake) = &*self.0;
            let mut mailbox = lock.lock().map_err(|_| "links_worker_closed")?;
            if mailbox.stop {
                return Err("links_worker_closed".into());
            }
            if mailbox.calls_pending >= LEDGER_RPC_LIMIT {
                crate::diagnostic!(
                    serde_json::json!({"component":"links","kind":"ledger.rpc_capacity","limit":LEDGER_RPC_LIMIT})
                );
                return Err("ledger_rpc_capacity".into());
            }
            mailbox.calls_pending += 1;
            mailbox.calls.push_back((call, send));
            wake.notify_one();
        }
        receive
            .recv_timeout(timeout)
            .map_err(|_| "links_worker_timeout".to_owned())?
    }
    pub fn ledger_started_at(&self) -> Option<u64> {
        let at = self.2.load(std::sync::atomic::Ordering::Acquire);
        (at != u64::MAX).then_some(at)
    }
    pub fn ledger(&self, update: LedgerUpdate) -> Result<(), String> {
        let (lock, wake) = &*self.0;
        let mut mailbox = lock.lock().map_err(|_| "links_worker_closed")?;
        if mailbox.stop {
            return Err("links_worker_closed".to_owned());
        }
        if mailbox.ledger_pending + mailbox.direct_pending >= LEDGER_QUEUE_LIMIT {
            crate::diagnostic!(
                serde_json::json!({"component":"links","kind":"ledger.queue_capacity","session_id":update.session().session_id})
            );
            return Err("ledger_queue_capacity".to_owned());
        }
        mailbox.ledger_pending += 1;
        mailbox.ledger.push_back(update);
        wake.notify_one();
        Ok(())
    }

    pub fn context(&self, session: &SessionIdentity) -> Option<LedgerContext> {
        {
            let mut cache = self.1.lock().ok()?;
            if let Some(context) = cache.values.get(session).cloned() {
                cache.used.retain(|held| held != session);
                cache.used.push_back(session.clone());
                return Some(context);
            }
        }
        // A miss is unknown authority. Request a bounded worker read without
        // blocking the label worker, the runtime or a session-sync thread.
        self.post(|mailbox| {
            if mailbox.contexts.contains(session) {
                return;
            }
            if mailbox.contexts.len() >= LEDGER_QUEUE_LIMIT {
                crate::diagnostic!(serde_json::json!({"component":"links","kind":"ledger.context_request_capacity","limit":LEDGER_QUEUE_LIMIT}));
                return;
            }
            mailbox.contexts.push_back(session.clone());
        });
        None
    }

    pub fn read_record(&self, generation: u64, session: SessionIdentity) {
        self.post(|mailbox| {
            mailbox.record = Some((generation, session));
            mailbox.record_closed = false;
        });
    }

    pub fn close_record(&self) {
        self.post(|mailbox| {
            mailbox.record = None;
            mailbox.record_closed = true;
        });
    }

    pub fn search_sessions(
        &self,
        generation: u64,
        query: String,
        options: SessionSearchOptions,
        summaries: bool,
        presence: Vec<SessionSearchPresence>,
    ) {
        self.post(|mailbox| {
            mailbox.search = Some(SearchRequest {
                generation,
                query,
                options,
                summaries,
                presence,
            })
        });
    }
    fn post(&self, put: impl FnOnce(&mut Mailbox)) {
        let (lock, wake) = &*self.0;
        if let Ok(mut mailbox) = lock.lock() {
            put(&mut mailbox);
            wake.notify_one();
        }
    }

    fn native_origin_before(&self, read_started: u64) -> Option<u64> {
        let mailbox = self.0.0.lock().ok()?;
        (!mailbox.stop && mailbox.projects.is_none()).then_some(read_started)
    }

    pub fn projects(&self, projects: Arc<Vec<ProjectFacts>>) {
        self.post(|mailbox| mailbox.projects = Some(projects));
    }

    pub fn panes(&self, panes: Arc<Vec<PaneFact>>) {
        self.post(|mailbox| mailbox.panes = Some(panes));
    }

    pub fn parents(&self, parents: Arc<Vec<ParentFact>>) {
        self.post(|mailbox| mailbox.parents = Some(parents));
    }

    /// The sessions the archive holds now, which retention also answers for.
    pub fn archived(&self, archived: Arc<Vec<ArchivedSession>>) {
        self.post(|mailbox| mailbox.archived = Some(archived));
    }

    /// Removes the named sessions' lines; the answer reaches `Sink::deleted`.
    pub fn delete_sessions(
        &self,
        request: String,
        sessions: Vec<SessionIdentity>,
    ) -> Result<(), String> {
        let (lock, wake) = &*self.0;
        let mut mailbox = lock.lock().map_err(|_| "links_worker_closed")?;
        if mailbox.stop {
            return Err("links_worker_closed".into());
        }
        if mailbox.deletes.len() >= DELETE_QUEUE_LIMIT {
            return Err("links_delete_capacity".into());
        }
        mailbox.deletes.push_back((request, sessions));
        wake.notify_one();
        Ok(())
    }

    pub fn read(&self, request: PanelRequest) {
        self.post(|mailbox| {
            mailbox.panel = Some(request);
            mailbox.panel_closed = false;
        });
    }

    pub fn close_panel(&self) {
        self.post(|mailbox| {
            mailbox.panel = None;
            mailbox.panel_closed = true;
        });
    }
}

/// Where the worker reads and writes.
#[derive(Clone, Debug)]
pub struct Paths {
    pub store: PathBuf,
    /// The old session search file, only read: its `policy` holds each
    /// project's Copied history days, which set record retention.
    pub search: PathBuf,
    /// The core's own node, whose session files are read here.
    pub local_device: String,
}

pub struct LinkWorker {
    client: LinkClient,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for LinkWorker {
    fn drop(&mut self) {
        self.client.request_stop(Instant::now());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl LinkWorker {
    /// `own` answers for `paths.local_device`, the node the core runs on.
    pub fn spawn(paths: Paths, own: Arc<dyn NodeLink>, sink: impl Sink) -> Result<Self, String> {
        let client = LinkClient(
            Arc::new((Mutex::new(Mailbox::default()), Condvar::new())),
            Arc::default(),
            Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
        );
        let mailbox = client.clone();
        let join = thread::Builder::new()
            .name("hide-links".into())
            .spawn(move || run(&mailbox, &paths, own.as_ref(), &sink))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            join: Some(join),
        })
    }

    pub fn client(&self) -> LinkClient {
        self.client.clone()
    }
}

struct State {
    direct: VecDeque<PendingDirect>,
    direct_facts: Option<Vec<DirectFact>>,
    direct_dirty: bool,
    ledger: VecDeque<LedgerUpdate>,
    ledger_retry_at: Option<Instant>,
    record: Option<(u64, SessionIdentity)>,
    record_answer: Option<Result<SessionRecord, String>>,
    search: Option<SearchRequest>,
    projects: Arc<Vec<ProjectFacts>>,
    /// Projects posted but not yet written, kept while the store is closed.
    projects_due: bool,
    panes: Arc<Vec<PaneFact>>,
    /// Parents posted but not yet written.
    parents: Option<Arc<Vec<ParentFact>>>,
    panel: Option<PanelRequest>,
    panel_answer: Option<Result<PanelAnswer, String>>,
    summaries: BTreeMap<String, ProjectLinkSummary>,
    queue: VecDeque<Candidate>,
    /// The listing whose files are being read, until its last page drains.
    listing: Option<Listing>,
    /// Whether this machine's first fill has finished (D-17).
    filled: bool,
    /// Whether `filled` still has to be read from the store.
    filled_unknown: bool,
    /// Per device: its files waiting to be read, and when it was last listed.
    devices: HashMap<String, DeviceQueue>,
    /// When the connected devices were last asked for.
    devices_at: Option<Instant>,
    filling: bool,
    /// The last write's failure, while writes keep failing: a panel then
    /// says the record failed rather than showing what it could not keep.
    write_failure: Option<String>,
    dirty_at: Option<Instant>,
    listed: Option<Instant>,
    panes_at: Option<Instant>,
    pruned: Option<Instant>,
    /// The sessions the archive holds, for retention.
    archived: Arc<Vec<ArchivedSession>>,
}

impl State {
    fn new() -> Self {
        Self {
            direct: VecDeque::new(),
            direct_facts: None,
            direct_dirty: true,
            ledger: VecDeque::new(),
            ledger_retry_at: None,
            record: None,
            record_answer: None,
            search: None,
            projects: Arc::default(),
            projects_due: false,
            panes: Arc::default(),
            parents: None,
            panel: None,
            panel_answer: None,
            summaries: BTreeMap::new(),
            queue: VecDeque::new(),
            listing: None,
            filled: false,
            filled_unknown: true,
            devices: HashMap::new(),
            devices_at: None,
            filling: false,
            write_failure: None,
            dirty_at: None,
            listed: None,
            panes_at: None,
            pruned: None,
            archived: Arc::default(),
        }
    }

    /// Notes a write's outcome; a change of failure re-answers the panel.
    fn wrote(&mut self, result: Result<(), String>, what: &str) -> bool {
        let failure = result.err();
        if let Some(code) = &failure {
            log_write(code, what);
        }
        if failure != self.write_failure {
            self.write_failure = failure.clone();
            self.dirty_at.get_or_insert_with(Instant::now);
        }
        failure.is_none()
    }

    /// Whether a device has files to read and is not waiting out a failure.
    fn device_reading(&self) -> bool {
        self.devices.values().any(DeviceQueue::reading)
    }
}

/// One listing of changed files, read page by page: its time is recorded as
/// listed only once every page has been read without a failed write, so a
/// failure is listed again rather than skipped (D-17, B32).
struct Listing {
    started: u64,
    since: u64,
    /// The next page's end: a full page may hide older files.
    next: Option<u64>,
    failed: bool,
}

impl Listing {
    fn start(since: u64) -> Self {
        Self {
            started: now_ms(),
            since,
            next: None,
            failed: false,
        }
    }

    /// Takes one page, newest first, and sets where the next one ends.
    fn page(
        &mut self,
        page: &hide_node_link::readers::ReaderPage<Candidate>,
        until: Option<u64>,
        device: &str,
    ) {
        self.next = None;
        if page.refused > 0 {
            self.failed = true;
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "listing.reader_refused", "device_id": device,
                "files": page.refused,
            }));
        }
        if page.scanned < links::CANDIDATE_LIMIT {
            return;
        }
        let Some(oldest) = page.oldest_unix_ms else {
            self.failed = true;
            return;
        };
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "listing.capped", "device_id": device,
            "files": page.scanned, "until_unix_ms": oldest,
        }));
        // A page of files that all share one time cannot move on; what it
        // could not hold is left, and the log says so.
        if until != Some(oldest) {
            self.next = Some(oldest);
        }
    }
}

/// The files of one page not yet read at their current stamp, oldest first,
/// so a session's earlier file is read before the one that continues it.
fn unread(page: Vec<Candidate>, stamps: &HashMap<String, String>) -> VecDeque<Candidate> {
    page.into_iter()
        .rev()
        .filter(|candidate| stamps.get(&candidate.path) != Some(&candidate.stamp))
        .collect()
}

fn open_store(path: &Path) -> Result<LinkStore, String> {
    let (store, opened) = LinkStore::open(path).inspect_err(|code| {
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "store.open_failed", "code": code,
        }));
    })?;
    if opened == Opened::Rebuilt {
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "store.rebuilt", "code": "links_store_corrupt",
        }));
    }
    store.recover_issue_attempts()?;
    Ok(store)
}

/// Yield between SQLite operations, including when shutdown arrives during a write.
/// The clock and write boundary let the regression order shutdown without waiting on a timer.
fn direct_observation_turn(
    client: &LinkClient,
    state: &mut State,
    mut now: impl FnMut() -> Instant,
    mut write: impl FnMut(&DirectFact) -> Result<bool, String>,
) -> bool {
    let deadline = now() + TURN;
    for _ in 0..16 {
        if client.stopping() {
            return true;
        }
        if now() >= deadline {
            break;
        }
        let Some(pending) = state.direct.pop_front() else {
            break;
        };
        if now().saturating_duration_since(pending.queued) >= observation::PENDING_DEADLINE {
            log_direct("direct_observation_pending_expired", &pending.fact);
        } else {
            match write(&pending.fact) {
                Ok(changed) => state.direct_dirty |= changed,
                Err(code) => log_direct(&code, &pending.fact),
            }
        }
        if let Ok(mut mailbox) = client.0.0.lock() {
            mailbox.direct_pending = mailbox.direct_pending.saturating_sub(1);
        }
    }
    client.stopping()
}

fn run(client: &LinkClient, paths: &Paths, own: &dyn NodeLink, sink: &impl Sink) {
    // A store that cannot open (busy, a newer schema, a full disk) is tried
    // again every listing period; until then a panel says why.
    let mut store = open_store(&paths.store);
    let mut opened_at = Instant::now();
    let mut state = State::new();
    'work: loop {
        let (lock, wake) = &*client.0;
        let Ok(mut mailbox) = lock.lock() else { return };
        let wait = if store.is_ok() {
            next_wait(&state)
        } else {
            LIST_EVERY.saturating_sub(opened_at.elapsed())
        };
        if !mailbox.stop
            && mailbox.projects.is_none()
            && mailbox.panes.is_none()
            && mailbox.parents.is_none()
            && mailbox.panel.is_none()
            && !mailbox.panel_closed
            && mailbox.ledger.is_empty()
            && mailbox.direct.is_empty()
            && mailbox.record.is_none()
            && !mailbox.record_closed
            && mailbox.search.is_none()
            && mailbox.calls.is_empty()
            && mailbox.deletes.is_empty()
            && (store.is_err() || mailbox.contexts.is_empty())
            && wait > Duration::ZERO
        {
            mailbox = match wake.wait_timeout(mailbox, wait) {
                Ok((mailbox, _)) => mailbox,
                Err(_) => return,
            };
        }
        if mailbox.stop {
            state.direct.extend(std::mem::take(&mut mailbox.direct));
            let deadline = mailbox
                .direct_shutdown_deadline
                .expect("the worker owner sets a deadline when requesting stop");
            drop(mailbox);
            if let Ok(store) = store.as_mut() {
                while Instant::now() < deadline {
                    let Some(pending) = state.direct.pop_front() else {
                        break;
                    };
                    if pending.queued.elapsed() >= observation::PENDING_DEADLINE {
                        log_direct("direct_observation_pending_expired", &pending.fact);
                    } else if let Err(code) =
                        store.apply_direct_observation(&pending.fact, now_ms())
                    {
                        log_direct(&code, &pending.fact);
                    }
                }
            }
            for pending in state.direct {
                log_direct("direct_observation_shutdown_undrained", &pending.fact);
            }
            return;
        }
        let projects = mailbox.projects.take();
        let panes = mailbox.panes.take();
        let parents = mailbox.parents.take();
        let panel = mailbox.panel.take();
        let panel_closed = std::mem::take(&mut mailbox.panel_closed);
        let ledger = std::mem::take(&mut mailbox.ledger);
        let direct = std::mem::take(&mut mailbox.direct);
        let record = mailbox.record.take();
        let record_closed = std::mem::take(&mut mailbox.record_closed);
        let search = mailbox.search.take();
        let calls = std::mem::take(&mut mailbox.calls);
        let deletes = std::mem::take(&mut mailbox.deletes);
        if let Some(archived) = mailbox.archived.take() {
            state.archived = archived;
        }
        let contexts = if store.is_ok() {
            let count = mailbox.contexts.len().min(16);
            mailbox.contexts.drain(..count).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        drop(mailbox);
        // Admission counts both mailbox and worker-owned pending updates.
        state.ledger.extend(ledger);
        state.direct.extend(direct);
        // Expire even while the store cannot open. Admission never leaves an
        // unbounded lifetime behind a full disk or unavailable database.
        while state
            .direct
            .front()
            .is_some_and(|p| p.queued.elapsed() >= observation::PENDING_DEADLINE)
        {
            let pending = state.direct.pop_front().expect("front exists");
            log_direct("direct_observation_pending_expired", &pending.fact);
            if let Ok(mut mailbox) = client.0.0.lock() {
                mailbox.direct_pending = mailbox.direct_pending.saturating_sub(1);
            }
        }
        if record_closed {
            state.record = None;
            state.record_answer = None;
        }
        if let Some(record) = record {
            state.record = Some(record);
            state.record_answer = None;
        }
        if let Some(search) = search {
            state.search = Some(search);
        }

        if panel_closed {
            state.panel = None;
            state.panel_answer = None;
        }
        if store.is_err() && opened_at.elapsed() >= LIST_EVERY {
            store = open_store(&paths.store);
            opened_at = Instant::now();
            if store.is_ok() {
                // The facts posted while it was closed are the newest ones.
                state.panes_at = None;
                state.dirty_at.get_or_insert_with(Instant::now);
            }
        }
        if let Some(request) = panel {
            state.panel = Some(request);
            state.panel_answer = None;
        }
        if let Some(projects) = projects {
            state.projects = projects;
            state.projects_due = true;
        }
        // Facts are handed only when they change, so what arrives while the
        // store is closed waits for it.
        if let Some(panes) = panes {
            state.panes = panes;
            state.panes_at = None;
        }
        if let Some(parents) = parents {
            state.parents = Some(parents);
        }
        let store = match store.as_mut() {
            Ok(store) => store,
            Err(code) => {
                for (request, _) in deletes {
                    sink.deleted(request, Err(code.clone()));
                }
                for (_, answer) in calls {
                    let _ = answer.send(Err(code.clone()));
                    if let Ok(mut mailbox) = client.0.0.lock() {
                        mailbox.calls_pending = mailbox.calls_pending.saturating_sub(1);
                    }
                }
                if let Some((generation, _)) = state.record.as_ref() {
                    sink.record(*generation, Err(code.clone()));
                }
                if let Some(request) = state.search.take() {
                    sink.session_search(request.generation, Err(code.clone()));
                }
                if let Some(request) = state.panel.as_ref()
                    && state.panel_answer.is_none()
                {
                    let answer = Err(code.clone());
                    sink.panel(request.generation, answer.clone());
                    state.panel_answer = Some(answer);
                }
                continue;
            }
        };
        for (request, sessions) in deletes {
            let result = store.delete_sessions(&sessions);
            if let Err(code) = &result {
                log_write(code, "delete");
            } else {
                state.dirty_at.get_or_insert_with(Instant::now);
            }
            sink.deleted(request, result);
        }
        if direct_observation_turn(client, &mut state, Instant::now, |fact| {
            store.apply_direct_observation(fact, now_ms())
        }) {
            continue 'work;
        }
        let now = now_ms();
        let expired = state.direct_facts.as_ref().is_some_and(|facts| {
            facts
                .iter()
                .any(|fact| now.saturating_sub(fact.at_unix_ms) >= observation::RETENTION_MS)
        });
        if state.direct_dirty || expired {
            let result = store
                .prune_direct_observations(now)
                .and_then(|()| store.direct_observations(now));
            match result {
                Ok(facts) => {
                    if state.direct_facts.as_ref() != Some(&facts) {
                        sink.direct_observations(facts.clone());
                        state.direct_facts = Some(facts);
                    }
                    state.direct_dirty = false;
                }
                Err(code) => log_write(&code, "direct_observations"),
            }
        }
        for (call, answer) in calls {
            let result = store.ledger_call(call);
            if let Err(code) = &result {
                log_write(code, "ledger_call");
            }
            if matches!(&result, Ok(LedgerAnswer::Saved | LedgerAnswer::Attempt(_))) {
                state.dirty_at.get_or_insert_with(Instant::now);
            }
            let _ = answer.send(result);
            if let Ok(mut mailbox) = client.0.0.lock() {
                mailbox.calls_pending = mailbox.calls_pending.saturating_sub(1);
            }
        }
        if client.ledger_started_at().is_none() {
            let started = match store.meta("ledger_started_at") {
                Ok(Some(value)) => value
                    .parse::<u64>()
                    .map_err(|_| "ledger_start_invalid".to_owned()),
                Ok(None) => {
                    let now = now_ms();
                    store
                        .set_meta("ledger_started_at", &now.to_string())
                        .map(|()| now)
                }
                Err(code) => Err(code),
            };
            match started {
                Ok(at) => client.2.store(at, std::sync::atomic::Ordering::Release),
                Err(code) => log_write(&code, "ledger_start"),
            }
        }
        if state.filled_unknown {
            state.filled_unknown = false;
            state.filled = store.meta(BACKFILL_DONE).is_ok_and(|done| done.is_some());
        }
        // Lineage first: a context read beside it names the parent.
        if let Some(parents) = state.parents.take() {
            let children = store.apply_parents(&parents);
            for child in children.iter().flatten() {
                update_context(client, store, child);
            }
            if children.is_err() {
                // Kept for the next pass.
                state.parents.get_or_insert(parents);
            }
            state.wrote(children.map(|_| ()), "parents");
            state.dirty_at.get_or_insert_with(Instant::now);
        }
        if state.ledger_retry_at.is_none_or(|at| at <= Instant::now()) {
            for _ in 0..16 {
                let Some(update) = state.ledger.front() else {
                    break;
                };
                let session = update.session().clone();
                let result = store.apply_ledger_update(update);
                if let Err(code) = &result
                    && !code.starts_with("links_store_")
                {
                    log_write(code, "ledger_rejected");
                    state.ledger.pop_front();
                    if let Ok(mut mailbox) = client.0.0.lock() {
                        mailbox.ledger_pending = mailbox.ledger_pending.saturating_sub(1);
                    }
                    continue;
                }
                if !state.wrote(result, "ledger") {
                    state.ledger_retry_at = Some(Instant::now() + Duration::from_secs(1));
                    break;
                }
                state.ledger.pop_front();
                state.ledger_retry_at = None;
                if let Ok(mut mailbox) = client.0.0.lock() {
                    mailbox.ledger_pending = mailbox.ledger_pending.saturating_sub(1);
                }
                state.dirty_at.get_or_insert_with(Instant::now);
                update_context(client, store, &session);
            }
        }
        if std::mem::take(&mut state.projects_due) {
            let projects = Arc::clone(&state.projects);
            for project in projects.iter() {
                let result = store.apply_project(project, now_ms());
                state.wrote(result, "project");
            }
            let result = store.refresh_ledger_outputs();
            state.wrote(result, "ledger_outputs");
            state.dirty_at.get_or_insert_with(Instant::now);
        }
        if state.panes_at.is_none_or(|at| at.elapsed() >= PANES_EVERY) {
            if !state.panes.is_empty() {
                let result = store.apply_panes(&state.panes, now_ms());
                state.wrote(result, "panes");
                for pane in state.panes.iter() {
                    update_context(
                        client,
                        store,
                        &SessionIdentity {
                            device_id: pane.device_id.clone(),
                            agent: pane.agent.clone(),
                            session_id: pane.session_id.clone(),
                        },
                    );
                }
                state.dirty_at.get_or_insert_with(Instant::now);
            }
            state.panes_at = Some(Instant::now());
        }
        for session in contexts {
            update_context(client, store, &session);
        }
        if state.panel.is_some() && state.panel_answer.is_none() {
            answer_panel(store, own, paths, &mut state, sink);
        }
        if state.record.is_some() && state.record_answer.is_none() {
            answer_record(store, &mut state, sink);
        }
        if let Some(request) = state.search.take() {
            let answer = match &state.write_failure {
                Some(code) => Err(code.clone()),
                None => store.ledger_search_page(
                    &request.query,
                    &request.options,
                    request.summaries,
                    &request.presence,
                ),
            };
            sink.session_search(request.generation, answer);
        }
        if state.queue.is_empty() {
            let next_page = state.listing.as_ref().and_then(|listing| listing.next);
            if next_page.is_some()
                || (state.listing.is_none()
                    && state
                        .listed
                        .is_none_or(|at| at.elapsed() >= list_every(&state)))
            {
                list(store, own, paths, &mut state);
            }
        }
        read_turn(store, own, paths, &mut state, client);
        if state.queue.is_empty() {
            finish_listing(store, &mut state);
        }
        device_turn(store, sink, &mut state, client);
        // The spinner is the first fill's, never a later listing's (B24).
        let filling = (!state.filled && !state.queue.is_empty())
            || state
                .devices
                .values()
                .any(|device| !device.filled && device.reading());
        if filling != state.filling {
            state.filling = filling;
            sink.filling(filling);
        }
        if state.pruned.is_none_or(|at| at.elapsed() >= PRUNE_EVERY) {
            prune(store, own, paths, &state.archived, sink);
            state.pruned = Some(Instant::now());
            state.dirty_at.get_or_insert_with(Instant::now);
        }
        if state
            .dirty_at
            .is_some_and(|at| at.elapsed() >= SUMMARY_AFTER)
        {
            state.dirty_at = None;
            publish_summaries(store, &mut state, sink);
            if state.panel.is_some() {
                answer_panel(store, own, paths, &mut state, sink);
            }
            if state.record.is_some() {
                answer_record(store, &mut state, sink);
            }
        }
    }
}

fn list_every(state: &State) -> Duration {
    if state.panel.is_some() || state.record.is_some() {
        LIST_OPEN_EVERY
    } else {
        LIST_EVERY
    }
}

fn next_wait(state: &State) -> Duration {
    if !state.direct.is_empty() {
        return REST;
    }
    if !state.ledger.is_empty() && state.ledger_retry_at.is_none_or(|at| at <= Instant::now()) {
        return REST;
    }
    // A listing with another page to take goes on at once, even when the
    // page before held nothing new.
    let paging = state
        .listing
        .as_ref()
        .is_some_and(|listing| listing.next.is_some())
        || state.devices.values().any(|device| {
            device.retry_at.is_none_or(|at| at <= Instant::now())
                && device
                    .listing
                    .as_ref()
                    .is_some_and(|listing| listing.next.is_some())
        });
    if !state.queue.is_empty() || state.device_reading() || paging {
        return REST;
    }
    let every = list_every(state);
    let mut wait = every;
    if let Some(at) = state.ledger_retry_at {
        wait = wait.min(at.saturating_duration_since(Instant::now()));
    }
    if let Some(at) = state.dirty_at {
        wait = wait.min(SUMMARY_AFTER.saturating_sub(at.elapsed()));
    }
    if let Some(at) = state.listed {
        wait = wait.min(every.saturating_sub(at.elapsed()));
    }
    for device in state.devices.values() {
        let waiting = !device.queue.is_empty()
            || device
                .listing
                .as_ref()
                .is_some_and(|listing| listing.next.is_some());
        if let Some(at) = device.retry_at
            && waiting
        {
            wait = wait.min(at.saturating_duration_since(Instant::now()));
        }
    }
    wait
}

fn log_write(code: &str, what: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "links", "kind": "store.write_failed", "what": what, "code": code,
    }));
}

fn log_direct(code: &str, fact: &DirectFact) {
    crate::diagnostic!(
        serde_json::json!({"component":"links","kind":"direct_observation.rejected","code":code,"observation_id":fact.id(),"device_id":fact.sender.device_id,"pane_id":fact.sender.pane_id})
    );
}

/// Lists one page of the session files changed since the last listing (the
/// last 90 days the first time, D-17) and queues the ones whose stamp moved.
fn list(store: &LinkStore, own: &dyn NodeLink, paths: &Paths, state: &mut State) {
    state.listed = Some(Instant::now());
    if state.listing.is_none() {
        let since = match (store.meta(BACKFILL_DONE), store.meta(LISTED_AT)) {
            (Ok(Some(_)), Ok(Some(at))) => at
                .parse::<u64>()
                .unwrap_or(0)
                .saturating_sub(LIST_OVERLAP_MS),
            (Ok(_), Ok(_)) => now_ms().saturating_sub(BACKFILL_MS),
            (Err(code), _) | (_, Err(code)) => {
                log_write(&code, "listing");
                return;
            }
        };
        state.listing = Some(Listing::start(since));
    }
    let Some(listing) = state.listing.as_mut() else {
        return;
    };
    let until = listing.next;
    let page = match hide_node_link::readers::link_files(own, listing.since, until, DEVICE_TIMEOUT)
        .map_err(|error| link_code(&error, "node"))
    {
        Ok(page) => page,
        Err(code) => {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "listing.failed", "code": code,
            }));
            listing.failed = true;
            listing.next = None;
            return;
        }
    };
    listing.page(&page, until, &paths.local_device);
    match store.stamps(&paths.local_device) {
        Ok(stamps) => state.queue = unread(page.rows, &stamps),
        Err(code) => {
            log_write(&code, "listing");
            listing.failed = true;
            listing.next = None;
        }
    }
}

/// Once a listing's last page has drained, records its time as listed and
/// the first fill as done, unless something in it failed (D-17, B32).
fn finish_listing(store: &LinkStore, state: &mut State) {
    let Some(listing) = state.listing.take_if(|listing| listing.next.is_none()) else {
        return;
    };
    if listing.failed {
        return;
    }
    if let Err(code) = store.set_meta(LISTED_AT, &listing.started.to_string()) {
        log_write(&code, "listing");
        return;
    }
    if !state.filled {
        if let Err(code) = store.set_meta(BACKFILL_DONE, "1") {
            log_write(&code, "backfill");
            return;
        }
        state.filled = true;
        crate::diagnostic!(serde_json::json!({"component": "links", "kind": "backfill.done"}));
    }
}

/// Reads queued files for one turn: one read budget per file per step, a
/// file with more waiting goes back to the front.
fn read_turn(
    store: &mut LinkStore,
    own: &dyn NodeLink,
    paths: &Paths,
    state: &mut State,
    client: &LinkClient,
) {
    if state.queue.is_empty() {
        return;
    }
    let turn = Instant::now();
    while turn.elapsed() < TURN {
        let Some(candidate) = state.queue.pop_front() else {
            break;
        };
        if let Err(error) = hide_node_link::link::check_reader_support(
            own,
            candidate.agent,
            hide_node_link::sessions::ReaderFeature::Links,
        ) {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "session.reader_refused",
                "agent": candidate.agent.as_str(), "code": link_code(&error, "node"),
            }));
            if let Some(listing) = state.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        let checkpoint = match store.checkpoint(&paths.local_device, &candidate.path) {
            Ok(checkpoint) => checkpoint,
            Err(code) => {
                log_write(&code, "cursor");
                if let Some(listing) = state.listing.as_mut() {
                    listing.failed = true;
                }
                continue;
            }
        };
        let read_started = now_ms();
        let read = hide_node_link::readers::links(
            own,
            vec![ReadRequest {
                agent: candidate.agent,
                path: candidate.path.clone(),
                checkpoint,
            }],
            DEVICE_TIMEOUT,
        )
        .map_err(|error| link_code(&error, "node"));
        let batch = match read {
            Ok(batch) => batch,
            Err(code) => {
                // The node itself failed, not the file: the listing is read
                // again from its start on its next turn.
                crate::diagnostic!(serde_json::json!({
                    "component": "links", "kind": "node.read_failed", "code": code,
                }));
                state.queue.clear();
                if let Some(listing) = state.listing.as_mut() {
                    listing.failed = true;
                    listing.next = None;
                }
                return;
            }
        };
        if batch.ignored_answers > 0 {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "node.reader_answers_refused", "answers": batch.ignored_answers,
            }));
        }
        if let Some(refusal) = batch.refused.first() {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "session.reader_refused",
                "agent": refusal.agent.as_str(), "code": refusal.reason.code(),
            }));
            if let Some(listing) = state.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        let Some(answer) = batch.answers.into_iter().next() else {
            if let Some(listing) = state.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        };
        if let Some(code) = answer.error.as_deref() {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "session.read_failed",
                "agent": candidate.agent.as_str(), "code": code,
                "session_id": answer.facts.session_id,
            }));
        }
        let more = answer.has_more && answer.error.is_none();
        let result = store.apply_answer_observed(
            &paths.local_device,
            &answer,
            &candidate.stamp,
            client.native_origin_before(read_started),
        );
        if !state.wrote(result, "session") {
            if let Some(listing) = state.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        refresh_reader_context(client, store, &paths.local_device, &answer);
        state.dirty_at.get_or_insert_with(Instant::now);
        if more {
            state.queue.push_front(candidate);
        }
    }
}

#[derive(Default)]
struct DeviceQueue {
    queue: VecDeque<Candidate>,
    listing: Option<Listing>,
    listed: Option<Instant>,
    /// After a failed call, when the device is asked again.
    retry_at: Option<Instant>,
    /// Whether its first listing has been read through (B24).
    filled: bool,
}

impl DeviceQueue {
    fn reading(&self) -> bool {
        !self.queue.is_empty() && self.retry_at.is_none_or(|at| at <= Instant::now())
    }

    fn failed(&mut self) {
        self.retry_at = Some(Instant::now() + DEVICE_RETRY);
    }
}

/// One step for each connected device: list its changed files every
/// [`DEVICE_EVERY`], then read up to a few of them per turn through its
/// helper. A device that is gone keeps every row it gave; when it returns,
/// its listing starts where the last one did, so what changed meanwhile is
/// read then (B40). A failed call leaves the device for [`DEVICE_RETRY`].
fn device_turn(store: &mut LinkStore, sink: &impl Sink, state: &mut State, client: &LinkClient) {
    let paging = state.devices.values().any(|device| {
        device
            .listing
            .as_ref()
            .is_some_and(|listing| listing.next.is_some())
    });
    if !state.device_reading()
        && !paging
        && state
            .devices_at
            .is_some_and(|at| at.elapsed() < DEVICE_EVERY)
    {
        return;
    }
    state.devices_at = Some(Instant::now());
    let devices = sink.devices();
    state
        .devices
        .retain(|id, _| devices.iter().any(|(device, _)| device == id));
    for (device, channel) in devices {
        let entry = state
            .devices
            .entry(device.clone())
            .or_insert_with(|| DeviceQueue {
                // A device listed through before is not filling for the first time.
                filled: store
                    .meta(&format!("{LISTED_AT}:{device}"))
                    .is_ok_and(|at| at.is_some()),
                ..DeviceQueue::default()
            });
        if entry.retry_at.is_some_and(|at| at > Instant::now()) {
            continue;
        }
        entry.retry_at = None;
        let next_page = entry.listing.as_ref().and_then(|listing| listing.next);
        if entry.queue.is_empty()
            && (next_page.is_some()
                || (entry.listing.is_none()
                    && entry.listed.is_none_or(|at| at.elapsed() >= DEVICE_EVERY)))
        {
            entry.listed = Some(Instant::now());
            if let Err(code) = list_device(store, &device, channel.as_ref(), entry) {
                entry.failed();
                log_device(&device, "listing", &code);
            }
        }
        if !entry.queue.is_empty() {
            read_device(
                store,
                &device,
                channel.as_ref(),
                entry,
                &mut state.write_failure,
                client,
            );
            state.dirty_at.get_or_insert_with(Instant::now);
        }
        if entry.queue.is_empty() {
            finish_device(store, &device, entry);
        }
    }
}

fn read_device(
    store: &mut LinkStore,
    device: &str,
    channel: &dyn NodeLink,
    entry: &mut DeviceQueue,
    write_failure: &mut Option<String>,
    client: &LinkClient,
) {
    let batch = entry
        .queue
        .drain(..entry.queue.len().min(links::READ_FILE_LIMIT))
        .collect::<Vec<_>>();
    let mut requests = Vec::with_capacity(batch.len());
    for candidate in &batch {
        if let Err(error) = hide_node_link::link::check_reader_support(
            channel,
            candidate.agent,
            hide_node_link::sessions::ReaderFeature::Links,
        ) {
            log_device(device, "reader", &link_code(&error, "device_helper"));
            if let Some(listing) = entry.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        match store.checkpoint(device, &candidate.path) {
            Ok(checkpoint) => requests.push(ReadRequest {
                agent: candidate.agent,
                path: candidate.path.clone(),
                checkpoint,
            }),
            Err(code) => {
                log_write(&code, "cursor");
                if let Some(listing) = entry.listing.as_mut() {
                    listing.failed = true;
                }
            }
        }
    }
    let read_started = now_ms();
    let answers = match hide_node_link::readers::links(channel, requests, DEVICE_TIMEOUT) {
        Ok(answers) => answers,
        Err(error) => {
            // The files wait for the device's next try.
            for candidate in batch.into_iter().rev() {
                entry.queue.push_front(candidate);
            }
            entry.failed();
            log_device(device, "read", &link_code(&error, "device_helper"));
            return;
        }
    };
    if answers.ignored_answers > 0 {
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "device.reader_answers_refused", "device_id": device, "answers": answers.ignored_answers,
        }));
    }
    for candidate in batch {
        if let Some(refusal) = answers
            .refused
            .iter()
            .find(|refusal| refusal.path == candidate.path && refusal.agent == candidate.agent)
        {
            log_device(device, "reader", refusal.reason.code());
            if let Some(listing) = entry.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        let Some(answer) = answers
            .answers
            .iter()
            .find(|answer| answer.path == candidate.path && answer.agent == candidate.agent)
        else {
            if let Some(listing) = entry.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        };
        if let Some(code) = answer.error.as_deref() {
            log_device(device, "session", code);
        }
        if let Err(code) = store.apply_answer_observed(
            device,
            answer,
            &candidate.stamp,
            client.native_origin_before(read_started),
        ) {
            log_write(&code, "session");
            *write_failure = Some(code);
            if let Some(listing) = entry.listing.as_mut() {
                listing.failed = true;
            }
            continue;
        }
        *write_failure = None;
        refresh_reader_context(client, store, device, answer);
        if answer.has_more && answer.error.is_none() {
            entry.queue.push_back(candidate);
        }
    }
}

fn list_device(
    store: &LinkStore,
    device: &str,
    channel: &dyn NodeLink,
    entry: &mut DeviceQueue,
) -> Result<(), String> {
    if entry.listing.is_none() {
        let since = match store.meta(&format!("{LISTED_AT}:{device}"))? {
            Some(at) => at
                .parse::<u64>()
                .unwrap_or(0)
                .saturating_sub(LIST_OVERLAP_MS),
            None => now_ms().saturating_sub(BACKFILL_MS),
        };
        entry.listing = Some(Listing::start(since));
    }
    let Some(listing) = entry.listing.as_mut() else {
        return Ok(());
    };
    let until = listing.next;
    // A page that could not be listed is files not read: the listing is not
    // recorded as listed through, and the next one starts where it did.
    let listed = hide_node_link::readers::link_files(channel, listing.since, until, DEVICE_TIMEOUT)
        .map_err(|error| link_code(&error, "device_helper"))
        .and_then(|page| Ok((store.stamps(device)?, page)));
    let (stamps, page) = listed.inspect_err(|_| {
        listing.failed = true;
        listing.next = None;
    })?;
    listing.page(&page, until, device);
    entry.queue = unread(page.rows, &stamps);
    Ok(())
}

/// The device's counterpart of [`finish_listing`].
fn finish_device(store: &LinkStore, device: &str, entry: &mut DeviceQueue) {
    let Some(listing) = entry.listing.take_if(|listing| listing.next.is_none()) else {
        return;
    };
    if listing.failed {
        return;
    }
    match store.set_meta(
        &format!("{LISTED_AT}:{device}"),
        &listing.started.to_string(),
    ) {
        Ok(()) => entry.filled = true,
        Err(code) => log_write(&code, "listing"),
    }
}

fn log_device(device: &str, what: &str, code: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "links", "kind": "device.read_failed", "device_id": device,
        "what": what, "code": code,
    }));
}

fn prune(
    store: &mut LinkStore,
    own: &dyn NodeLink,
    paths: &Paths,
    archived: &[ArchivedSession],
    sink: &impl Sink,
) {
    let policies = match store.policies(&paths.search) {
        Ok(policies) => policies,
        Err(code) => {
            // An unreadable policy is not Off: nothing is removed this pass.
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "policy.read_failed", "code": code,
            }));
            return;
        }
    };
    let days = |project: &str| policies.get(project).copied();
    let mut present = |asked: &[String]| files_present(own, asked);
    let now = now_ms();
    let mut gone = match store.prune(&days, &paths.local_device, &mut present, now) {
        Ok(removed) => {
            if !removed.is_empty() {
                crate::diagnostic!(serde_json::json!({
                    "component": "links", "kind": "retention.pruned", "sessions": removed.len(),
                }));
            }
            removed
        }
        Err(code) => {
            log_write(&code, "retention");
            return;
        }
    };
    match store.expired_archived(archived, &days, now) {
        Ok(expired) => gone.extend(expired),
        Err(code) => log_write(&code, "retention_archived"),
    }
    if !gone.is_empty() {
        sink.pruned(gone);
    }
}

fn publish_summaries(store: &LinkStore, state: &mut State, sink: &impl Sink) {
    let mut summaries = BTreeMap::new();
    for project in state.projects.iter() {
        match store.summary(&project.key, &project.worktrees) {
            Ok(mut summary) => {
                match store.ledger_issue_refs(&project.key) {
                    Ok(refs) => summary.record_issues = refs,
                    Err(code) => {
                        log_write(&code, "ledger_issue_refs");
                        if let Some(held) = state.summaries.get(&project.workspace_id) {
                            summary.record_issues = held.record_issues.clone();
                        }
                    }
                }
                match store.ledger_summaries(&project.key) {
                    Ok(work) => {
                        match store.ledger_creators(&work) {
                            Ok(creators) => summary.creators = creators,
                            Err(code) => {
                                log_write(&code, "ledger_creators");
                                if let Some(held) = state.summaries.get(&project.workspace_id) {
                                    summary.creators = held.creators.clone();
                                }
                            }
                        }
                        summary.work = work;
                    }
                    Err(code) => {
                        log_write(&code, "ledger_summary");
                        if let Some(held) = state.summaries.get(&project.workspace_id) {
                            summary.work = held.work.clone();
                            summary.creators = held.creators.clone();
                        }
                    }
                }
                summaries.insert(project.workspace_id.clone(), summary);
            }
            Err(code) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "links", "kind": "summary.failed",
                    "workspace_id": project.workspace_id, "code": code,
                }));
                if let Some(held) = state.summaries.get(&project.workspace_id) {
                    summaries.insert(project.workspace_id.clone(), held.clone());
                }
            }
        }
    }
    if summaries != state.summaries {
        state.summaries = summaries.clone();
        sink.summaries(summaries);
    }
}

/// A read that recorded requests changes which of them a label still owes a
/// summary, so the session's cached context is read again.
fn refresh_reader_context(
    client: &LinkClient,
    store: &LinkStore,
    device: &str,
    answer: &links::ReadAnswer,
) {
    if answer.facts.requests.is_empty() {
        return;
    }
    match store.ledger_file_session(device, &answer.path) {
        Ok(Some(session)) => update_context(client, store, &session),
        Ok(None) => {}
        Err(code) => log_write(&code, "ledger_reader_context"),
    }
}

fn update_context(client: &LinkClient, store: &LinkStore, session: &SessionIdentity) {
    match store.ledger_context(session) {
        Ok(context) => {
            if let Ok(mut contexts) = client.1.lock() {
                if !contexts.values.contains_key(session)
                    && contexts.values.len() >= LEDGER_QUEUE_LIMIT
                {
                    if let Some(first) = contexts.used.pop_front() {
                        contexts.values.remove(&first);
                    }
                    crate::diagnostic!(
                        serde_json::json!({"component":"links","kind":"ledger.context_capacity"})
                    );
                }
                contexts.used.retain(|held| held != session);
                contexts.used.push_back(session.clone());
                contexts.values.insert(session.clone(), context);
            }
        }
        Err(code) => log_write(&code, "ledger_context"),
    }
}

fn answer_record(store: &LinkStore, state: &mut State, sink: &impl Sink) {
    let Some((generation, session)) = &state.record else {
        return;
    };
    let answer = match state.write_failure.as_ref() {
        Some(code) => Err(code.clone()),
        None => store.ledger_record(session),
    };
    if state.record_answer.as_ref() != Some(&answer) {
        sink.record(*generation, answer.clone());
        state.record_answer = Some(answer);
    }
}

fn answer_panel(
    store: &LinkStore,
    own: &dyn NodeLink,
    paths: &Paths,
    state: &mut State,
    sink: &impl Sink,
) {
    let Some(request) = state.panel.as_ref() else {
        return;
    };
    let present = |asked: &[String]| files_present(own, asked);
    let local = Some(LocalFiles {
        device: &paths.local_device,
        present: &present,
    });
    let answer = if let Some(code) = &state.write_failure {
        Err(code.clone())
    } else {
        match &request.target {
            PanelTarget::Pr(number) => store
                .pr_panel(&request.project, *number, local)
                .map(PanelAnswer::Pr),
            PanelTarget::Issue(key) => store
                .issue_panel(&request.project, key, local)
                .map(PanelAnswer::Issue),
        }
    };
    if let Err(code) = &answer {
        crate::diagnostic!(serde_json::json!({
            "component": "links", "kind": "panel.read_failed", "code": code,
            "generation": request.generation,
        }));
    }
    if state.panel_answer.as_ref() != Some(&answer) {
        state.panel_answer = Some(answer.clone());
        sink.panel(request.generation, answer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{FileState, PrFact, ProjectFacts, SessionRole, WorktreeFact};
    use crate::node_access::{LinkAnswer, LinkError};
    use hide_node_link::protocol::Call;
    use std::sync::atomic::{AtomicBool, Ordering};

    const CREATED: u64 = 1_790_000_000_000;

    #[test]
    fn observation_stop_during_commit_starts_no_second_write_or_new_drain_budget() {
        use crate::links::observation::DirectOutcome;
        use crate::model::GraphParty;
        use std::cell::Cell;

        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
        let client = LinkClient::open_mailbox_for_test();
        let at = now_ms();
        for index in 0..16 {
            client
                .observe(DirectFact {
                    source_server: "local-server".into(),
                    call_key: format!("call-{index}"),
                    sender: GraphParty {
                        device_id: "local".into(),
                        pane_id: "sender".into(),
                        session: "a".repeat(64),
                    },
                    recipient: None,
                    at_unix_ms: at,
                    outcome: DirectOutcome::Unknown,
                    reason: None,
                })
                .unwrap();
        }
        let mut state = State::new();
        state.direct = std::mem::take(&mut client.0.0.lock().unwrap().direct);
        let stopped_at = Instant::now();
        let clock = Cell::new(stopped_at);
        let mut writes = 0;
        let stopped = direct_observation_turn(
            &client,
            &mut state,
            || clock.get(),
            |fact| {
                let answer = store.apply_direct_observation(fact, at);
                writes += 1;
                // Order shutdown inside the real write boundary without a busy timeout or sleep.
                client.request_stop(stopped_at);
                clock.set(stopped_at + observation::SHUTDOWN_DRAIN);
                answer
            },
        );
        assert!(stopped);
        assert_eq!(writes, 1);
        assert_eq!(store.direct_observations(at).unwrap().len(), 1);
        assert_eq!(state.direct.len(), 15);
        client.request_stop(clock.get());
        assert_eq!(
            client.0.0.lock().unwrap().direct_shutdown_deadline,
            Some(clock.get())
        );
        let pending = state.direct.front().unwrap().fact.clone();
        assert_eq!(client.observe(pending), Err("links_worker_closed".into()));
    }

    /// A Claude Code session file that printed pull request 7.
    fn printed_a_pull_request(home: &Path, id: &str, at: &str) -> String {
        let folder = home.join(".claude/projects/-work-app");
        std::fs::create_dir_all(&folder).unwrap();
        let lines = [
            serde_json::json!({"type": "user", "uuid": format!("{id}-u"), "parentUuid": null,
                "message": {"role": "user", "content": "Open the pull request"},
                "timestamp": at, "origin": {"kind": "human"}, "userType": "external",
                "promptId": "p", "cwd": "/work/app", "sessionId": id, "gitBranch": "feat/x"}),
            serde_json::json!({"type": "pr-link", "sessionId": id, "prNumber": 7,
                "prRepository": "acme/app", "prUrl": "https://github.com/acme/app/pull/7",
                "timestamp": at}),
        ];
        let path = folder.join(format!("{id}.jsonl"));
        std::fs::write(&path, lines.map(|line| format!("{line}\n")).concat()).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn read_to_the_end(store: &mut LinkStore, home: &Path, path: &str) -> links::ReadAnswer {
        let answer = links::read(
            home,
            &[links::ReadRequest {
                agent: hide_session::Agent::Claude,
                path: path.to_owned(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert!(answer.error.is_none() && !answer.has_more);
        store.apply_answer("local", &answer, "stamp").unwrap();
        answer
    }

    /// The ledger's links can name a session it holds no record of: a Pi or
    /// omp child is named by its file until its reader reads it, a spawn is
    /// known before its file is read, and retention removes a
    /// session while its file's cursor stays. Each was a write failure on
    /// every read answer that reached it, about one every two seconds, and
    /// the failure kept the parent's work from the turns that continue it.
    #[test]
    fn a_session_the_ledger_never_recorded_fails_no_write_and_stops_no_context() {
        use crate::labels::analysis::LabelEnd;
        use crate::links::ledger::{RecordedRequest, TurnJudgment, WorkAuthor};
        let home = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
        let session = |id: &str| SessionIdentity {
            device_id: "local".into(),
            agent: "claude".into(),
            session_id: id.into(),
        };
        // A session retention removed: its file's cursor still names it.
        let removed = printed_a_pull_request(home.path(), "removed", "2026-07-01T00:00:00Z");
        let removed_answer = read_to_the_end(&mut store, home.path(), &removed);
        let mut present = |_: &[String]| Ok(std::collections::BTreeSet::new());
        assert_eq!(
            store
                .prune(&|_| None, "local", &mut present, CREATED + 91 * 86_400_000)
                .unwrap()
                .len(),
            1
        );
        // A root with open work, a child whose file printed a pull request,
        // and a Pi child its spawn named by its file.
        store
            .apply_ledger_update(&LedgerUpdate::Requests {
                session: session("root"),
                cwd: "/work/app".into(),
                path: None,
                requests: vec![RecordedRequest {
                    offset: 1,
                    at_unix_ms: CREATED,
                    text: "Build the release".into(),
                    author: WorkAuthor::Operator,
                    ..RecordedRequest::default()
                }],
            })
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::Chapter {
                session: session("root"),
                goal: "Ship the release".into(),
                at_unix_ms: CREATED,
                change: crate::links::ledger::FlowChange::New,
            })
            .unwrap();
        store
            .apply_ledger_update(&LedgerUpdate::TurnEnd {
                session: session("root"),
                offset: 1,
                started_at_unix_ms: CREATED + 1,
                at_unix_ms: CREATED + 1,
                end: LabelEnd::Working,
                work_author: true,
                line: String::new(),
                turn_witness: None,
                judgment: TurnJudgment::default(),
            })
            .unwrap();
        let child = printed_a_pull_request(home.path(), "child", "2026-09-27T00:00:00Z");
        let pi = SessionIdentity {
            device_id: "local".into(),
            agent: "pi".into(),
            session_id: "/operator/.pi/agent/sessions/--work-app--/2026-10-10_01a1.jsonl".into(),
        };
        let parented = |child: &SessionIdentity| ParentFact {
            device_id: child.device_id.clone(),
            agent: child.agent.clone(),
            session_id: child.session_id.clone(),
            parent_device_id: "local".into(),
            parent_agent: "claude".into(),
            parent_session_id: "root".into(),
            parent_name: "lead".into(),
        };
        store
            .apply_parents(&[parented(&session("child")), parented(&pi)])
            .unwrap();
        let child_answer = read_to_the_end(&mut store, home.path(), &child);

        let client = LinkClient::full_mailbox_for_test();
        let ((), records) = crate::diagnostics::capture(|| {
            refresh_reader_context(&client, &store, "local", &removed_answer);
            refresh_reader_context(&client, &store, "local", &child_answer);
            update_context(&client, &store, &pi);
        });
        assert!(
            !records
                .iter()
                .any(|record| record["kind"] == "store.write_failed"),
            "{records:?}"
        );
        let unrecorded = client.context(&pi).unwrap();
        assert_eq!(unrecorded.parent_name.as_deref(), Some("lead"));
        assert!(unrecorded.owed.is_empty());
        assert!(
            client
                .context(&session("removed"))
                .is_none_or(|context| context.owed.is_empty())
        );
        assert_eq!(
            store
                .ledger_record(&session("root"))
                .unwrap()
                .children
                .iter()
                .map(|record| record.session.session_id.as_str())
                .collect::<Vec<_>>(),
            ["child"],
            "the record lists the child it holds"
        );
    }

    /// A connected device whose helper answers the two link calls from its
    /// own home, the way `hided node serve` does.
    struct Device {
        home: PathBuf,
        connected: AtomicBool,
    }

    impl NodeLink for Device {
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            static READERS: std::sync::LazyLock<hide_node_link::sessions::ReaderFeatures> =
                std::sync::LazyLock::new(hide_node_link::sessions::ReaderFeatures::implemented);
            self.connected.load(Ordering::SeqCst).then_some(&READERS)
        }
        fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            if !self.connected.load(Ordering::SeqCst) {
                return Err(LinkError::NotConnected("gone".into()));
            }
            let value = match call {
                Call::LinkFiles {
                    since_unix_ms,
                    until_unix_ms,
                } => serde_json::to_value(
                    links::candidates(&self.home, since_unix_ms, until_unix_ms).unwrap(),
                ),
                Call::LinkRead { requests } => {
                    serde_json::to_value(links::read(&self.home, &requests))
                }
                other => panic!("unexpected call {other:?}"),
            };
            Ok(LinkAnswer::Parsed(value.unwrap()))
        }
    }

    struct ConnectionChangesDuringRead {
        device: Device,
        client: LinkClient,
        observations: Vec<Arc<Vec<ProjectFacts>>>,
    }

    impl NodeLink for ConnectionChangesDuringRead {
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            self.device.reader_features()
        }
        fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
            let reading = matches!(call, Call::LinkRead { .. });
            let answer = self.device.call(call, timeout)?;
            if reading {
                for facts in &self.observations {
                    self.client.projects(Arc::clone(facts));
                }
            }
            Ok(answer)
        }
    }

    #[test]
    fn a_connection_edit_during_native_intake_cannot_certify_the_old_repository() {
        intake_connection_changes(false);
    }

    #[test]
    fn an_a_to_b_to_a_edit_during_native_intake_cannot_certify_the_old_repository() {
        intake_connection_changes(true);
    }

    fn intake_connection_changes(return_to_original: bool) {
        let directory = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        device_session(home.path());
        let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
        let project = |repository: &str, connection_epoch| ProjectFacts {
            key: hide_project::project_id("mini", Path::new("/mini/app")),
            device_id: "mini".into(),
            root: "/mini/app".into(),
            repository: Some(repository.into()),
            connection_epoch,
            ..Default::default()
        };
        store
            .apply_project(&project("acme/original", 0), CREATED - 70_000)
            .unwrap();
        let client = LinkClient::full_mailbox_for_test();
        let mut observations = vec![Arc::new(vec![project("acme/new", 1)])];
        if return_to_original {
            observations.push(Arc::new(vec![project("acme/original", 2)]));
        }
        let node = ConnectionChangesDuringRead {
            device: Device {
                home: home.path().into(),
                connected: AtomicBool::new(true),
            },
            client: client.clone(),
            observations,
        };
        let candidates = links::candidates(home.path(), 0, None).unwrap();
        assert_eq!(candidates.len(), 1);
        let mut queue = DeviceQueue {
            queue: candidates.into(),
            ..Default::default()
        };
        let mut failure = None;
        read_device(&mut store, "mini", &node, &mut queue, &mut failure, &client);
        assert_eq!(failure, None);
        let identity = SessionIdentity {
            device_id: "mini".into(),
            agent: "claude".into(),
            session_id: "s-mini".into(),
        };
        assert_eq!(store.ledger_record(&identity).unwrap().repository, None);
        // Applying the newest connection and replaying cannot promote unknown.
        let latest = client.0.0.lock().unwrap().projects.take().unwrap();
        for facts in latest.iter() {
            store.apply_project(facts, CREATED + 2_000).unwrap();
        }
        let answers = links::read(
            home.path(),
            &[ReadRequest {
                agent: hide_session::Agent::Claude,
                path: home
                    .path()
                    .join(".claude/projects/-mini-app/s-mini.jsonl")
                    .to_string_lossy()
                    .into(),
                checkpoint: None,
            }],
        );
        store
            .apply_answer_observed("mini", &answers[0], "replay", Some(now_ms()))
            .unwrap();
        assert_eq!(store.ledger_record(&identity).unwrap().repository, None);
    }

    #[derive(Default)]
    struct Seen {
        panel: Option<Result<PanelAnswer, String>>,
        summaries: BTreeMap<String, ProjectLinkSummary>,
    }

    struct TestSink {
        seen: Arc<Mutex<Seen>>,
        device: Arc<Device>,
    }

    impl Sink for TestSink {
        fn summaries(&self, summaries: BTreeMap<String, ProjectLinkSummary>) {
            self.seen.lock().unwrap().summaries = summaries;
        }
        fn panel(&self, _generation: u64, answer: Result<PanelAnswer, String>) {
            self.seen.lock().unwrap().panel = Some(answer);
        }
        fn filling(&self, _filling: bool) {}
        fn devices(&self) -> Vec<(String, Arc<dyn NodeLink>)> {
            vec![("mini".to_owned(), self.device.clone() as Arc<dyn NodeLink>)]
        }
    }

    fn iso(ms: u64) -> String {
        jiff::Timestamp::from_millisecond(ms as i64)
            .unwrap()
            .to_string()
    }

    /// The device's session that made PR 7 from its own checkout.
    fn device_session(home: &Path) {
        let dir = home.join(".claude/projects/-mini-app");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            serde_json::json!({
                "type": "user", "isSidechain": false, "uuid": "u1", "parentUuid": null,
                "message": {"role": "user", "content": "mini에서 PR 올려 줘"},
                "timestamp": iso(CREATED - 60_000), "promptId": "p",
                "origin": {"kind": "human"}, "userType": "external", "entrypoint": "cli",
                "cwd": "/mini/app", "sessionId": "s-mini", "gitBranch": "feat",
            }),
            serde_json::json!({
                "type": "pr-link", "sessionId": "s-mini", "prNumber": 7,
                "prRepository": "acme/app", "prUrl": "https://github.com/acme/app/pull/7",
                "timestamp": iso(CREATED + 1_000),
            }),
        ];
        let text = lines.map(|line| line.to_string()).join("\n") + "\n";
        std::fs::write(dir.join("s-mini.jsonl"), text).unwrap();
    }

    fn wait_until(what: &str, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            #[allow(clippy::disallowed_methods)] // a polling helper bounded by a deadline
            thread::sleep(Duration::from_millis(5));
        }
    }

    struct Refusing;

    impl NodeLink for Refusing {
        fn call(&self, _call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
            Err(LinkError::NotConnected("gone".into()))
        }
    }

    /// A device that drops during its listing is not recorded as listed
    /// through, so what it holds is listed again when it is back (B40).
    #[test]
    fn a_failed_device_listing_is_not_recorded_as_listed() {
        let state = tempfile::tempdir().unwrap();
        let (store, _) = LinkStore::open(&state.path().join("links.sqlite3")).unwrap();
        let mut entry = DeviceQueue::default();
        assert!(list_device(&store, "mini", &Refusing, &mut entry).is_err());
        finish_device(&store, "mini", &mut entry);
        assert_eq!(store.meta("listed_at:mini").unwrap(), None);
        assert!(!entry.filled);
        assert!(entry.listing.is_none());
    }

    /// B20, B40: a connected device's session reaches a Mac project's panel
    /// with its device, its rows stay while the device is gone, and what it
    /// adds meanwhile is read when it is back.
    #[test]
    fn a_devices_session_joins_the_panel_and_outlives_the_connection() {
        let state = tempfile::tempdir().unwrap();
        let device_home = tempfile::tempdir().unwrap();
        device_session(device_home.path());
        let own_home = tempfile::tempdir().unwrap();
        let device = Arc::new(Device {
            home: device_home.path().to_path_buf(),
            connected: AtomicBool::new(true),
        });
        let seen = Arc::new(Mutex::new(Seen::default()));
        let worker = LinkWorker::spawn(
            Paths {
                store: state.path().join("links.sqlite3"),
                search: state.path().join("session-search.sqlite3"),
                local_device: "local".into(),
            },
            Arc::new(Device {
                home: own_home.path().to_path_buf(),
                connected: AtomicBool::new(true),
            }),
            TestSink {
                seen: Arc::clone(&seen),
                device: Arc::clone(&device),
            },
        )
        .unwrap();
        let client = worker.client();
        client.projects(Arc::new(vec![ProjectFacts {
            key: "app".into(),
            device_id: "local".into(),
            workspace_id: "w".into(),
            root: "/work/app".into(),
            repository: Some("acme/app".into()),
            repository_id: None,
            connection_epoch: 0,
            worktrees: vec![WorktreeFact {
                path: "/work/app".into(),
                branch: Some("main".into()),
                created_at_unix_ms: None,
            }],
            prs: vec![PrFact {
                repository: "acme/app".into(),
                number: 7,
                branch: "feat".into(),
                title: "PR 7".into(),
                url: "https://github.com/acme/app/pull/7".into(),
                created_at: Some(CREATED),
                closed_at: None,
                merged_at: None,
                issues: Vec::new(),
                hide_issue_known: false,
            }],
            prs_read: true,
            outputs: Vec::new(),
        }]));
        client.read(PanelRequest {
            generation: 1,
            project: "app".into(),
            target: PanelTarget::Pr(7),
        });
        let lines = || match &seen.lock().unwrap().panel {
            Some(Ok(PanelAnswer::Pr(Some(links)))) => links.sessions.clone(),
            _ => Vec::new(),
        };
        wait_until("the device's line", || !lines().is_empty());
        let line = &lines()[0];
        assert_eq!(
            (line.device_id.as_str(), line.role),
            ("mini", SessionRole::Created)
        );
        assert_eq!(line.request.as_deref(), Some("mini에서 PR 올려 줘"));
        // This machine cannot look at a device's file.
        assert_eq!(line.file, FileState::Unknown);
        wait_until("the chip", || {
            seen.lock()
                .unwrap()
                .summaries
                .get("w")
                .is_some_and(|summary| summary.sessions.contains_key("s-mini"))
        });

        // Gone: nothing is read and nothing leaves (B40).
        device.connected.store(false, Ordering::SeqCst);
        client.read(PanelRequest {
            generation: 2,
            project: "app".into(),
            target: PanelTarget::Pr(7),
        });
        wait_until("the reread", || {
            matches!(
                &seen.lock().unwrap().panel,
                Some(Ok(PanelAnswer::Pr(Some(_))))
            )
        });
        assert_eq!(lines().len(), 1);
        drop(worker);
    }
}
