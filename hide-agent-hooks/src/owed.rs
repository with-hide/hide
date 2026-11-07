//! Letter confirmations a hook owes (`docs/delivery.md`, A confirmation
//! that missed its hook).
//!
//! A hook that printed letters to its agent and could not confirm them
//! before its own deadline keeps their ids here by pane, on the machine the
//! pane runs on. The hook's deadline is the agent's, so no later hook is
//! a better place to send them: the node of that machine hands the record
//! over when the core asks for it ([`take`]), which the core does once the
//! pane rests, and the core confirms the letters then. Without it the
//! letters, handed and never belled again, stayed pending until another
//! hook happened to confirm them or their deadline passed.
//!
//! One record per pane holds one bundle and the session it reached; the
//! core takes only the ids of letters to that session. A record older than
//! a letter's deadline, counted from its first miss, is dropped, since the
//! core ended its letters then. At most [`OWED_PANES`] panes owe at once:
//! past it a new confirmation is not kept and the miss is reported, and its
//! letters end at their deadline as before. The same code runs on every
//! system and in a device's hooks, under the hooks' own records folder.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::delivery::{BUNDLE_LETTERS, DELIVERY_EXPIRY_MS, valid_letter_ids};

/// The record's version; one of another version is not read.
const RECORD_VERSION: u32 = 1;
/// The largest record read: one bundle's ids and a session.
const RECORD_LIMIT: u64 = 64 * 1024;
/// The most panes whose confirmations are owed at once.
pub const OWED_PANES: usize = 64;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    session: String,
    ids: Vec<String>,
    owed_at_unix_ms: u64,
}

/// What a pane owes: the letters its hooks handed to the agent of
/// `session` and could not confirm, as the node hands them to the core.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Owed {
    pub session: String,
    pub ids: Vec<String>,
}

/// The folder owed confirmations are kept in, under the hooks' records of
/// the pane this process runs in (`hide_kit::layout::owed_confirmations`
/// names it).
pub fn folder(home: &Path) -> PathBuf {
    crate::route::pane_records(home).join("owed-confirmations")
}

/// The folder of the panes the installation `install` starts, as a node
/// reads it ([`crate::route::pane_records_of`]).
pub fn folder_of(home: &Path, install: Option<&str>) -> PathBuf {
    crate::route::pane_records_of(home, install).join("owed-confirmations")
}

fn path(folder: &Path, pane: &str) -> PathBuf {
    crate::route::pane_file(folder, pane)
}

/// Keeps the confirmation of `ids`, which reached the agent of `session` in
/// `pane`, for that pane's next turn end. A confirmation already owed there
/// for the same session is kept with it, one bundle at most, and keeps its
/// first miss's time; one of another session is replaced. Refused past
/// [`OWED_PANES`] with `owed_capacity`, and past [`RECORD_LIMIT`] with
/// `owed_record`. Every pane's record past a letter's deadline, or damaged,
/// is dropped first.
pub fn owe(
    home: &Path,
    pane: &str,
    session: &str,
    ids: &[String],
    now: u64,
) -> Result<(), &'static str> {
    let folder = folder(home);
    sweep(&folder, now);
    let path = path(&folder, pane);
    let (mut owed, owed_at_unix_ms) = match read(&path) {
        Ok(Some(record)) if record.session == session => (record.ids, record.owed_at_unix_ms),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && count(&folder) >= OWED_PANES =>
        {
            return Err("owed_capacity");
        }
        // Another session's, a damaged record or one not read now is
        // replaced in its own place.
        _ => (Vec::new(), now),
    };
    for id in ids {
        if !owed.contains(id) {
            owed.push(id.clone());
        }
    }
    if owed.len() > BUNDLE_LETTERS {
        owed.drain(..owed.len() - BUNDLE_LETTERS);
    }
    if !valid_letter_ids(&owed) {
        return Err("owed_record");
    }
    let bytes = serde_json::to_vec(&Record {
        version: RECORD_VERSION,
        session: session.to_owned(),
        ids: owed,
        owed_at_unix_ms,
    })
    .map_err(|_| "owed_record")?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err("owed_record");
    }
    hide_platform::fs::private::create_dir_all(&folder).map_err(|_| "owed_record")?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|_| "owed_record")
}

/// Hands over what `pane` owes, from the records in `folder`, and forgets
/// it: the node answers the core's ask with it, and the core confirms the
/// letters of the record's session that wait for it. `None` when nothing is
/// owed, and for a record past a letter's deadline or damaged, which is
/// dropped; one the system does not let it read now is left for a later ask.
pub fn take(folder: &Path, pane: &str, now: u64) -> Option<Owed> {
    let path = path(folder, pane);
    let record = read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    record
        .filter(|record| !expired(record, now))
        .map(|record| Owed {
            session: record.session,
            ids: record.ids,
        })
}

fn expired(record: &Record, now: u64) -> bool {
    now.saturating_sub(record.owed_at_unix_ms) >= DELIVERY_EXPIRY_MS
}

/// Drops the records whose letters are past their deadline, and those that
/// are damaged; one the system did not let it read now is left alone.
fn sweep(folder: &Path, now: u64) {
    for path in records(folder) {
        if read(&path).is_ok_and(|record| record.is_none_or(|record| expired(&record, now))) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn count(folder: &Path) -> usize {
    records(folder).len()
}

/// The records in the folder; a write still on its way is a dot file beside
/// them (`hide_platform::fs::atomic`) and is not one.
fn records(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !name.starts_with('.') && name.ends_with(".json"))
        })
        .collect()
}

