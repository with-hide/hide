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
use crate::missions::NodeStatus;

/// Events one mission keeps; past this the oldest goes (B21).
pub const EVENT_LIMIT: usize = 500;
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
    /// An answer from the banner typed without Herdr confirming it arrived.
    AnswerUnconfirmed,
    Reported,
    Cancelled,
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
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
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

/// What the save thread still has to do for one mission: delete its file,
/// then add these events to its log and write it. A burst of events for a
/// mission is written once.
#[derive(Default)]
struct Work {
    removed: bool,
    added: Vec<Arc<MissionEvent>>,
}

/// The logs read at start, which the thread's own copies begin from, and
/// the work by mission.
#[derive(Default)]
struct Pending {
    restored: HashMap<String, EventLog>,
    work: BTreeMap<String, Work>,
    stop: bool,
}

pub(crate) struct MissionEventStore {
    dir: PathBuf,
    shared: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl MissionEventStore {
    pub(crate) fn new(dir: PathBuf) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = {
            let shared = Arc::clone(&shared);
            let dir = dir.clone();
            std::thread::Builder::new()
                .name("hide-mission-events".to_owned())
                .spawn(move || run(&shared, &dir))
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
            let path = file(&self.dir, id);
            match read_capped(&path, READ_CAP)
                .and_then(|bytes| bytes.map(|b| decode(&b)).transpose())
            {
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
        let held = out
            .iter()
            .map(|(id, log)| (id.clone(), History::from(log.clone())))
            .collect();
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .restored = out;
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

/// Deletes and writes what is pending until the store stops, finishing the
/// work already handed over first. The thread keeps each log it writes, so
/// an event costs the lock only its pointer.
fn run(shared: &(Mutex<Pending>, Condvar), dir: &Path) {
    let (lock, wake) = shared;
    let mut logs: HashMap<String, EventLog> = HashMap::new();
    loop {
        let work = {
            let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
            while pending.work.is_empty() && !pending.stop {
                pending = wake.wait(pending).unwrap_or_else(PoisonError::into_inner);
            }
            for (id, log) in std::mem::take(&mut pending.restored) {
                logs.entry(id).or_insert(log);
            }
            if pending.work.is_empty() {
                return;
            }
            std::mem::take(&mut pending.work)
        };
        for (mission, work) in work {
            let path = file(dir, &mission);
            let mut result = Ok(());
            if work.removed {
                logs.remove(&mission);
                result = match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(format!("could not be deleted: {error}")),
                };
            }
            if result.is_ok() && !work.added.is_empty() {
                let log = logs.entry(mission.clone()).or_default();
                for event in work.added {
                    log.push(Arc::unwrap_or_clone(event));
                }
                result = write(dir, &path, log);
            }
            if let Err(reason) = result {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission_events.save_failed",
                    "mission": mission, "message": reason,
                }));
            }
        }
    }
}

fn write(dir: &Path, path: &Path, log: &EventLog) -> Result<(), String> {
    let bytes = encode(log)?;
    hide_platform::fs::private::create_dir_all(dir)
        .map_err(|error| format!("folder could not be prepared: {error}"))?;
    hide_platform::fs::atomic::write_file(path, &bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| format!("could not be replaced: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at: u64) -> MissionEvent {
        MissionEvent::new(at, EventKind::Node).node("a")
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
}
