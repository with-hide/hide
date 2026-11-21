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
//!
//! Two builds of one installation read this folder: the one that wrote a
//! record and the one an update or a rollback put in its place (PRD
//! device-connect-iroh D-65, D-77 (8)). So a field this build does not know
//! is kept when it writes the record again, and a record it cannot read
//! (another version, or not a record at all) is another build's: it is
//! never handed over, replaced or removed for being unreadable, only once
//! the file is older than a letter's deadline, when no build has a letter
//! left to confirm by it.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::delivery::{BUNDLE_LETTERS, OWED_KEPT_MS, valid_letter_ids};

/// The record's version; one of another version is not read.
const RECORD_VERSION: u32 = 1;
/// The largest record read: one bundle's ids and a session.
const RECORD_LIMIT: u64 = 64 * 1024;
/// The most panes whose confirmations are owed at once.
pub const OWED_PANES: usize = 64;

#[derive(Debug, Deserialize, Serialize)]
struct Record {
    version: u32,
    session: String,
    ids: Vec<String>,
    owed_at_unix_ms: u64,
    /// What a later build of this installation wrote beside those: kept as
    /// it is when this build writes the session's record again.
    #[serde(flatten)]
    later: serde_json::Map<String, serde_json::Value>,
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
pub fn folder(home: &Path) -> std::io::Result<PathBuf> {
    Ok(folder_in(&crate::route::pane_records(home)?))
}

/// [`folder`] under a records folder a caller already resolved
/// ([`crate::route::records_of_state`]): only the name below it is decided
/// here, and the state folder and the installation are the caller's, applied
/// once by the route. A node, the core and `doctor` name what they own and
/// call this, never the process environment (engineering rules 5 and 7).
pub fn folder_in(records: &Path) -> PathBuf {
    records.join("owed-confirmations")
}

fn path(folder: &Path, pane: &str) -> PathBuf {
    crate::route::pane_file(folder, pane)
}

/// Keeps the confirmation of `ids`, which reached the agent of `session` in
/// `pane`, for that pane's next turn end. A confirmation already owed there
/// for the same session is kept with it, one bundle at most, and keeps its
/// first miss's time; one of another session is replaced. Refused past
/// [`OWED_PANES`] with `owed_capacity`, and with `owed_record` past
/// [`RECORD_LIMIT`] or while the pane's place holds a record this build
/// cannot read. Every pane's record past a letter's deadline is dropped
/// first.
pub fn owe(
    home: &Path,
    pane: &str,
    session: &str,
    ids: &[String],
    now: u64,
) -> Result<(), &'static str> {
    let folder = folder(home).map_err(|_| "owed_record")?;
    owe_in(&folder, pane, session, ids, now)
}

/// [`owe`] in a records folder named directly, as a test or a stand-in node
/// keeps its records without reading this process's environment.
pub fn owe_in(
    folder: &Path,
    pane: &str,
    session: &str,
    ids: &[String],
    now: u64,
) -> Result<(), &'static str> {
    sweep(folder, now);
    let path = path(folder, pane);
    let (mut owed, owed_at_unix_ms, later) = match read(&path) {
        Ok(Some(record)) if record.session == session => {
            (record.ids, record.owed_at_unix_ms, record.later)
        }
        // Another build's record, still inside a letter's deadline: it
        // keeps its place, and this miss is reported as not kept.
        Ok(None) => return Err("owed_record"),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && count(folder) >= OWED_PANES =>
        {
            return Err("owed_capacity");
        }
        // Another session's record, or one not read now, is replaced in
        // its own place.
        _ => (Vec::new(), now, serde_json::Map::new()),
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
        later,
    })
    .map_err(|_| "owed_record")?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err("owed_record");
    }
    hide_platform::fs::private::create_dir_all(folder).map_err(|_| "owed_record")?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|_| "owed_record")
}

