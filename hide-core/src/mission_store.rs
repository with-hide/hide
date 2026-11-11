//! Where the Conductors' missions live on disk (mission-55, missions-files):
//! one file per mission in `missions/` (`hide_kit::layout::missions_dir`),
//! each a book of that one mission in the book's schema, beside an index
//! that holds the next id and one line per mission (`MissionLine`). A change
//! to one mission rewrites its file alone, and the index only when a line
//! changed, so the record grows by files rather than by one file every
//! change rewrites (engineering #15).
//!
//! The index is written before the files it names: it is what says a
//! mission exists and whether it is closed, so a stop between the two
//! leaves a line whose file is not there yet (a create the start drops, as
//! a save not yet made) or a file no line names (a delete the start
//! finishes). The one exception is the move: builds before the folder kept
//! every mission in `missions.json`, which is read as before while no index
//! exists, and the first save writes every mission's file and then the
//! index, so a move cut short leaves no index and the next start reads the
//! old file again. The old file is never written after it, so a downgrade
//! reads the missions as they were at the move; once the index exists the
//! folder is the record, and what a downgraded build wrote to the old file
//! stays there unread.
//!
//! In memory the core holds the open missions, the closed ones that still
//! ask someone, and the newest `LOADED_CLOSED_LIMIT` closed ones; an older
//! closed mission is shelved: only its index line is held, its file stays,
//! and `hide mission show` and `reopen` read it back (`read_shelved`).

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::latest_file::read_capped;
use crate::missions::{
    Decoded, Mission, MissionBook, MissionLine, READ_CAP, SCHEMA_BEFORE_CRITERIA,
    SCHEMA_BEFORE_SIMPLE, SCHEMA_VERSION, decode, encode_mission, mission_number,
};

/// Closed missions held whole, the newest by their close; past it the
/// oldest that asks no one is shelved (engineering #15). The open ones, and
/// the closed ones that still ask, are live and never shelved.
pub(crate) const LOADED_CLOSED_LIMIT: usize = 128;
const INDEX_SCHEMA: u32 = 1;
/// A line is under 200 bytes, so this reads tens of thousands of missions.
const INDEX_READ_CAP: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
struct Index {
    schema_version: u32,
    next: u64,
    missions: Vec<MissionLine>,
}

/// What a start found: no mission yet, the folder's book, the old file's
/// book the first save moves into the folder, or a record it cannot use,
/// which is kept as it is and never written over.
pub(crate) enum Restored {
    Fresh,
    Read(MissionBook),
    Moving(MissionBook),
    Unreadable(String),
}

/// Where the missions are read from at launch, before the runtime lock is
/// taken; they are written by the missions' one writer, after the
/// histories they go with (`mission_events::MissionEventStore`), through
/// [`MissionFiles`].
pub(crate) struct MissionStore {
    old: PathBuf,
    dir: PathBuf,
}

