//! What happened in a mission, in order (PRD mission-plan-visibility D-15,
//! D-22, B31): its plans, each node's moves, its pull requests, main, the
//! operator's decisions and the report. The 흐름 table reads it; nothing in
//! the record's contracts does.
//!
//! Each mission keeps its own file beside the book, at most `EVENT_LIMIT`
//! events, the oldest dropped and counted past that, so a long mission never
//! makes the book's own save budget refuse a change (B21). The runtime holds
//! the events and decides them under the lock, each event and the list
//! shared, so publishing a history copies a pointer; it hands the save
//! thread only the new events, and the thread keeps its own copy of each
//! log, which it extends, encodes and writes off the lock
//! (docs/PERFORMANCE_TESTING.md). The thread is joined when the store drops.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::latest_file::read_capped;
use crate::missions::{
    AttentionReason, FixReason, HideAction, NodeStatus, OperatorAction, Strength,
};

/// Events one mission keeps; past this the oldest goes (B21).
pub const EVENT_LIMIT: usize = 500;
/// Shelved bodies/history held until durable, separate from loaded work.
pub(crate) const PENDING_SHELVED_LIMIT: usize = 32;
/// The longest line an event keeps.
const LINE_LIMIT: usize = 600;
/// 500 events of a few hundred bytes each, with room to spare.
const READ_CAP: u64 = 2 * 1024 * 1024;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The first plan.
    Planned,
    /// A later plan, with its reason and how many nodes it changed.
    Replanned,
    /// A node moved to `status`.
    Node,
    /// A node is tried again; `count` is its attempt.
    Retried,
    PrOpened,
    PrMerged,
    /// A direct change reached main; `line` is its commit.
    Landed,
    /// The Conductor asked the operator; `line` is the question.
    Asked,
    /// The operator answered; `line` is the answer when Hide carried it.
    Answered,
    /// An answer from the banner that did not reach the Conductor (D-30).
    AnswerUndelivered,
    /// An answer from the banner typed without the engine confirming it arrived.
    AnswerUnconfirmed,
    Reported,
    Cancelled,
    /// The Conductor went past a finish line; `criterion` is its key and
    /// `line` the reason (PRD mission-map-git-criteria B11).
    Skipped,
    /// A later node's defect opened the node again; `line` is the defect,
    /// `criterion` the node that found it and `count` how often (B21).
    Reopened,
    /// A plan changed the verification strength to `strength`; `line` is
    /// why, when the plan said.
    Strength,
    /// Hide itself acted on a node (PRD mission-node-simple D-15): `hide`
    /// says what, with its `pr` and `why`.
    Hide,
    /// Hide closed the mission with every node finished (B18).
    HideClosed,
    /// A node waits on the Conductor to look; `reason` says why.
    Raised,
    /// A node waits on the operator; `reason` says why (B14).
    RaisedToOperator,
    /// The operator settled what waited on a node with `action`; `line`
    /// names the lines it went past (B15, D-15).
    Resolved,
}

/// Where an answer came from (B31).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSource {
    Banner,
    Conversation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionEvent {
    pub at_unix_ms: u64,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AnswerSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<Strength>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide: Option<HideAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<FixReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<AttentionReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<OperatorAction>,
    /// The key of an act Hide's own evaluation made, from the facts it
    /// stands on: the mission, the node and its work round, what was done
    /// and the pull request or head it was about (B27). The same facts
    /// evaluated again make the same key, and a history holding it takes
    /// the event once. None for an event of a command or a person's choice,
    /// which happens once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl MissionEvent {
    pub(crate) fn new(at_unix_ms: u64, kind: EventKind) -> Self {
        Self {
            at_unix_ms,
            kind,
            node: None,
            status: None,
            line: None,
            options: Vec::new(),
            count: None,
            pr: None,
            source: None,
            criterion: None,
            strength: None,
            hide: None,
            why: None,
            reason: None,
            action: None,
            id: None,
        }
    }

    pub(crate) fn node(mut self, key: &str) -> Self {
        self.node = Some(key.to_owned());
        self
    }

    /// The event's one line, cut to `LINE_LIMIT` characters, so a long
    /// report or question never makes the file too large to read back.
    pub(crate) fn line(mut self, line: Option<&str>) -> Self {
        self.line = line.and_then(|line| crate::display_text::one_line(line, LINE_LIMIT));
        self
    }
}