/// The record at `path`; `None` for a damaged one (too large, not one this
/// build reads, or naming what is not a letter id), and the system's error
/// for one it could not read.
fn read(path: &Path) -> std::io::Result<Option<Record>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Ok(None);
    }
    Ok(serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(|record| record.version == RECORD_VERSION && valid_letter_ids(&record.ids)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn taken(home: &Path, pane: &str, now: u64) -> Option<Owed> {
        take(&folder(home), pane, now)
    }

    /// The node hands over what the pane's hook could not confirm, once:
    /// the record is gone after it.
    #[test]
    fn an_owed_confirmation_is_handed_over_once_with_its_session() {
        let home = tempfile::tempdir().unwrap();
        owe(
            home.path(),
            "w1:p1",
            "s1",
            &ids(&["letter-1", "letter-2"]),
            NOW,
        )
        .unwrap();
        assert_eq!(
            taken(home.path(), "w1:p1", NOW + 1_000),
            Some(Owed {
                session: "s1".to_owned(),
                ids: ids(&["letter-1", "letter-2"]),
            })
        );
        assert_eq!(taken(home.path(), "w1:p1", NOW + 2_000), None);
        assert_eq!(
            taken(home.path(), "w1:p2", NOW + 2_000),
            None,
            "nothing owed"
        );
    }

    /// A record past a letter's deadline has letters the core already
    /// ended, and a damaged one names nothing to confirm: neither is handed
    /// over, and both are gone.
    #[test]
    fn a_past_deadline_or_a_damaged_record_is_dropped_unsent() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        assert_eq!(taken(home.path(), "w1:p1", NOW + DELIVERY_EXPIRY_MS), None);
        assert_eq!(count(&folder(home.path())), 0);
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        std::fs::write(path(&folder(home.path()), "w1:p1"), b"{").unwrap();
        assert_eq!(taken(home.path(), "w1:p1", NOW), None);
        assert_eq!(count(&folder(home.path())), 0);
    }

    /// A node reads the folder of the installation that starts its panes:
    /// the app's is the hooks' own, another's is kept apart.
    #[test]
    fn a_node_reads_the_records_of_its_own_installation() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(folder_of(home.path(), None), folder(home.path()));
        assert_eq!(
            folder_of(home.path(), Some(crate::route::APP_INSTALL)),
            folder(home.path())
        );
        assert_ne!(
            folder_of(home.path(), Some("candidate")),
            folder(home.path())
        );
    }

    /// A record larger than a read takes is not written, rather than
    /// written and dropped unread.
    #[test]
    fn a_record_past_its_size_is_refused_when_written() {
        let home = tempfile::tempdir().unwrap();
        let long: Vec<String> = (0..300).map(|n| format!("letter-{n:0>240}")).collect();
        assert_eq!(
            owe(home.path(), "w1:p1", "s1", &long, NOW),
            Err("owed_record")
        );
        assert_eq!(count(&folder(home.path())), 0);
    }

    /// Another pane's record the system does not let a miss read now is
    /// left for later, not dropped as damaged.
    #[cfg(unix)]
    #[test]
    fn a_record_not_read_now_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        let other = path(&folder(home.path()), "w1:p1");
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o000)).unwrap();
        owe(home.path(), "w1:p2", "s2", &ids(&["letter-2"]), NOW).unwrap();
        assert!(other.exists());
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// A second miss of the session does not make its letters' deadline
    /// later: the record ends a letter's deadline after its first miss.
    #[test]
    fn a_second_miss_keeps_the_first_misses_deadline() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        owe(
            home.path(),
            "w1:p1",
            "s1",
            &ids(&["letter-2"]),
            NOW + DELIVERY_EXPIRY_MS - 1,
        )
        .unwrap();
        assert_eq!(taken(home.path(), "w1:p1", NOW + DELIVERY_EXPIRY_MS), None);
    }

    /// Two misses of one session are confirmed together; one of a new
    /// session replaces the old session's.
    #[test]
    fn misses_of_one_session_are_kept_together_and_a_new_session_replaces_them() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        owe(
            home.path(),
            "w1:p1",
            "s1",
            &ids(&["letter-1", "letter-2"]),
            NOW,
        )
        .unwrap();
        let read_ids = || {
            read(&path(&folder(home.path()), "w1:p1"))
                .unwrap()
                .unwrap()
                .ids
        };
        assert_eq!(read_ids(), ids(&["letter-1", "letter-2"]));
        owe(home.path(), "w1:p1", "s2", &ids(&["letter-3"]), NOW).unwrap();
        assert_eq!(read_ids(), ids(&["letter-3"]));
    }

    /// Past the cap a new pane's confirmation is refused, which the caller
    /// reports; a pane already owing still adds to its own, and a record
    /// past its deadline frees its place.
    #[test]
    fn owed_confirmations_are_capped_by_pane_and_a_past_deadline_frees_a_place() {
        let home = tempfile::tempdir().unwrap();
        for pane in 0..OWED_PANES {
            owe(
                home.path(),
                &format!("w1:p{pane}"),
                "s1",
                &ids(&["letter-1"]),
                NOW,
            )
            .unwrap();
        }
        assert_eq!(
            owe(home.path(), "w9:p9", "s1", &ids(&["letter-2"]), NOW),
            Err("owed_capacity")
        );
        owe(home.path(), "w1:p0", "s1", &ids(&["letter-2"]), NOW).unwrap();
        owe(
            home.path(),
            "w9:p9",
            "s1",
            &ids(&["letter-2"]),
            NOW + DELIVERY_EXPIRY_MS,
        )
        .unwrap();
        assert_eq!(count(&folder(home.path())), 1);
    }
}
