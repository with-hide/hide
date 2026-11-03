//! The import record (PRD engine-bundle-4-herdr-removal D-25): one per
//! machine, in that machine's engine folder (`hide_kit::layout::
//! herdr_import_in`), the one owner of what moving its Herdr session into
//! the core's own engine leaves between two starts of the daemon.
//!
//! It holds what the Herdr save was found to hold and is not moved yet (the
//! workspaces whose Hide state the engine's cleanup must not drop until they
//! are moved, or nothing could be read and it must drop only what the
//! engine made itself), the workspaces moved (Hide's from then on, never
//! offered again, D-9), the conversations waiting to resume because the
//! Herdr they came from still has them open (D-8), the move the operator
//! asked for and the core starts with, the mark that the move was committed
//! (D-27: after it no older build is started on the state folder again),
//! and what the last move came to, which the screen shows.
//!
//! Every step runs while the core starts, before the engine opens on the
//! folder: that is the one path a Herdr session takes into the engine, for a
//! move at a reboot, at the update, later, or for a read-only import (D-7),
//! so nothing else writes the engine's session then.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::engine::own::herdr_session::{self, HerdrRead, PendingResume};

/// The format this build writes and reads.
pub const FORMAT: u32 = 1;

/// The largest record read: a record of a session of 1024 workspaces with a
/// waiting conversation in each of 1024 panes stays far below it.
const READ_CAP: u64 = 4 * 1024 * 1024;

/// The import record of one machine's engine folder.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) format: u32,
    /// What the Herdr save held when last read.
    pub(crate) found: Found,
    /// The workspaces moved into the engine.
    pub(crate) moved: BTreeSet<String>,
    /// The workspaces an import was writing into the engine's session when
    /// it last ran: those the engine holds are moved, the rest are not.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) moving: BTreeSet<String>,
    pub(crate) pending: Vec<PendingResume>,
    /// The move the core starts with next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) asked: Option<Asked>,
    /// Set right before the first step of a move that cannot be undone.
    pub(crate) committed: bool,
    /// What the last move or read came to, for the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) outcome: Option<Outcome>,
}

/// What reading a machine's Herdr save found.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Found {
    /// No save, or none read yet: nothing of Herdr's is held.
    #[default]
    Nothing,
    /// The save's workspaces not moved yet, and the save as it was read, so
    /// a save Herdr changed since is offered again.
    NotMoved {
        workspaces: BTreeSet<String>,
        stamp: Stamp,
    },
    /// A save is there and could not be read.
    Unknown { reason: String },
}

/// A file's length and last change, which tell a save Herdr wrote again.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stamp {
    pub(crate) len: u64,
    pub(crate) modified_ns: u128,
}

/// A move the operator asked for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asked {
    /// Whether the agents resume: false while the Herdr the save is read
    /// from runs on with their conversations open (D-8).
    pub resume: bool,
}

/// What the last move or read came to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Moved {
        workspaces: usize,
        tabs: usize,
        agents: usize,
        waiting: usize,
        refused: Vec<RefusedWorkspace>,
    },
    /// The save could not be read: why, in one line.
    Unreadable { reason: String },
    /// Hide's Herdr could not be stopped, so nothing was moved.
    NotStopped,
}

/// A workspace a move left out, with its name and why.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefusedWorkspace {
    pub workspace: String,
    pub label: String,
    pub reason: String,
}

/// What the cleanup of a machine's pane state must leave alone (D-25).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Held {
    /// Nothing: every id the machine's engine does not hold may go.
    #[default]
    Nothing,
    /// The state of every id in these workspaces of the Herdr save.
    Workspaces(BTreeSet<String>),
    /// Everything but what the engine made itself, since the save could not
    /// be read.
    Unknown,
}

impl Held {
    /// Whether a cleanup that holds the machine's whole topology may drop the
    /// state keyed by the machine's `id` (a workspace, tab or pane id).
    pub(crate) fn releases(&self, id: &str) -> bool {
        match self {
            Self::Nothing => true,
            Self::Workspaces(workspaces) => {
                !workspaces.contains(id.split_once(':').map_or(id, |(workspace, _)| workspace))
            }
            Self::Unknown => crate::engine::own::made_by_engine(id),
        }
    }
}