/// One mission's events as the core holds and publishes them: the list and
/// each event shared, so the 보기 slot's copy is a pointer and a change
/// that finds the list shared copies pointers, never the events.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct History {
    pub events: Arc<Vec<Arc<MissionEvent>>>,
    pub dropped: u64,
}

impl History {
    /// Whether an event with key `id` is in the history kept.
    pub(crate) fn holds(&self, id: &str) -> bool {
        self.events
            .iter()
            .any(|event| event.id.as_deref() == Some(id))
    }

    /// Adds `event`, dropping the oldest past `EVENT_LIMIT` as `EventLog`
    /// does, so the save thread's copy stays the same.
    pub(crate) fn push(&mut self, event: Arc<MissionEvent>) {
        let events = Arc::make_mut(&mut self.events);
        events.push(event);
        if events.len() > EVENT_LIMIT {
            let over = events.len() - EVENT_LIMIT;
            events.drain(..over);
            self.dropped = self.dropped.saturating_add(over as u64);
        }
    }
}

impl From<EventLog> for History {
    fn from(log: EventLog) -> Self {
        Self {
            events: Arc::new(log.events.into_iter().map(Arc::new).collect()),
            dropped: log.dropped,
        }
    }
}

/// One mission's events, oldest first, and how many were dropped before
/// the first of them: the file's shape, and the save thread's copy.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventLog {
    #[serde(default)]
    pub events: Vec<MissionEvent>,
    #[serde(default)]
    pub dropped: u64,
}

impl EventLog {
    fn holds(&self, id: &str) -> bool {
        self.events
            .iter()
            .any(|event| event.id.as_deref() == Some(id))
    }

    /// Adds `event`, dropping the oldest past `EVENT_LIMIT`. Returns
    /// whether one was dropped, which the caller logs.
    pub(crate) fn push(&mut self, event: MissionEvent) -> bool {
        self.events.push(event);
        if self.events.len() <= EVENT_LIMIT {
            return false;
        }
        let over = self.events.len() - EVENT_LIMIT;
        self.events.drain(..over);
        self.dropped = self.dropped.saturating_add(over as u64);
        true
    }
}

#[derive(Deserialize)]
struct Stored {
    schema_version: u32,
    #[serde(flatten)]
    log: EventLog,
}

#[derive(Serialize)]
struct StoredRef<'a> {
    schema_version: u32,
    #[serde(flatten)]
    log: &'a EventLog,
}

fn encode(log: &EventLog) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&StoredRef {
        schema_version: SCHEMA_VERSION,
        log,
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

fn decode(bytes: &[u8]) -> Result<EventLog, String> {
    let decoded = |error: serde_json::Error| format!("could not be decoded: {error}");
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(decoded)?;
    simple_statuses(&mut value);
    let stored: Stored = serde_json::from_value(value).map_err(decoded)?;
    if stored.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            stored.schema_version
        ));
    }
    if stored.log.events.len() > EVENT_LIMIT {
        return Err("holds more events than the limit allows".to_owned());
    }
    Ok(stored.log)
}

/// A node's move written before nodes took three states (PRD
/// mission-node-simple D-16) reads as the state it is now: in review, held
/// or failed is in progress, waiting is not started. The history keeps the
/// line it was written with, and a failure keeps its meaning as the
/// Conductor's recorded failure.
fn simple_statuses(value: &mut serde_json::Value) {
    let Some(events) = value
        .get_mut("events")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for event in events {
        let Some(status) = event.get_mut("status") else {
            continue;
        };
        let failed = status.as_str() == Some("failed");
        let next = match status.as_str() {
            Some("review" | "held" | "failed") => "working",
            Some("waiting") => "planned",
            _ => continue,
        };
        *status = next.into();
        if failed && let Some(event) = event.as_object_mut() {
            event
                .entry("reason")
                .or_insert_with(|| AttentionReason::ConductorFailed.as_str().into());
        }
    }
}

/// What the save thread still has to do for one mission: delete its file,
/// then add these events to its log and write it. A burst of events for a
/// mission is written once.
#[derive(Default)]
struct Work {
    removed: bool,
    /// A legacy log must be copied before the migration's index is saved.
    rewrite: bool,
    /// Its mission was shelved: the thread lets go of its copy of the log
    /// and leaves the file.
    released: bool,
    added: Vec<Arc<MissionEvent>>,
}