/// Hands over what `pane` owes, from the records in `folder`, and forgets
/// it: the node answers the core's ask with it, and the core confirms the
/// letters of the record's session that wait for it. `None` when nothing is
/// owed, for a record past a letter's deadline, which is dropped, and for
/// one this build cannot read, which stays for the build that can; one the
/// system does not let it take now is left for a later ask.
///
/// The record is moved aside before it is read, so a hook that misses
/// again meanwhile writes a record of its own for the next ask rather than
/// adding to one being forgotten. A record of a session that ended in the
/// pane is handed over and forgotten too: the core confirms nothing by it,
/// and its letters end at their deadline as they would.
pub fn take(folder: &Path, pane: &str, now: u64) -> Option<Owed> {
    let path = path(folder, pane);
    let file_name = path.file_name()?.to_str()?;
    // Another build's record is not moved from its place.
    read(&path).ok()??;
    let taken = folder.join(format!(".{file_name}.taken"));
    hide_platform::fs::atomic::replace_file(&path, &taken).ok()?;
    let record = match read(&taken) {
        Ok(Some(record)) => record,
        Ok(None) | Err(_) => {
            // Back under its name for a later ask, unless a new miss took
            // the name meanwhile.
            if hide_platform::fs::atomic::rename_no_replace_path(&taken, &path).is_err() {
                let _ = std::fs::remove_file(&taken);
            }
            return None;
        }
    };
    let _ = std::fs::remove_file(&taken);
    (!expired(&record, now)).then_some(Owed {
        session: record.session,
        ids: record.ids,
    })
}

fn expired(record: &Record, now: u64) -> bool {
    now.saturating_sub(record.owed_at_unix_ms) >= OWED_KEPT_MS
}

