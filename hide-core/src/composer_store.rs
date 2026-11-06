//! The panes the doorbell holds as an unsent draft, kept across a restart.
//!
//! What the operator typed into a composer is on the screen and nowhere else,
//! so the doorbell knows a draft only from the input and submit clocks it
//! keeps in memory (`runtime/delivery.rs`). A daemon that starts over sees
//! every pane fresh, with no draft assumed, and would type a bell over a
//! half-written message. This file keeps the one fact that cannot be read
//! back: this pane held a draft when the last run stopped.
//!
//! A mark is written when a pane's draft begins and removed when it ends (a
//! prompt hook of the pane's own session, or the pane entering work), so the
//! file moves with a prompt, not with a key. A restored mark holds the pane
//! as a draft until the same events end it; it does not silence the pane
//! beyond that, and a pane with no mark starts as before, with no draft
//! assumed and a 30 second grace.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::delivery::Actor;
use crate::latest_file::{LatestFile, read_capped};

const SCHEMA_VERSION: u32 = 1;
/// Panes held as a draft at once. A draft is one prompt's worth of typing,
/// so a file past this holds panes nobody is typing into.
pub(crate) const MARK_LIMIT: usize = 256;
/// How long a mark of a pane that is not observed is kept: the pane may be on
/// a device that is away, or the daemon may be down, but not for ever.
pub(crate) const UNSEEN_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const READ_CAP: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mark {
    pub pane_id: String,
    pub device_id: String,
    pub session: Option<String>,
    /// When the pane was first held as a draft in this record.
    pub since_unix_ms: u64,
}

impl Mark {
    pub(crate) fn of(actor: &Actor, since_unix_ms: u64) -> Self {
        Self {
            pane_id: actor.pane_id.clone(),
            device_id: actor.device_id.clone(),
            session: actor.session.clone(),
            since_unix_ms,
        }
    }

    /// Whether `actor` is the pane and session this mark was written for.
    pub(crate) fn matches(&self, actor: &Actor) -> bool {
        self.pane_id == actor.pane_id
            && self.device_id == actor.device_id
            && self.session == actor.session
    }
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    marks: Vec<Mark>,
}

fn encode(marks: &Arc<[Mark]>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        marks: marks.to_vec(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

fn decode(bytes: &[u8], now: u64) -> Result<Vec<Mark>, String> {
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if stored.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            stored.schema_version
        ));
    }
    if stored.marks.len() > MARK_LIMIT {
        return Err(format!(
            "holds {} marks, past {MARK_LIMIT}",
            stored.marks.len()
        ));
    }
    let valid = |mark: &Mark| {
        crate::delivery::valid_key(&mark.pane_id)
            && crate::delivery::valid_key(&mark.device_id)
            && mark
                .session
                .as_deref()
                .is_none_or(crate::delivery::valid_key)
    };
    if !stored.marks.iter().all(valid) {
        return Err("holds a mark that names no pane".to_owned());
    }
    Ok(stored
        .marks
        .into_iter()
        .filter(|mark| now.saturating_sub(mark.since_unix_ms) < UNSEEN_TTL_MS)
        .collect())
}

pub(crate) struct ComposerStore {
    path: PathBuf,
    file: LatestFile<Arc<[Mark]>>,
}

impl ComposerStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-composer-save",
                "delivery",
                "composer_marks",
                encode,
            ),
            path,
        }
    }

    /// The panes the last run held as a draft. A missing file is the first
    /// run; any other reason a file is not used is stated, then the file is
    /// ignored and the next save replaces it.
    pub(crate) fn restore(&self, now: u64) -> Vec<Mark> {
        let discard = |reason: String| {
            crate::diagnostic!(serde_json::json!({
                "component": "delivery",
                "kind": "composer_marks.discarded",
                "path": self.path.to_string_lossy(),
                "message": reason,
            }));
            Vec::new()
        };
        match read_capped(&self.path, READ_CAP) {
            Ok(None) => Vec::new(),
            Ok(Some(bytes)) => decode(&bytes, now).unwrap_or_else(discard),
            Err(reason) => discard(reason),
        }
    }

    /// Queues `marks` to replace the file; only the newest of a burst is
    /// written.
    pub(crate) fn save(&self, marks: Vec<Mark>) {
        self.file.save(marks.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(pane: &str, since: u64) -> Mark {
        Mark {
            pane_id: pane.into(),
            device_id: "node".into(),
            session: Some(format!("{pane}-session")),
            since_unix_ms: since,
        }
    }

    #[test]
    fn saved_marks_come_back_and_the_newest_save_wins() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-marks.json");
        {
            let store = ComposerStore::new(path.clone());
            store.save(vec![mark("w1:p1", 10)]);
            store.save(vec![mark("w1:p1", 10), mark("w1:p2", 20)]);
        }
        let restored = ComposerStore::new(path).restore(30);
        assert_eq!(restored, vec![mark("w1:p1", 10), mark("w1:p2", 20)]);
    }

    #[test]
    fn a_missing_damaged_or_other_version_file_restores_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-marks.json");
        let store = ComposerStore::new(path.clone());
        assert!(store.restore(1).is_empty());
        for bytes in [
            &b"not json"[..],
            br#"{"schema_version":2,"marks":[]}"#,
            br#"{"schema_version":1,"marks":[{"pane_id":"","device_id":"node","session":null,"since_unix_ms":1}]}"#,
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(store.restore(1).is_empty());
        }
    }

    #[test]
    fn a_mark_older_than_the_ttl_is_not_restored() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("composer-marks.json");
        {
            ComposerStore::new(path.clone())
                .save(vec![mark("w1:p1", 1), mark("w1:p2", UNSEEN_TTL_MS)]);
        }
        let restored = ComposerStore::new(path).restore(UNSEEN_TTL_MS + 1);
        assert_eq!(restored, vec![mark("w1:p2", UNSEEN_TTL_MS)]);
    }
}