impl MissionStore {
    pub(crate) fn new(state_dir: &Path) -> Self {
        Self {
            old: hide_kit::layout::missions(state_dir),
            dir: hide_kit::layout::missions_dir(state_dir),
        }
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file a start that could not read the record names in its log.
    pub(crate) fn path(&self) -> PathBuf {
        let index = hide_kit::layout::mission_index(&self.dir);
        if index.exists() { index } else { self.old.clone() }
    }

    pub(crate) fn restore(&self) -> Restored {
        let index = hide_kit::layout::mission_index(&self.dir);
        match read_capped(&index, INDEX_READ_CAP) {
            Ok(Some(bytes)) => self.read_folder(&bytes),
            Ok(None) => self.read_old(),
            Err(reason) => Restored::Unreadable(format!("its index {reason}")),
        }
    }

    /// The folder's book: every open mission and the newest closed ones
    /// read whole, the rest shelved by their lines. A mission whose file is
    /// gone was never saved and is left out; one whose file cannot be read
    /// is shelved, so it is never written over, and logged.
    fn read_folder(&self, bytes: &[u8]) -> Restored {
        let index: Index = match serde_json::from_slice(bytes) {
            Ok(index) => index,
            Err(error) => {
                return Restored::Unreadable(format!("its index could not be decoded: {error}"));
            }
        };
        if index.schema_version != INDEX_SCHEMA {
            return Restored::Unreadable(format!(
                "its index has schema version {}, expected {INDEX_SCHEMA}",
                index.schema_version
            ));
        }
        let loaded = loaded_ids(&index.missions);
        let mut book = MissionBook {
            next: index.next,
            ..MissionBook::default()
        };
        for line in index.missions {
            if !loaded.contains(&line.id) {
                book.shelved.push(line);
                continue;
            }
            match read_mission(&self.dir, &line.id) {
                Ok(Some(mission)) => book.missions.push(mission),
                Ok(None) => crate::diagnostic!(json!({
                    "component": "missions", "kind": "missions.file_missing",
                    "mission": line.id,
                })),
                Err(reason) => {
                    crate::diagnostic!(json!({
                        "component": "missions", "kind": "missions.file_unreadable",
                        "mission": line.id, "message": reason,
                    }));
                    book.shelved.push(line);
                }
            }
        }
        book.missions.sort_by_key(|mission| mission_number(&mission.id));
        Restored::Read(book)
    }

    /// The book builds before the folder kept: an older schema's book is
    /// converted, and the file it came from is copied aside once before
    /// anything writes the current schema.
    fn read_old(&self) -> Restored {
        match read_capped(&self.old, READ_CAP) {
            Ok(None) => Restored::Fresh,
            Ok(Some(bytes)) => match decode(&bytes) {
                Ok(Decoded { book, from }) => {
                    // With no copy kept the converted book is not written
                    // over the old one: it is read as unreadable, which
                    // keeps the file as it is.
                    if from != SCHEMA_VERSION
                        && let Err(reason) = self.keep_before_conversion(&bytes, from)
                    {
                        return Restored::Unreadable(reason);
                    }
                    Restored::Moving(book)
                }
                Err(reason) => Restored::Unreadable(reason),
            },
            Err(reason) => Restored::Unreadable(reason),
        }
    }

    /// The copy a downgrade is restored from; an existing copy is the
    /// earliest one and is kept.
    fn keep_before_conversion(&self, bytes: &[u8], from: u32) -> Result<(), String> {
        let Some(directory) = self.old.parent() else {
            return Err("has no folder to keep its copy before conversion in".to_owned());
        };
        let backup = match from {
            SCHEMA_BEFORE_CRITERIA => hide_kit::layout::missions_before_criteria(directory),
            SCHEMA_BEFORE_SIMPLE => hide_kit::layout::missions_before_simple(directory),
            _ => hide_kit::layout::missions_before_gates(directory),
        };
        let outcome = crate::latest_file::keep_first_copy(&backup, bytes);
        crate::diagnostic!(json!({
            "component": "missions",
            "kind": "missions.converted",
            "from": from,
            "to": SCHEMA_VERSION,
            "backup": backup.to_string_lossy(),
            "written": outcome.as_ref().ok(),
            "message": outcome.as_ref().err().map(ToString::to_string),
        }));
        outcome
            .map(|_| ())
            .map_err(|error| format!("could not keep its copy before conversion: {error}"))
    }

    /// Deletes the files no line of the folder's book names: a delete
    /// stopped between its index and its file, or a move cut short. Called
    /// only with a book read from the folder.
    pub(crate) fn sweep(&self, book: &MissionBook) {
        let kept: BTreeSet<&str> = book
            .missions
            .iter()
            .map(|mission| mission.id.as_str())
            .chain(book.shelved.iter().map(|line| line.id.as_str()))
            .collect();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "missions.sweep_failed",
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
            if name.to_str() == Some(hide_kit::layout::MISSION_INDEX) || kept.contains(id) {
                continue;
            }
            if let Err(error) = std::fs::remove_file(entry.path()) {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "missions.sweep_failed",
                    "mission": id, "message": error.to_string(),
                }));
            }
        }
    }
}

/// The missions a start reads whole: every one that is not at rest, and
/// the newest `LOADED_CLOSED_LIMIT` that are.
fn loaded_ids(lines: &[MissionLine]) -> BTreeSet<String> {
    let mut resting: Vec<&MissionLine> = lines.iter().filter(|line| line.at_rest()).collect();
    resting.sort_by_key(|line| std::cmp::Reverse(line.closed_order()));
    resting.truncate(LOADED_CLOSED_LIMIT);
    lines
        .iter()
        .filter(|line| !line.at_rest())
        .chain(resting)
        .map(|line| line.id.clone())
        .collect()
}

/// Mission `id`'s file, read and checked to hold that one mission; `None`
/// when there is no file.
pub(crate) fn read_mission(dir: &Path, id: &str) -> Result<Option<Mission>, String> {
    let Some(bytes) = read_capped(&hide_kit::layout::mission_file(dir, id), READ_CAP)? else {
        return Ok(None);
    };
    let mut missions = decode(&bytes)?.book.missions;
    match (missions.pop(), missions.is_empty()) {
        (Some(mission), true) if mission.id == id => Ok(Some(mission)),
        _ => Err(format!("does not hold mission {id} alone")),
    }
}

/// A shelved mission read back off the lock for `hide mission show` or
/// `reopen` (`Core::mission_command`): its record, and for a reopen its
/// history, which its next events extend.
pub(crate) struct Visiting {
    pub(crate) mission: Mission,
    pub(crate) history: Option<crate::mission_events::EventLog>,
}

/// Reads shelved mission `id` from `dir`, with its history from `events`
/// when it is to be reopened.
pub(crate) fn read_shelved(
    dir: &Path,
    events: Option<&Path>,
    id: &str,
) -> Result<Visiting, String> {
    let mission = read_mission(dir, id)?.ok_or("is missing")?;
    let history = events
        .map(|events| crate::mission_events::read_log(events, id))
        .transpose()?
        .flatten();
    Ok(Visiting { mission, history })
}