/// The logs read at start, which the thread's own copies begin from, the
/// work by mission, and the newest book to write after them.
#[derive(Default)]
struct Pending {
    restored: HashMap<String, EventLog>,
    work: BTreeMap<String, Work>,
    book: Option<Arc<crate::missions::MissionBook>>,
    /// The missions shelved since the last book written, whose files take
    /// their last state before the book's memory lets go of them.
    leaving: Vec<crate::missions::Mission>,
    /// Includes bodies owned by both this queue and the writer, until the
    /// matching book and histories have all been saved.
    unsaved_shelved: std::collections::BTreeSet<String>,
    barrier: Option<std::sync::mpsc::SyncSender<Result<(), String>>>,
    stop: bool,
}

/// An explicit read waits off the runtime lock for the save it follows.
/// Without this, show/reopen just after shelving could read the old body.
pub(crate) struct ReadBarrier(Arc<(Mutex<Pending>, Condvar)>);

impl ReadBarrier {
    pub(crate) fn wait(self) -> Result<(), String> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        {
            let mut pending = self.0.0.lock().unwrap_or_else(PoisonError::into_inner);
            // Core commands run on one owner thread: at most one explicit
            // shelved read can wait. A timed-out read must not grow a queue.
            pending.barrier = Some(sender);
        }
        self.0.1.notify_one();
        receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "mission save did not answer".to_owned())?
    }
}

/// The one writer of the missions' files: each mission's history, and,
/// when it is given the mission folder (`mission_store::MissionFiles`), the
/// missions after them (B27). A history
/// is written before the book that a change of it went with, and the book
/// waits while a history write fails, so a stop between the two leaves a
/// book that does not yet know the act, which its next evaluation makes
/// again under the same key, never an act the history lost.
pub(crate) struct MissionEventStore {
    dir: PathBuf,
    shared: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl MissionEventStore {
    /// A writer of the histories in `dir` only.
    #[cfg(test)]
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self::start(dir, None)
    }

    /// The writer of the histories in `dir` and of the missions' `files`.
    pub(crate) fn with_book(dir: PathBuf, files: crate::mission_store::MissionFiles) -> Self {
        Self::start(dir, Some(files))
    }

