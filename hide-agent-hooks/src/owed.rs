//! Letter confirmations a hook owes (`docs/delivery.md`, A confirmation
//! that missed its hook).
//!
//! A hook that printed letters to its agent and could not confirm them
//! before its own deadline, as a first run of a cold helper cannot, keeps
//! their ids here by pane, and the pane's next turn-end hook of the same
//! session confirms them; the core takes a confirmation that comes late.
//! Without it the letters, handed and never belled again, stayed pending
//! until another letter's bell brought them or their deadline passed.
//!
//! One record per pane holds one bundle. A record of another session in the
//! pane is that session's, which ended there, and is dropped; one older than
//! a letter's deadline is dropped too, since the core ended its letters
//! then. At most [`OWED_PANES`] panes owe at once: past it a new
//! confirmation is not kept and the miss is reported, and its letters end at
//! their deadline as before. The same code runs on every system and in a
//! device's hooks, under the hooks' own records folder.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::delivery::{BUNDLE_LETTERS, DELIVERY_EXPIRY_MS, Failure, valid_letter_ids};

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

/// The folder owed confirmations are kept in, under the hooks' records
/// (`hide_kit::layout::owed_confirmations` names it).
pub fn folder(home: &Path) -> PathBuf {
    crate::route::pane_records(home).join("owed-confirmations")
}

fn path(home: &Path, pane: &str) -> PathBuf {
    crate::route::pane_file(&folder(home), pane)
}

/// Keeps the confirmation of `ids`, which reached the agent of `session` in
/// `pane`, for that pane's next turn end. A confirmation already owed there
/// for the same session is kept with it, one bundle at most; one of another
/// session is replaced. Refused past [`OWED_PANES`] with `owed_capacity`.
pub fn owe(
    home: &Path,
    pane: &str,
    session: &str,
    ids: &[String],
    now: u64,
) -> Result<(), &'static str> {
    sweep(home, now);
    let path = path(home, pane);
    let mut owed = match read(&path) {
        Some(record) if record.session == session => record.ids,
        Some(_) => Vec::new(),
        None if count(home) >= OWED_PANES => return Err("owed_capacity"),
        None => Vec::new(),
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
        owed_at_unix_ms: now,
    })
    .map_err(|_| "owed_record")?;
    hide_platform::fs::private::create_dir_all(&folder(home)).map_err(|_| "owed_record")?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|_| "owed_record")
}

/// Sends the confirmation `pane` owes for `session`, its own session's turn
/// end, with `confirm`, and forgets it once sent; one that fails is kept for
/// the next turn end. A record of another session, which ended in the pane,
/// or past a letter's deadline is dropped unsent, as is any pane's record
/// past it.
pub fn settle(
    home: &Path,
    pane: &str,
    session: &str,
    now: u64,
    confirm: impl FnOnce(&[String]) -> Result<(), Failure>,
) -> Result<(), Failure> {
    sweep(home, now);
    let path = path(home, pane);
    let Some(record) = read(&path) else {
        return Ok(());
    };
    if record.session != session || expired(&record, now) {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    confirm(&record.ids)?;
    let _ = std::fs::remove_file(&path);
    Ok(())
}

fn expired(record: &Record, now: u64) -> bool {
    now.saturating_sub(record.owed_at_unix_ms) >= DELIVERY_EXPIRY_MS
}

/// Drops the records whose letters are past their deadline, and those that
/// cannot be read.
fn sweep(home: &Path, now: u64) {
    for path in records(home) {
        if read(&path).is_none_or(|record| expired(&record, now)) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn count(home: &Path) -> usize {
    records(home).len()
}

/// The records in the folder; a write still on its way is a dot file beside
/// them (`hide_platform::fs::atomic`) and is not one.
fn records(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder(home)) else {
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

fn read(path: &Path) -> Option<Record> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return None;
    }
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    (record.version == RECORD_VERSION && valid_letter_ids(&record.ids)).then_some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// Settles with a confirmation that records what it was asked and
    /// answers `answer`.
    fn settled(
        home: &Path,
        pane: &str,
        session: &str,
        now: u64,
        answer: Result<(), &'static str>,
    ) -> (Result<(), &'static str>, Option<Vec<String>>) {
        let mut asked = None;
        let result = settle(home, pane, session, now, |ids| {
            asked = Some(ids.to_vec());
            answer.map_err(Failure::from)
        })
        .map_err(|failure| failure.code);
        (result, asked)
    }

    /// The pane's next turn end of the same session confirms what its
    /// prompt hook could not, once: the record is gone after it.
    #[test]
    fn an_owed_confirmation_is_sent_by_the_next_turn_end_of_its_session_once() {
        let home = tempfile::tempdir().unwrap();
        owe(
            home.path(),
            "w1:p1",
            "s1",
            &ids(&["letter-1", "letter-2"]),
            NOW,
        )
        .unwrap();
        let (result, asked) = settled(home.path(), "w1:p1", "s1", NOW + 1_000, Ok(()));
        assert_eq!(result, Ok(()));
        assert_eq!(asked, Some(ids(&["letter-1", "letter-2"])));
        let (_, asked) = settled(home.path(), "w1:p1", "s1", NOW + 2_000, Ok(()));
        assert_eq!(asked, None, "nothing is owed any more");
    }

    /// A confirmation that fails again is kept for the next turn end.
    #[test]
    fn a_confirmation_that_fails_again_is_kept_for_the_next_turn_end() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        let (result, _) = settled(home.path(), "w1:p1", "s1", NOW + 1_000, Err("deadline"));
        assert_eq!(result, Err("deadline"));
        let (_, asked) = settled(home.path(), "w1:p1", "s1", NOW + 2_000, Ok(()));
        assert_eq!(asked, Some(ids(&["letter-1"])));
    }

    /// Another session in the pane means the one that owed ended there; a
    /// record past a letter's deadline has letters the core already ended.
    /// Neither is sent, and both are gone.
    #[test]
    fn an_ended_session_or_a_past_deadline_drops_the_record_unsent() {
        let home = tempfile::tempdir().unwrap();
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        let (_, asked) = settled(home.path(), "w1:p1", "s2", NOW + 1_000, Ok(()));
        assert_eq!(asked, None);
        assert_eq!(count(home.path()), 0);
        owe(home.path(), "w1:p1", "s1", &ids(&["letter-1"]), NOW).unwrap();
        let (_, asked) = settled(home.path(), "w1:p1", "s1", NOW + DELIVERY_EXPIRY_MS, Ok(()));
        assert_eq!(asked, None);
        assert_eq!(count(home.path()), 0);
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
        let read_ids = || read(&path(home.path(), "w1:p1")).unwrap().ids;
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
        assert_eq!(count(home.path()), 1);
    }
}