impl Record {
    fn new() -> Self {
        Self {
            format: FORMAT,
            ..Self::default()
        }
    }

    /// What the cleanup must leave alone while this record stands.
    pub(crate) fn held(&self) -> Held {
        match &self.found {
            Found::Nothing => Held::Nothing,
            Found::NotMoved { workspaces, .. } if workspaces.is_empty() => Held::Nothing,
            Found::NotMoved { workspaces, .. } => Held::Workspaces(workspaces.clone()),
            Found::Unknown { .. } => Held::Unknown,
        }
    }
}

fn path_in(engine_dir: &Path) -> PathBuf {
    hide_kit::layout::herdr_import_in(engine_dir)
}

/// The record in `engine_dir`, `None` when there is none yet.
pub(crate) fn read(engine_dir: &Path) -> Result<Option<Record>, String> {
    let path = path_in(engine_dir);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be opened: {error}", path.display())),
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, READ_CAP + 1), &mut text)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    if text.len() as u64 > READ_CAP {
        return Err(format!("{} is larger than {READ_CAP} bytes", path.display()));
    }
    let record: Record = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not an import record: {error}", path.display()))?;
    if record.format != FORMAT {
        return Err(format!(
            "{} is format {}, this build reads format {FORMAT}",
            path.display(),
            record.format
        ));
    }
    Ok(Some(record))
}

/// What the cleanup of `engine_dir`'s machine must leave alone: what its
/// record holds, and everything the engine did not make while the record
/// cannot be read, since then nothing says what is Herdr's.
pub(crate) fn held(engine_dir: &Path) -> Held {
    match read(engine_dir) {
        Ok(record) => record.map(|record| record.held()).unwrap_or_default(),
        Err(reason) => {
            crate::diagnostic!(serde_json::json!({
                "component": "herdr_import",
                "kind": "herdr_import.record_unreadable",
                "reason": reason,
            }));
            Held::Unknown
        }
    }
}

fn write(engine_dir: &Path, record: &Record) -> Result<(), String> {
    let path = path_in(engine_dir);
    let bytes = serde_json::to_vec(record)
        .map_err(|error| format!("the import record could not be written out: {error}"))?;
    hide_platform::fs::private::create_dir_all(engine_dir)
        .map_err(|error| format!("{} could not be made: {error}", engine_dir.display()))?;
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{} could not be replaced: {error}", path.display()))
}

/// Reads `engine_dir`'s record, or starts one; a record that cannot be read
/// is refused, so no step runs on a guess of what it held.
fn open(engine_dir: &Path) -> Result<Record, String> {
    let mut record = read(engine_dir)?.unwrap_or_else(Record::new);
    // An import that ended while it wrote the engine's session: what the
    // engine holds of it moved, the rest did not (principle #11).
    if !record.moving.is_empty() {
        let held = crate::engine::own::saved_workspaces(engine_dir);
        let moving = std::mem::take(&mut record.moving);
        record
            .moved
            .extend(moving.into_iter().filter(|workspace| held.contains(workspace)));
    }
    Ok(record)
}

/// What a machine's Herdr save offers to move.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Look {
    /// No save, or every workspace in it moved: nothing is shown.
    #[default]
    Nothing,
    Offer(Offer),
    /// A save is there and could not be read: why, in one line.
    Unreadable(String),
}

/// What moving the save would move.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Offer {
    pub workspaces: usize,
    pub tabs: usize,
    pub agents: usize,
}

