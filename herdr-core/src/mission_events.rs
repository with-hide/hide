//! What happened in a mission, in order (PRD mission-plan-visibility D-15,
//! D-22, B31): its plans, each node's moves, its pull requests, main, the
//! operator's decisions and the report. The 흐름 table reads it; nothing in
//! the record's contracts does.
//!
//! Each mission keeps its own file beside the book, at most `EVENT_LIMIT`
//! events, the oldest dropped and counted past that, so a long mission never
//! makes the book's own save budget refuse a change (B21). The runtime holds
//! the events and decides them under the lock; one save thread writes and
//! deletes the files off it, and is joined when the store drops.

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

    pub(crate) fn line(mut self, line: Option<&str>) -> Self {
        self.line = line.map(str::to_owned);
        self
    }
}

/// One mission's events, oldest first, and how many were dropped before
/// the first of them.
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

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    #[serde(flatten)]
    log: EventLog,
}

fn encode(log: &EventLog) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        log: log.clone(),
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

/// What the save thread still has to do, by mission: write that log, or
/// delete the file (`None`). A newer request for a mission replaces the
/// older one, so a burst writes once.
#[derive(Default)]
struct Pending {
    work: BTreeMap<String, Option<Arc<EventLog>>>,
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
    /// taken. A file that cannot be read is logged and its mission starts
    /// with none; it is replaced on that mission's next event.
    pub(crate) fn restore<'a>(
        &self,
        missions: impl Iterator<Item = &'a str>,
    ) -> HashMap<String, EventLog> {
        let mut out = HashMap::new();
        for id in missions {
            let path = file(&self.dir, id);
            match read_capped(&path, READ_CAP).and_then(|bytes| bytes.map(|b| decode(&b)).transpose()) {
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
        out
    }

    pub(crate) fn save(&self, mission: &str, log: Arc<EventLog>) {
        self.post(mission, Some(log));
    }

    /// Deletes the mission's file with the mission (D-22).
    pub(crate) fn remove(&self, mission: &str) {
        self.post(mission, None);
    }

    fn post(&self, mission: &str, work: Option<Arc<EventLog>>) {
        let (lock, wake) = &*self.shared;
        lock.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .work
            .insert(mission.to_owned(), work);
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

/// Writes and deletes what is pending until the store stops, finishing the
/// work already handed over first.
fn run(shared: &(Mutex<Pending>, Condvar), dir: &Path) {
    let (lock, wake) = shared;
    loop {
        let work = {
            let mut pending = lock.lock().unwrap_or_else(PoisonError::into_inner);
            while pending.work.is_empty() && !pending.stop {
                pending = wake.wait(pending).unwrap_or_else(PoisonError::into_inner);
            }
            if pending.work.is_empty() {
                return;
            }
            std::mem::take(&mut pending.work)
        };
        for (mission, log) in work {
            let path = file(dir, &mission);
            let result = match log {
                Some(log) => write(dir, &path, &log),
                None => match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(format!("could not be deleted: {error}")),
                },
            };
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
        let mut log = EventLog::default();
        log.push(event(1));
        log.dropped = 3;
        {
            let store = MissionEventStore::new(folder.clone());
            store.save("mission-1", Arc::new(log.clone()));
            store.save("mission-2", Arc::new(log.clone()));
            store.remove("mission-2");
        }
        let store = MissionEventStore::new(folder.clone());
        let restored = store.restore(["mission-1", "mission-2"].into_iter());
        assert_eq!(restored.get("mission-1"), Some(&log));
        assert!(!restored.contains_key("mission-2"));
        assert!(!folder.join("mission-2.json").exists());
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