    fn start(dir: PathBuf, book: Option<crate::mission_store::MissionFiles>) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = {
            let shared = Arc::clone(&shared);
            let dir = dir.clone();
            std::thread::Builder::new()
                .name("hide-mission-events".to_owned())
                .spawn(move || run(&shared, &dir, book))
                .inspect_err(|error| {
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "mission_events.worker_failed",
                        "message": error.to_string(),
                    }));
                })
                .ok()
        };
        Self {
            dir,
            shared,
            worker,
        }
    }

    /// Reads the events of `missions` from their files, before the lock is
    /// taken, and gives the save thread the same logs to extend. A file
    /// that cannot be read is logged and its mission starts with none; it
    /// is replaced on that mission's next event.
    pub(crate) fn restore<'a>(
        &self,
        missions: impl Iterator<Item = &'a str>,
    ) -> HashMap<String, History> {
        let mut out = HashMap::new();
        for id in missions {
            match read_log(&self.dir, id) {
                Ok(Some(log)) => {
                    out.insert(id.to_owned(), log);
                }
                Ok(None) => {}
                Err(reason) => crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission_events.unreadable",
                    "mission": id, "message": reason,
                })),
            }
        }
        self.seed(out, false)
    }

    /// The first migration reads the legacy histories once. Every log is
    /// written into the folder book before its index, and a failure keeps
    /// that book pending. The legacy files are never changed or swept.
    pub(crate) fn restore_legacy<'a>(
        &self,
        legacy: &Path,
        missions: impl Iterator<Item = &'a str>,
    ) -> Result<HashMap<String, History>, String> {
        let mut logs = HashMap::new();
        for id in missions {
            if let Some(log) = read_log(legacy, id)
                .map_err(|reason| format!("history of {id} could not be migrated: {reason}"))?
            {
                logs.insert(id.to_owned(), log);
            }
        }
        Ok(self.seed(logs, true))
    }

    fn seed(&self, out: HashMap<String, EventLog>, copy: bool) -> HashMap<String, History> {
        let held = out
            .iter()
            .map(|(id, log)| (id.clone(), History::from(log.clone())))
            .collect();
        let mut pending = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        if copy {
            for id in out.keys() {
                pending.work.entry(id.clone()).or_default().rewrite = true;
            }
        }
        pending.restored = out;
        held
    }

    /// Deletes the files of missions the book no longer holds (D-22): a
    /// delete that failed, or a stop between the book's save and the
    /// delete, is finished at the next start. Called only with a book that
    /// was read, so an unreadable one deletes nothing.
    pub(crate) fn sweep<'a>(&self, missions: impl Iterator<Item = &'a str>) {
        let kept: std::collections::HashSet<&str> = missions.collect();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission_events.sweep_failed",
                    "message": error.to_string(),
                }));
                return;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
                continue;
            };
            if !kept.contains(id) {
                self.remove(id);
            }
        }
    }

    /// Hands the save thread a mission's new events; only they cross the
    /// lock, as shared pointers.
    pub(crate) fn append(&self, mission: &str, events: &[Arc<MissionEvent>]) {
        self.post(mission, |work| work.added.extend(events.iter().cloned()));
    }

    /// Hands the save thread the book to write once the histories handed
    /// over before it are written, with the missions it shelved since
    /// (`leaving`); a newer book replaces one still waiting, and the
    /// missions leaving are kept until written.
    pub(crate) fn save_book(
        &self,
        book: Arc<crate::missions::MissionBook>,
        leaving: Vec<crate::missions::Mission>,
    ) {
        let (lock, wake) = &*self.shared;
        let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
        pending.book = Some(book);
        pending
            .unsaved_shelved
            .extend(leaving.iter().map(|mission| mission.id.clone()));
        pending.leaving.extend(leaving);
        drop(pending);
        wake.notify_one();
    }

    pub(crate) fn unsaved_shelved(&self) -> std::collections::BTreeSet<String> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unsaved_shelved
            .clone()
    }

    /// Lets the save thread drop its copy of a shelved mission's log; the
    /// file stays.
    pub(crate) fn release(&self, mission: &str) {
        self.post(mission, |work| work.released = true);
    }

    pub(crate) fn read_barrier(&self) -> ReadBarrier {
        ReadBarrier(Arc::clone(&self.shared))
    }

    /// Gives the save thread the log of a mission read back from its shelf
    /// (`mission_store::read_shelved`), which its next events extend.
    pub(crate) fn adopt(&self, mission: &str, log: EventLog) {
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .restored
            .insert(mission.to_owned(), log);
    }

    /// Deletes the mission's file with the mission (D-22).
    pub(crate) fn remove(&self, mission: &str) {
        self.post(mission, |work| {
            work.removed = true;
            work.added.clear();
        });
    }

    fn post(&self, mission: &str, change: impl FnOnce(&mut Work)) {
        let (lock, wake) = &*self.shared;
        change(
            lock.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .work
                .entry(mission.to_owned())
                .or_default(),
        );
        wake.notify_one();
    }
}

impl Drop for MissionEventStore {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap_or_else(PoisonError::into_inner).stop = true;
        wake.notify_one();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission_events.join_failed",
            }));
        }
    }
}

/// The file of `mission`'s events. A mission id is a key (`valid_key`), so
/// it names one file in the folder and nothing outside it.
fn file(dir: &Path, mission: &str) -> PathBuf {
    dir.join(format!("{mission}.json"))
}

/// `mission`'s history as its file in `dir` holds it; `None` with no file.
pub(crate) fn read_log(dir: &Path, mission: &str) -> Result<Option<EventLog>, String> {
    read_capped(&file(dir, mission), READ_CAP)?
        .map(|bytes| decode(&bytes))
        .transpose()
}