/// Reads the Herdr save at `herdr_session`, only reading it, records what of
/// it is not moved in `engine_dir`'s record, and says what moving it would
/// move. It runs at every start of the core before the engine opens, so the
/// engine's cleanup never owns a pane of the save before it is moved.
pub fn look(engine_dir: &Path, herdr_session: &Path) -> Result<Look, String> {
    let mut record = open(engine_dir)?;
    let look = match herdr_session::read(herdr_session) {
        HerdrRead::Missing => {
            record.found = Found::Nothing;
            Look::Nothing
        }
        HerdrRead::Unreadable(reason) => {
            record.found = Found::Unknown {
                reason: reason.clone(),
            };
            Look::Unreadable(reason)
        }
        HerdrRead::Read(session) => {
            let workspaces = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            record.found = Found::NotMoved {
                workspaces,
                stamp: stamp(herdr_session),
            };
            let held = crate::engine::own::saved_workspaces(engine_dir);
            let converted =
                herdr_session::convert(&session, &record.moved, &held, String::new);
            let offer = Offer {
                workspaces: converted.moved.len(),
                tabs: converted
                    .moved
                    .iter()
                    .map(|moved| moved.saved.tabs.len())
                    .sum(),
                agents: converted
                    .moved
                    .iter()
                    .flat_map(|moved| &moved.saved.tabs)
                    .flat_map(|tab| &tab.panes)
                    .filter(|pane| pane.agent.is_some())
                    .count(),
            };
            if offer.workspaces == 0 {
                Look::Nothing
            } else {
                Look::Offer(offer)
            }
        }
    };
    write(engine_dir, &record)?;
    Ok(look)
}

/// Whether a save Herdr changed since `engine_dir`'s record read it waits to
/// be read again: an offer the operator put off is made again then (D-9).
pub fn changed_since_read(engine_dir: &Path, herdr_session: &Path) -> bool {
    match read(engine_dir) {
        Ok(Some(Record {
            found: Found::NotMoved { stamp: read, .. },
            ..
        })) => read != stamp(herdr_session),
        _ => true,
    }
}

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path).map_or(Stamp::default(), |metadata| Stamp {
        len: metadata.len(),
        modified_ns: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos()),
    })
}

/// Records the move the operator asked for, which the core starts with next.
pub fn ask(engine_dir: &Path, asked: Asked) -> Result<(), String> {
    let mut record = open(engine_dir)?;
    record.asked = Some(asked);
    write(engine_dir, &record)
}

/// The move the core is to start with, if one was asked for.
pub fn asked(engine_dir: &Path) -> Result<Option<Asked>, String> {
    Ok(read(engine_dir)?.and_then(|record| record.asked))
}

/// Marks the move committed (D-27), right before its first step that cannot
/// be undone: from then on no older build is started on this state folder.
pub fn commit(engine_dir: &Path) -> Result<(), String> {
    let mut record = open(engine_dir)?;
    if record.committed {
        return Ok(());
    }
    record.committed = true;
    write(engine_dir, &record)
}

/// Whether a move was committed on `engine_dir`'s machine; a record that
/// cannot be read counts as committed, so no older build is started on a
/// guess.
pub fn committed(engine_dir: &Path) -> bool {
    read(engine_dir).map_or(true, |record| record.is_some_and(|record| record.committed))
}

/// Records that Hide's Herdr could not be stopped, so nothing was moved, and
/// puts the asked move away: the operator asks again from the screen.
pub fn not_stopped(engine_dir: &Path) -> Result<(), String> {
    let mut record = open(engine_dir)?;
    record.asked = None;
    record.outcome = Some(Outcome::NotStopped);
    write(engine_dir, &record)
}