/// Drops the records whose letters are past their deadline: one this build
/// reads by its first miss, and one it cannot read by when its file was
/// last written. One the system did not let it read now is left alone.
fn sweep(folder: &Path, now: u64) {
    for path in records(folder) {
        let past = match read(&path) {
            Ok(Some(record)) => expired(&record, now),
            Ok(None) => written_before_deadline(&path, now),
            Err(_) => false,
        };
        if past {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Whether the file at `path` was last written a letter's deadline ago or
/// more; false when the system does not say when.
fn written_before_deadline(path: &Path, now: u64) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|written| written.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|written| now.saturating_sub(written.as_millis() as u64) >= OWED_KEPT_MS)
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

/// The record at `path`; `None` for one this build cannot read (too large,
/// of another version, not a record, or naming what is not a letter id),
/// and the system's error for one it could not open.
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
        take(&folder_of_home(home), pane, now)
    }

    fn folder_of_home(home: &Path) -> PathBuf {
        folder(home).expect("the account's own records folder")
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
    /// ended: it is not handed over, and it is gone.
    #[test]
    fn a_past_deadline_record_is_dropped_unsent() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        assert_eq!(taken(home.path(), "w1:p1", NOW + OWED_KEPT_MS), None);
        assert_eq!(count(&folder_of_home(home.path())), 0);
    }

    /// PRD device-connect-iroh D-77 (8): a rollback runs an older build
    /// over the records a newer one wrote. What this build cannot read is
    /// the other build's: it is not handed over, a miss does not replace
    /// it, and no sweep removes it while a letter of it can still be
    /// confirmed.
    #[test]
    fn a_record_another_build_wrote_is_left_in_its_place_until_its_deadline() {
        let home = tempfile::tempdir().unwrap();
        let folder = folder_of_home(home.path());
        std::fs::create_dir_all(&folder).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let newer = br#"{"version":2,"bundles":[{"session":"s1","ids":["letter-1"]}]}"#;
        let record = path(&folder, "w1:p1");
        std::fs::write(&record, newer).unwrap();

        assert_eq!(take(&folder, "w1:p1", now), None);
        assert_eq!(
            owe_in(&folder, "w1:p1", "s1", &ids(&["letter-2"]), now),
            Err("owed_record"),
            "the miss is reported, and the other build's record keeps its place"
        );
        owe_in(&folder, "w1:p2", "s1", &ids(&["letter-3"]), now).unwrap();
        assert_eq!(std::fs::read(&record).unwrap(), newer);

        // Once no letter can be confirmed by it any more, it frees its
        // place. The file was written after `now` was read, so the deadline
        // is counted with a minute to spare.
        owe_in(
            &folder,
            "w1:p3",
            "s1",
            &ids(&["letter-4"]),
            now + OWED_KEPT_MS + 60_000,
        )
        .unwrap();
        assert!(!record.exists());
    }

    /// PRD device-connect-iroh D-65: a field a later build added to a
    /// record this build still reads is carried through this build's own
    /// write of it, so an update, a rollback and an update again lose
    /// nothing.
    #[test]
    fn a_field_a_later_build_added_is_kept_when_this_build_writes_the_record_again() {
        let home = tempfile::tempdir().unwrap();
        let folder = folder_of_home(home.path());
        std::fs::create_dir_all(&folder).unwrap();
        let record = path(&folder, "w1:p1");
        std::fs::write(
            &record,
            format!(
                r#"{{"version":1,"session":"s1","ids":["letter-1"],"owed_at_unix_ms":{NOW},"reached":{{"letter-1":"printed"}}}}"#
            ),
        )
        .unwrap();
        owe_in(&folder, "w1:p1", "s1", &ids(&["letter-2"]), NOW + 1).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        assert_eq!(written["ids"], serde_json::json!(["letter-1", "letter-2"]));
        assert_eq!(
            written["reached"],
            serde_json::json!({"letter-1": "printed"})
        );
        assert_eq!(written["owed_at_unix_ms"], NOW);
        assert_eq!(
            take(&folder, "w1:p1", NOW + 2),
            Some(Owed {
                session: "s1".to_owned(),
                ids: ids(&["letter-1", "letter-2"]),
            })
        );
    }

    /// A record being taken is moved aside first, so a hook that misses
    /// meanwhile keeps a record of its own for the next ask instead of
    /// adding to the one being forgotten.
    #[test]
    fn a_miss_after_a_take_is_kept_for_the_next_ask() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        assert!(taken(home.path(), "w1:p1", NOW).is_some());
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-2"]), NOW).unwrap();
        assert_eq!(
            taken(home.path(), "w1:p1", NOW).map(|owed| owed.ids),
            Some(ids(&["letter-2"]))
        );
        assert_eq!(count(&folder_of_home(home.path())), 0);
    }

    /// A node reads the folder of the installation that starts its panes by
    /// handing the route the state folder and the installation it owns: the
    /// app's folder's `owed-confirmations` is the hooks' own, and another
    /// installation's sits under its mark, decided once by the route
    /// (engineering rule 5).
    #[test]
    fn a_node_reads_the_records_of_its_own_installation() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let own = folder_in(&crate::route::records_of_state(&home, Some(&state), None).unwrap());
        assert_eq!(
            own,
            crate::route::records_in(&state).join("owed-confirmations")
        );
        assert_eq!(
            folder_in(
                &crate::route::records_of_state(
                    &home,
                    Some(&state),
                    Some(crate::route::APP_INSTALL)
                )
                .unwrap()
            ),
            own
        );
        assert_ne!(
            folder_in(
                &crate::route::records_of_state(&home, Some(&state), Some("candidate")).unwrap()
            ),
            own
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
        assert_eq!(count(&folder_of_home(home.path())), 0);
    }

    /// Another pane's record the system does not let a miss read now is
    /// left for later, not dropped as damaged.
    #[cfg(unix)]
    #[test]
    fn a_record_not_read_now_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        let other = path(&folder_of_home(home.path()), "w1:p1");
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
            NOW + OWED_KEPT_MS - 1,
        )
        .unwrap();
        assert_eq!(taken(home.path(), "w1:p1", NOW + OWED_KEPT_MS), None);
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
            read(&path(&folder_of_home(home.path()), "w1:p1"))
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
            NOW + OWED_KEPT_MS,
        )
        .unwrap();
        assert_eq!(count(&folder_of_home(home.path())), 1);
    }
}