impl MissionEventStore {
    /// The folder of the histories, which a shelved mission's is read from.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

/// How long a history or book whose write failed waits before it is
/// written again, while nothing new arrives for it.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// Deletes and writes what is pending until the store stops, finishing the
/// work already handed over first, then the book (`MissionEventStore`).
/// The thread keeps each log it writes, so an event costs the lock only
/// its pointer. A write that failed keeps its log, or the newest book, in
/// memory and is written again after `RETRY_AFTER`, with the next change,
/// and once more when the store stops, so a disk that recovers loses
/// nothing; the book is not written while a history is failing. Each
/// failure is logged once until a write succeeds. What it holds is one log
/// per loaded mission and one latest book; shelved bodies awaiting a save
/// remain owned here until the disk takes them.
fn run(
    shared: &(Mutex<Pending>, Condvar),
    dir: &Path,
    mut files: Option<crate::mission_store::MissionFiles>,
) {
    let (lock, wake) = shared;
    let mut logs: HashMap<String, EventLog> = HashMap::new();
    let mut failing: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut book: Option<Arc<crate::missions::MissionBook>> = None;
    let mut leaving: Vec<crate::missions::Mission> = Vec::new();
    let mut book_failing = false;
    let mut book_held = false;
    let mut releasing = std::collections::BTreeSet::new();
    let mut shelving = std::collections::BTreeSet::new();
    loop {
        let (mut work, barrier, stop) = {
            let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
            while pending.work.is_empty()
                && pending.book.is_none()
                && pending.barrier.is_none()
                && !pending.stop
            {
                if failing.is_empty() && book.is_none() {
                    pending = wake.wait(pending).unwrap_or_else(PoisonError::into_inner);
                } else {
                    let (next, waited) = wake
                        .wait_timeout(pending, RETRY_AFTER)
                        .unwrap_or_else(PoisonError::into_inner);
                    pending = next;
                    if waited.timed_out() {
                        break;
                    }
                }
            }
            for (id, log) in std::mem::take(&mut pending.restored) {
                releasing.remove(&id);
                logs.entry(id).or_insert(log);
            }
            if let Some(newer) = pending.book.take() {
                book = Some(newer);
            }
            shelving.extend(pending.leaving.iter().map(|mission| mission.id.clone()));
            leaving.append(&mut pending.leaving);
            (
                std::mem::take(&mut pending.work),
                pending.barrier.take(),
                pending.stop,
            )
        };
        // A log whose write failed is written again with this pass.
        for mission in &failing {
            work.entry(mission.clone()).or_default();
        }
        for (mission, work) in work {
            let path = file(dir, &mission);
            let mut result = Ok(());
            if work.removed {
                releasing.remove(&mission);
                logs.remove(&mission);
                result = match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(format!("could not be deleted: {error}")),
                };
            } else {
                if work.released {
                    releasing.insert(mission.clone());
                }
                let log = logs.entry(mission.clone()).or_default();
                let mut added = false;
                for event in work.added {
                    // An act evaluated again on the same facts is in the log
                    // already (`MissionEvent::id`).
                    if event.id.as_deref().is_some_and(|id| log.holds(id)) {
                        continue;
                    }
                    log.push(Arc::unwrap_or_clone(event));
                    added = true;
                }
                if added || work.rewrite || failing.contains(&mission) {
                    result = write(dir, &path, log);
                }
            }
            match result {
                Ok(()) => {
                    failing.remove(&mission);
                    if releasing.remove(&mission) {
                        logs.remove(&mission);
                    }
                }
                Err(reason) => {
                    if failing.insert(mission.clone()) {
                        crate::diagnostic!(json!({
                            "component": "missions", "kind": "mission_events.save_failed",
                            "mission": mission, "message": reason,
                        }));
                    }
                    // A delete that failed is finished by the next start's
                    // sweep, not here.
                    if work.removed {
                        failing.remove(&mission);
                    }
                }
            }
        }
        let mut saved = false;
        match (&book, files.as_mut()) {
            (Some(_), None) => {
                book = None;
                leaving.clear();
                saved = true;
            }
            (Some(_), Some(_)) if !failing.is_empty() => {
                if !book_held {
                    book_held = true;
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "missions.save_held",
                        "failing": failing.len(),
                    }));
                }
            }
            (Some(waiting), Some(files)) => {
                book_held = false;
                match files.write(waiting, &mut leaving) {
                    Ok(()) => {
                        book = None;
                        book_failing = false;
                        saved = true;
                    }
                    Err(reason) => {
                        if !book_failing {
                            book_failing = true;
                            crate::diagnostic!(json!({
                                "component": "missions", "kind": "missions.save_failed",
                                "message": reason,
                            }));
                        }
                    }
                }
            }
            (None, _) => {}
        }
        if saved {
            let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
            for id in std::mem::take(&mut shelving) {
                if !pending.leaving.iter().any(|mission| mission.id == id) {
                    pending.unsaved_shelved.remove(&id);
                }
            }
        }
        if let Some(barrier) = barrier {
            let outcome = if failing.is_empty() && book.is_none() {
                Ok(())
            } else {
                Err("mission save failed; its files are kept for retry".into())
            };
            let _ = barrier.send(outcome);
        }
        if stop {
            return;
        }
    }
}

fn write(dir: &Path, path: &Path, log: &EventLog) -> Result<(), String> {
    replace(dir, path, &encode(log)?)
}