/// Moves the workspaces of the Herdr save at `herdr_session` that no import
/// moved into the engine's session in `engine_dir` (D-3), recording which
/// moved, which were left out and why, and the conversations left waiting
/// when `asked.resume` is false; the asked move is done after it, whatever
/// it came to. A save that cannot be read moves nothing and says why.
pub fn import(engine_dir: &Path, herdr_session: &Path, asked: Asked) -> Result<Outcome, String> {
    let mut record = open(engine_dir)?;
    record.asked = None;
    let outcome = match herdr_session::read(herdr_session) {
        HerdrRead::Missing => {
            record.found = Found::Nothing;
            Outcome::Moved {
                workspaces: 0,
                tabs: 0,
                agents: 0,
                waiting: 0,
                refused: Vec::new(),
            }
        }
        HerdrRead::Unreadable(reason) => {
            record.found = Found::Unknown {
                reason: reason.clone(),
            };
            Outcome::Unreadable { reason }
        }
        HerdrRead::Read(session) => {
            record.moving = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            write(engine_dir, &record)?;
            let imported =
                herdr_session::import(engine_dir, &session, &record.moved, asked.resume)?;
            record.moving.clear();
            record.moved.extend(imported.workspaces.iter().cloned());
            let waiting = imported.pending.len();
            record.pending.extend(imported.pending);
            let workspaces = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            record.found = Found::NotMoved {
                workspaces,
                stamp: stamp(herdr_session),
            };
            Outcome::Moved {
                workspaces: imported.workspaces.len(),
                tabs: imported.tabs,
                agents: imported.agents,
                waiting,
                refused: imported
                    .refused
                    .into_iter()
                    .map(|refused| RefusedWorkspace {
                        workspace: refused.workspace,
                        label: refused.label,
                        reason: refused.reason,
                    })
                    .collect(),
            }
        }
    };
    record.outcome = Some(outcome.clone());
    write(engine_dir, &record)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("herdr").join("session.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/herdr-session/herdr-0.9.5.json"),
            &path,
        )
        .unwrap();
        path
    }

    // D-25, B38: before a move, the save's workspaces are held; once moved,
    // nothing of them is, and the save is offered no more.
    #[test]
    fn a_look_holds_the_save_s_workspaces_until_they_are_moved() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        assert_eq!(
            look(&engine, &save).unwrap(),
            Look::Offer(Offer {
                workspaces: 2,
                tabs: 3,
                agents: 1
            })
        );
        let before = held(&engine);
        assert!(!before.releases("w1:p1") && !before.releases("w2"));
        assert!(before.releases("h1:p1") && before.releases("w9:p1"));

        let outcome = import(&engine, &save, Asked { resume: true }).unwrap();
        assert!(matches!(
            outcome,
            Outcome::Moved { workspaces: 2, tabs: 3, agents: 1, waiting: 0, .. }
        ));
        assert_eq!(held(&engine), Held::Nothing);
        assert_eq!(look(&engine, &save).unwrap(), Look::Nothing);
        assert!(!changed_since_read(&engine, &save));
    }

    // D-25, B43: while the save cannot be read, only what the engine made
    // may go; a move then moves nothing and says why.
    #[test]
    fn an_unreadable_save_holds_all_but_the_engine_s_own_ids() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        std::fs::write(&save, "{").unwrap();
        assert!(matches!(look(&engine, &save).unwrap(), Look::Unreadable(_)));
        let unknown = held(&engine);
        assert!(!unknown.releases("w1:p1") && !unknown.releases("x"));
        assert!(unknown.releases("h1:p1"));
        assert!(matches!(
            import(&engine, &save, Asked { resume: true }).unwrap(),
            Outcome::Unreadable { .. }
        ));
        assert_eq!(std::fs::read_to_string(&save).unwrap(), "{", "the save is only read");
    }

    // Principle #11: an import that ended while it wrote the engine's
    // session finishes on the next start without moving anything twice.
    #[test]
    fn an_import_cut_short_converges_on_what_the_engine_holds() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        import(&engine, &save, Asked { resume: true }).unwrap();
        let mut record = read(&engine).unwrap().unwrap();
        record.moving = record.moved.clone();
        record.moved.clear();
        write(&engine, &record).unwrap();
        let again = import(&engine, &save, Asked { resume: true }).unwrap();
        assert!(matches!(again, Outcome::Moved { workspaces: 0, .. }), "{again:?}");
        assert_eq!(
            crate::engine::own::saved_workspaces(&engine),
            BTreeSet::from(["w1".to_owned(), "w2".to_owned()])
        );
    }

    // D-27: a move is committed once, and a record that cannot be read
    // counts as committed.
    #[test]
    fn a_commit_stands_and_an_unreadable_record_counts_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        assert!(!committed(&engine));
        commit(&engine).unwrap();
        assert!(committed(&engine));
        std::fs::write(path_in(&engine), "{").unwrap();
        assert!(committed(&engine));
    }
}