/// The writer's side of the folder, kept on the missions' one save thread:
/// what each mission's file and the index hold now, so a save writes only
/// what changed. It keeps a hash of each file it wrote, not a copy.
pub(crate) struct MissionFiles {
    dir: PathBuf,
    /// The index on disk; none until the move into the folder is done.
    index: Option<Vec<u8>>,
    /// Each loaded mission's file on disk, by a hash of its bytes.
    written: HashMap<String, u64>,
}

impl MissionFiles {
    /// The writer of the folder `dir`, holding what a start read from it:
    /// `book` for a book read from the folder, none for one the first save
    /// moves there or a fresh one.
    pub(crate) fn new(dir: PathBuf, read: Option<&MissionBook>) -> Self {
        let mut files = Self {
            dir,
            index: None,
            written: HashMap::new(),
        };
        if let Some(book) = read {
            files.index = encode_index(book).ok();
            for mission in &book.missions {
                if let Ok(bytes) = encode_mission(mission) {
                    files.written.insert(mission.id.clone(), digest(&bytes));
                }
            }
        }
        files
    }

    /// Writes what changed since the last save: `leaving`, the missions
    /// shelved since, and every loaded mission whose file differs; the index
    /// before them once the folder is the record, after them while this
    /// save moves the book into it; then deletes the files of missions the
    /// index no longer names. A failure is returned with what is not yet
    /// written kept for the next save, which writes it again.
    pub(crate) fn write(
        &mut self,
        book: &MissionBook,
        leaving: &mut Vec<Mission>,
    ) -> Result<(), String> {
        hide_platform::fs::private::create_dir_all(&self.dir)
            .map_err(|error| format!("folder could not be prepared: {error}"))?;
        let index = encode_index(book)?;
        let moving = self.index.is_none();
        let gone: Vec<String> = match &self.index {
            Some(before) if *before != index => removed_ids(before, book)?,
            _ => Vec::new(),
        };
        if !moving && self.index.as_ref() != Some(&index) {
            self.replace(hide_kit::layout::MISSION_INDEX, &index)?;
            self.index = Some(index.clone());
        }
        while let Some(mission) = leaving.first() {
            let bytes = encode_mission(mission)?;
            if self.written.get(&mission.id) != Some(&digest(&bytes)) {
                self.replace(&format!("{}.json", mission.id), &bytes)?;
            }
            let mission = leaving.remove(0);
            self.written.remove(&mission.id);
        }
        for mission in &book.missions {
            let bytes = encode_mission(mission)?;
            let hash = digest(&bytes);
            if self.written.get(&mission.id) != Some(&hash) {
                self.replace(&format!("{}.json", mission.id), &bytes)?;
                self.written.insert(mission.id.clone(), hash);
            }
        }
        if moving {
            self.replace(hide_kit::layout::MISSION_INDEX, &index)?;
            self.index = Some(index);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "missions.moved",
                "missions": book.missions.len(),
            }));
        }
        self.written
            .retain(|id, _| book.missions.iter().any(|mission| mission.id == *id));
        for id in gone {
            match std::fs::remove_file(hide_kit::layout::mission_file(&self.dir, &id)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                // The next start's sweep deletes what is left.
                Err(error) => crate::diagnostic!(json!({
                    "component": "missions", "kind": "missions.delete_failed",
                    "mission": id, "message": error.to_string(),
                })),
            }
        }
        Ok(())
    }

    fn replace(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        hide_platform::fs::atomic::write_file(
            &self.dir.join(name),
            bytes,
            hide_platform::fs::Access::Private,
        )
        .map(|_| ())
        .map_err(|error| format!("{name} could not be replaced: {error}"))
    }
}

/// The index of `book`, its lines in id order so a reorder of the book
/// writes nothing.
fn encode_index(book: &MissionBook) -> Result<Vec<u8>, String> {
    let mut missions: Vec<MissionLine> = book
        .missions
        .iter()
        .map(MissionLine::of)
        .chain(book.shelved.iter().cloned())
        .collect();
    missions.sort_by_key(|line| mission_number(&line.id));
    serde_json::to_vec(&Index {
        schema_version: INDEX_SCHEMA,
        next: book.next,
        missions,
    })
    .map_err(|error| format!("the index could not be encoded: {error}"))
}

/// The missions the index `before` named that `book` holds no longer.
fn removed_ids(before: &[u8], book: &MissionBook) -> Result<Vec<String>, String> {
    let before: Index = serde_json::from_slice(before)
        .map_err(|error| format!("the index could not be decoded: {error}"))?;
    Ok(before
        .missions
        .into_iter()
        .map(|line| line.id)
        .filter(|id| {
            book.get(id).is_none() && book.shelf(id).is_none()
        })
        .collect())
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