/// Replaces the file at `path` in `folder`, making the folder first: the
/// one place the writer puts a history on disk.
fn replace(folder: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
    hide_platform::fs::private::create_dir_all(folder)
        .map_err(|error| format!("folder could not be prepared: {error}"))?;
    hide_platform::fs::atomic::write_file(path, bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| format!("could not be replaced: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at: u64) -> MissionEvent {
        MissionEvent::new(at, EventKind::Node).node("a")
    }

    /// The two books own different histories, even when a downgraded
    /// build reuses an id. Exercise both stores' actual load/sweep paths.
    #[test]
    fn a_downgrade_sweep_and_reused_id_leave_the_folder_book_and_its_history_alone() {
        use crate::mission_store::{MissionFiles, MissionStore, Restored};
        use crate::missions::{MissionBook, SCHEMA_VERSION};
        let root = tempfile::tempdir().unwrap();
        let legacy = hide_kit::layout::mission_events(root.path());
        let histories = hide_kit::layout::mission_histories(root.path());
        let dir = hide_kit::layout::missions_dir(root.path());
        let mut book = MissionBook::default();
        book.next = 1;
        book.missions.push(crate::missions::tests::mission());
        let save_old = |book: &MissionBook| {
            std::fs::write(
                hide_kit::layout::missions(root.path()),
                serde_json::to_vec(&json!({
                    "schema_version":SCHEMA_VERSION,"next":book.next,"missions":book.missions
                }))
                .unwrap(),
            )
            .unwrap();
        };
        save_old(&book);
        {
            let old = MissionEventStore::new(legacy.clone());
            old.append("mission-1", &[Arc::new(event(1))]);
        }
        let original = std::fs::read(file(&legacy, "mission-1")).unwrap();
        {
            let Restored::Moving(moving) = MissionStore::new(root.path()).restore() else {
                panic!("first migration")
            };
            let store = MissionEventStore::with_book(
                histories.clone(),
                MissionFiles::new(dir.clone(), None),
            );
            store
                .restore_legacy(&legacy, moving.missions.iter().map(|m| m.id.as_str()))
                .unwrap();
            store.save_book(Arc::new(moving), Vec::new());
            store.read_barrier().wait().unwrap();
            let mut added = book.missions[0].clone();
            added.id = "mission-2".into();
            added.title = "Made by the folder build".into();
            book.next = 2;
            book.missions.push(added);
            store.append("mission-2", &[Arc::new(event(2))]);
            store.save_book(Arc::new(book.clone()), Vec::new());
            store.read_barrier().wait().unwrap();
        }
        assert_eq!(std::fs::read(file(&legacy, "mission-1")).unwrap(), original);
        // The older build still has next=1 and allocates mission-2 again,
        // then mission-3. Its sweep must not reach the folder histories.
        let mut downgraded = crate::missions::decode(
            &std::fs::read(hide_kit::layout::missions(root.path())).unwrap(),
        )
        .unwrap()
        .book;
        assert_eq!(downgraded.next, 1);
        let mut collision = downgraded.missions[0].clone();
        collision.id = downgraded.allocate();
        assert_eq!(collision.id, "mission-2");
        collision.title = "Made by the downgraded build".into();
        let mut old_only = collision.clone();
        old_only.id = downgraded.allocate();
        downgraded.missions.extend([collision, old_only]);
        save_old(&downgraded);
        {
            let old = MissionEventStore::new(legacy.clone());
            old.sweep(downgraded.missions.iter().map(|m| m.id.as_str()));
            old.restore(downgraded.missions.iter().map(|m| m.id.as_str()));
            old.append("mission-2", &[Arc::new(event(20))]);
            old.append("mission-3", &[Arc::new(event(30))]);
        }
        let store = MissionStore::new(root.path());
        let Restored::Read(restored) = store.restore() else {
            panic!("folder remains authoritative")
        };
        store.sweep(&restored);
        let events = MissionEventStore::with_book(
            histories.clone(),
            MissionFiles::new(dir, Some(&restored)),
        );
        events.sweep(restored.missions.iter().map(|m| m.id.as_str()));
        let logs = events.restore(restored.missions.iter().map(|m| m.id.as_str()));
        assert_eq!(
            restored.get("mission-2").unwrap().title,
            "Made by the folder build"
        );
        assert!(restored.get("mission-3").is_none());
        assert_eq!(logs["mission-2"].events[0].at_unix_ms, 2);
        assert_eq!(
            read_log(&legacy, "mission-2").unwrap().unwrap().events[0].at_unix_ms,
            20
        );
        assert_eq!(
            read_log(&legacy, "mission-3").unwrap().unwrap().events[0].at_unix_ms,
            30
        );
    }

    #[test]
    fn a_failed_history_copy_keeps_no_index_and_restarts_from_the_legacy_log() {
        use crate::mission_store::{MissionFiles, MissionStore, Restored};
        let root = tempfile::tempdir().unwrap();
        let legacy = hide_kit::layout::mission_events(root.path());
        {
            let old = MissionEventStore::new(legacy.clone());
            old.append("mission-1", &[Arc::new(event(1))]);
        }
        let book = crate::missions::MissionBook {
            next: 1,
            missions: vec![crate::missions::tests::mission()],
            ..Default::default()
        };
        std::fs::write(
            hide_kit::layout::missions(root.path()),
            serde_json::to_vec(&json!({"schema_version":4,"next":1,"missions":book.missions}))
                .unwrap(),
        )
        .unwrap();
        let histories = hide_kit::layout::mission_histories(root.path());
        std::fs::create_dir_all(&histories).unwrap();
        let blocker = file(&histories, "mission-1");
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("block"), b"").unwrap();
        let dir = hide_kit::layout::missions_dir(root.path());
        {
            let events = MissionEventStore::with_book(
                histories.clone(),
                MissionFiles::new(dir.clone(), None),
            );
            events
                .restore_legacy(&legacy, ["mission-1"].into_iter())
                .unwrap();
            events.save_book(Arc::new(book.clone()), Vec::new());
            assert!(events.read_barrier().wait().is_err());
        }
        assert!(!hide_kit::layout::mission_index(&dir).exists());
        assert!(matches!(
            MissionStore::new(root.path()).restore(),
            Restored::Moving(_)
        ));
        std::fs::remove_dir_all(blocker).unwrap();
        let events =
            MissionEventStore::with_book(histories.clone(), MissionFiles::new(dir.clone(), None));
        events
            .restore_legacy(&legacy, ["mission-1"].into_iter())
            .unwrap();
        events.save_book(Arc::new(book), Vec::new());
        events.read_barrier().wait().unwrap();
        assert!(hide_kit::layout::mission_index(&dir).is_file());
        assert_eq!(
            read_log(&histories, "mission-1").unwrap(),
            read_log(&legacy, "mission-1").unwrap()
        );
    }

    #[test]
    fn a_log_past_its_limit_drops_the_oldest_and_counts_them() {
        let mut log = EventLog::default();
        for at in 0..EVENT_LIMIT as u64 {
            assert!(!log.push(event(at)));
        }
        assert!(log.push(event(EVENT_LIMIT as u64)));
        assert!(log.push(event(EVENT_LIMIT as u64 + 1)));
        assert_eq!(log.events.len(), EVENT_LIMIT);
        assert_eq!(log.dropped, 2);
        assert_eq!(log.events[0].at_unix_ms, 2);
    }

    #[test]
    fn a_saved_log_reads_back_and_a_removed_one_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mission-events");
        let added = [Arc::new(event(1))];
        {
            let store = MissionEventStore::new(folder.clone());
            store.append("mission-1", &added);
            store.append("mission-2", &added);
            store.remove("mission-2");
        }
        let store = MissionEventStore::new(folder.clone());
        let restored = store.restore(["mission-1", "mission-2"].into_iter());
        assert_eq!(restored["mission-1"].events.as_slice(), &added);
        assert!(!restored.contains_key("mission-2"));
        assert!(!folder.join("mission-2.json").exists());
        // Events added after a start extend the log read back, past the limit
        // as the core's own copy drops them.
        let more: Vec<_> = (2..=EVENT_LIMIT as u64 + 1)
            .map(|at| Arc::new(event(at)))
            .collect();
        store.append("mission-1", &more);
        drop(store);
        let restored = MissionEventStore::new(folder).restore(["mission-1"].into_iter());
        let log = &restored["mission-1"];
        assert_eq!((log.events.len(), log.dropped), (EVENT_LIMIT, 1));
        assert_eq!(log.events[0].at_unix_ms, 2);
    }

    #[test]
    fn a_start_deletes_the_files_of_missions_the_book_no_longer_holds() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["mission-1", "mission-2"] {
            std::fs::write(
                dir.path().join(format!("{id}.json")),
                r#"{"schema_version":1,"events":[]}"#,
            )
            .unwrap();
        }
        {
            let store = MissionEventStore::new(dir.path().to_path_buf());
            store.sweep(["mission-1"].into_iter());
        }
        assert!(dir.path().join("mission-1.json").exists());
        assert!(!dir.path().join("mission-2.json").exists());
    }

    #[test]
    fn a_file_of_another_version_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("mission-1.json"),
            r#"{"schema_version":9,"events":[]}"#,
        )
        .unwrap();
        let store = MissionEventStore::new(dir.path().to_path_buf());
        assert!(store.restore(["mission-1"].into_iter()).is_empty());
    }

    /// B27, review 4 P2-1: the book is written after the histories handed
    /// over before it, never while one fails, and a failed write is tried
    /// again when the store stops.
    #[test]
    fn the_book_waits_for_a_failing_history_and_both_are_written_once_the_disk_takes_them() {
        let dir = tempfile::tempdir().unwrap();
        let history = dir.path().join("mission-events");
        let folder = dir.path().join("missions");
        let files = || crate::mission_store::MissionFiles::new(folder.clone(), None);
        let index = hide_kit::layout::mission_index(&folder);
        let book = Arc::new(crate::missions::MissionBook::default());
        std::fs::write(&history, "a file").unwrap();
        let store = MissionEventStore::with_book(history.clone(), files());
        store.append("mission-1", &[Arc::new(event(1))]);
        store.save_book(Arc::clone(&book), Vec::new());
        drop(store);
        assert!(!index.exists(), "the book waits for its history");
        std::fs::remove_file(&history).unwrap();
        let store = MissionEventStore::with_book(history.clone(), files());
        store.append("mission-1", &[Arc::new(event(1))]);
        store.save_book(book, Vec::new());
        drop(store);
        assert!(index.exists());
        let restored = MissionEventStore::new(history).restore(["mission-1"].into_iter());
        assert_eq!(restored["mission-1"].events.len(), 1);
    }

    /// B27: an act written again under its key is in the file once, also
    /// after a start, and a log whose write failed is written with the next
    /// event once the disk takes it, nothing lost.
    #[test]
    fn a_keyed_event_is_written_once_and_a_failed_write_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mission-events");
        let keyed = |at: u64, key: &str| {
            let mut event = event(at);
            event.id = Some(key.to_owned());
            Arc::new(event)
        };
        {
            let store = MissionEventStore::new(folder.clone());
            store.append("mission-1", &[keyed(1, "a@0:done#0")]);
            store.append(
                "mission-1",
                &[keyed(2, "a@0:done#0"), keyed(3, "a@0:raise:x")],
            );
        }
        {
            let store = MissionEventStore::new(folder.clone());
            store.restore(["mission-1"].into_iter());
            store.append("mission-1", &[keyed(4, "a@0:done#0")]);
        }
        let restored = MissionEventStore::new(folder.clone()).restore(["mission-1"].into_iter());
        let keys: Vec<_> = restored["mission-1"]
            .events
            .iter()
            .map(|event| event.id.clone().unwrap())
            .collect();
        assert_eq!(keys, ["a@0:done#0", "a@0:raise:x"]);

        // A file where the folder should be fails the write; once it is
        // gone, the next event writes both.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let store = MissionEventStore::new(blocked.clone());
        store.append("mission-1", &[keyed(5, "b@0:done#0")]);
        std::fs::remove_file(&blocked).unwrap();
        store.append("mission-1", &[keyed(6, "b@0:raise:y")]);
        drop(store);
        let restored = MissionEventStore::new(blocked).restore(["mission-1"].into_iter());
        assert_eq!(restored["mission-1"].events.len(), 2);
    }
    /// Mission-55: closing and shelving can share one batch. The close is
    /// saved before the writer releases its log, including a failed write.
    #[test]
    fn a_close_and_shelve_in_one_batch_keep_the_last_event_after_a_failed_save() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("mission-events");
        std::fs::write(&dir, b"blocks the folder").unwrap();
        let store = MissionEventStore::new(dir.clone());
        {
            let mut pending = store.shared.0.lock().unwrap();
            pending.work.insert(
                "mission-1".into(),
                Work {
                    released: true,
                    added: vec![Arc::new(MissionEvent::new(7, EventKind::Reported))],
                    ..Default::default()
                },
            );
        }
        assert!(store.read_barrier().wait().is_err());
        std::fs::remove_file(&dir).unwrap();
        store.read_barrier().wait().unwrap();
        drop(store);
        let log = read_log(&dir, "mission-1").unwrap().unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.events[0].kind, EventKind::Reported);
    }
}
