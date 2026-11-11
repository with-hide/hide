//! Where the Conductors' missions live on disk (mission-55, missions-files):
//! one file per mission in `missions/` (`hide_kit::layout::missions_dir`),
//! each a book of that one mission in the book's schema, beside an index
//! that holds the next id and one line per mission (`MissionLine`). A change
//! to one mission rewrites its file alone, and the index only when a line
//! changed, so the record grows by files rather than by one file every
//! change rewrites (engineering #15).
//!
//! Every changed body is written before its index line, including a take,
//! create or reopen. A failed body write leaves the previous index intact.
//! A file no line names is a create or delete interrupted before its index
//! was saved; startup removes it. Builds before the folder kept
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
        if index.exists() {
            index
        } else {
            self.old.clone()
        }
    }

    pub(crate) fn restore(&self) -> Restored {
        let index = hide_kit::layout::mission_index(&self.dir);
        match std::fs::read(&index) {
            Ok(bytes) => self.read_folder(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self.read_old(),
            Err(error) => Restored::Unreadable(format!("its index could not be read: {error}")),
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
        if let Err(reason) = validate_lines(&index.missions) {
            return Restored::Unreadable(reason);
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
        book.missions
            .sort_by_key(|mission| mission_number(&mission.id));
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
    if !valid_id(id) {
        return Err("has an invalid mission id".into());
    }
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

pub(crate) struct ShelfRead {
    pub(crate) id: String,
    pub(crate) dir: PathBuf,
    pub(crate) events: Option<PathBuf>,
    pub(crate) barrier: Option<crate::mission_events::ReadBarrier>,
}

impl ShelfRead {
    pub(crate) fn read(self) -> Result<Visiting, String> {
        if let Some(barrier) = self.barrier {
            barrier.wait()?;
        }
        read_shelved(&self.dir, self.events.as_deref(), &self.id)
    }
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
    /// shelved since, and every loaded mission whose file differs. The index
    /// follows every changed body, including changes of Conductor. Then delete files the
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
        if self.index.as_ref() != Some(&index) {
            self.replace(hide_kit::layout::MISSION_INDEX, &index)?;
            self.index = Some(index);
        }
        if moving {
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

// Only canonical allocated ids may name a file on any supported system.
fn valid_id(id: &str) -> bool {
    let n = mission_number(id);
    n > 0 && id == format!("mission-{n}")
}

fn validate_lines(lines: &[MissionLine]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for line in lines {
        if !valid_id(&line.id) || !ids.insert(&line.id) {
            return Err("its index holds an invalid or repeated mission id".into());
        }
    }
    Ok(())
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
    validate_lines(&missions)?;
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
        .filter(|id| book.get(id).is_none() && book.shelf(id).is_none())
        .collect())
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missions::tests::{mission, node, plan};
    use crate::missions::{MissionStatus, NodeKind, NodeStatus, apply_plan};

    /// A synthetic schema-4 book matching the live file shape and the
    /// reported 50-mission / 42-closed mix (2026-11-11), each with a three-node plan and its finish lines, a
    /// pull request per change node and its Conductor's session; and that
    /// book as `missions.json` holds it.
    fn real_shape() -> (MissionBook, Vec<u8>) {
        let mut book = MissionBook {
            next: 50,
            ..MissionBook::default()
        };
        for n in 1..=50u64 {
            let nodes = vec![
                node("research", NodeKind::Research, &[]),
                node("fix", NodeKind::Change, &["research"]),
                node("docs", NodeKind::Change, &["fix"]),
            ];
            let mut planned = apply_plan(&mission(), &plan(nodes), n).unwrap();
            planned.id = format!("mission-{n}");
            planned.title = format!("mission {n}");
            planned.created_at_unix_ms = n * 1_000;
            planned.conductor.session = Some(crate::links::ledger::SessionIdentity {
                device_id: "node-a".into(),
                agent: "claude".into(),
                session_id: format!("session-{n}"),
            });
            planned.status = match n {
                1..=40 => MissionStatus::Done,
                41..=42 => MissionStatus::Cancelled,
                43..=49 => MissionStatus::Running,
                _ => MissionStatus::Planning,
            };
            if !planned.status.open() {
                planned.closed_at_unix_ms = Some(n * 1_000 + 500);
            }
            for (index, node) in planned.nodes.iter_mut().enumerate() {
                if planned.status == MissionStatus::Done {
                    node.status = NodeStatus::Done;
                }
                if node.kind == NodeKind::Change {
                    node.prs = vec![(n * 10 + index as u64) as u32];
                }
            }
            book.missions.push(planned);
        }
        let bytes = serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION,
            "next": book.next,
            "missions": book.missions,
        }))
        .unwrap();
        (book, bytes)
    }

    fn read(state: &Path) -> Restored {
        MissionStore::new(state).restore()
    }

    fn moved(state: &Path) -> MissionBook {
        match read(state) {
            Restored::Read(book) => book,
            _ => panic!("the folder is read"),
        }
    }

    /// c2: a schema 4 `missions.json` of the real shape moves on the first
    /// start with every mission as it was, the folder is read after, and the
    /// old file stays as it was for a downgrade.
    #[test]
    fn fifty_missions_move_into_the_folder_with_none_lost_and_the_old_file_kept() {
        let state = tempfile::tempdir().unwrap();
        let (written, bytes) = real_shape();
        std::fs::write(hide_kit::layout::missions(state.path()), &bytes).unwrap();
        let Restored::Moving(book) = read(state.path()) else {
            panic!("the old file is read while no index exists");
        };
        assert_eq!(book.missions, written.missions);
        let dir = hide_kit::layout::missions_dir(state.path());
        MissionFiles::new(dir.clone(), None)
            .write(&book, &mut Vec::new())
            .unwrap();
        let after = moved(state.path());
        assert_eq!((after.missions, after.next), (written.missions, 50));
        assert!(after.shelved.is_empty());
        assert_eq!(
            std::fs::read(hide_kit::layout::missions(state.path())).unwrap(),
            bytes
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 51);
    }

    /// c2: a move that fails partway writes no index, so the next start
    /// reads the old file as before, and the move is made again whole once
    /// the disk takes it.
    #[test]
    fn a_move_cut_short_leaves_the_old_file_read_as_before() {
        let state = tempfile::tempdir().unwrap();
        let (written, bytes) = real_shape();
        std::fs::write(hide_kit::layout::missions(state.path()), &bytes).unwrap();
        let dir = hide_kit::layout::missions_dir(state.path());
        // A folder where mission 25's file goes stops the move there.
        let blocker = hide_kit::layout::mission_file(&dir, "mission-25");
        std::fs::create_dir_all(&blocker).unwrap();
        std::fs::write(blocker.join("in-the-way"), b"").unwrap();
        let Restored::Moving(book) = read(state.path()) else {
            panic!("the old file is read");
        };
        let mut files = MissionFiles::new(dir.clone(), None);
        assert!(files.write(&book, &mut Vec::new()).is_err());
        assert!(hide_kit::layout::mission_file(&dir, "mission-1").exists());
        assert!(!hide_kit::layout::mission_index(&dir).exists());
        let Restored::Moving(again) = read(state.path()) else {
            panic!("a move cut short leaves the old file read as before");
        };
        assert_eq!(again.missions, written.missions);
        std::fs::remove_dir_all(&blocker).unwrap();
        files.write(&book, &mut Vec::new()).unwrap();
        assert_eq!(moved(state.path()).missions, written.missions);
    }

    /// Breaking change: once the folder holds the missions, a build from
    /// before that writes `missions.json` again (a downgrade) is not read
    /// back by the next start of this build, and its file is left as it
    /// wrote it.
    #[test]
    fn once_the_folder_is_the_record_the_old_file_is_neither_read_nor_written() {
        let state = tempfile::tempdir().unwrap();
        let (written, bytes) = real_shape();
        let old = hide_kit::layout::missions(state.path());
        std::fs::write(&old, &bytes).unwrap();
        let Restored::Moving(book) = read(state.path()) else {
            panic!("the old file is read");
        };
        let dir = hide_kit::layout::missions_dir(state.path());
        let mut files = MissionFiles::new(dir, None);
        files.write(&book, &mut Vec::new()).unwrap();
        // The downgraded build adds mission 51 to its file.
        let mut downgraded = written.clone();
        let mut added = written.missions[49].clone();
        added.id = "mission-51".into();
        downgraded.missions.push(added);
        let downgraded = serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION, "next": 51, "missions": downgraded.missions,
        }))
        .unwrap();
        std::fs::write(&old, &downgraded).unwrap();
        let mut book = moved(state.path());
        assert_eq!((book.missions.len(), book.next), (50, 50));
        book.missions[0].title = "changed".into();
        files.write(&book, &mut Vec::new()).unwrap();
        assert_eq!(std::fs::read(&old).unwrap(), downgraded);
    }

    /// c1: a change to one mission rewrites its file alone; the other
    /// missions' files and the index are not written again.
    #[test]
    fn a_change_to_one_mission_rewrites_its_file_alone() {
        let state = tempfile::tempdir().unwrap();
        let (mut book, _) = real_shape();
        let dir = hide_kit::layout::missions_dir(state.path());
        let mut files = MissionFiles::new(dir.clone(), None);
        files.write(&book, &mut Vec::new()).unwrap();
        // A file the writer would replace if it wrote it again.
        let marked = |id: &str| {
            let path = if id == "index" {
                hide_kit::layout::mission_index(&dir)
            } else {
                hide_kit::layout::mission_file(&dir, id)
            };
            std::fs::write(&path, b"not written again").unwrap();
            path
        };
        let others: Vec<PathBuf> = ["mission-1", "mission-49", "index"]
            .into_iter()
            .map(marked)
            .collect();
        let running = book.get_mut("mission-48").unwrap();
        running.nodes[0].status = NodeStatus::Done;
        running.updated_at_unix_ms += 1;
        files.write(&book, &mut Vec::new()).unwrap();
        for path in others {
            assert_eq!(
                std::fs::read(&path).unwrap(),
                b"not written again",
                "{path:?}"
            );
        }
        assert_eq!(
            read_mission(&dir, "mission-48").unwrap().as_ref(),
            book.get("mission-48")
        );
    }

    /// A damaged index naming a missing body drops it; a delete interrupted
    /// after the index was saved is finished at startup.
    #[test]
    fn a_start_finishes_a_delete_and_drops_a_create_a_stop_cut_short() {
        let state = tempfile::tempdir().unwrap();
        let (mut book, _) = real_shape();
        let dir = hide_kit::layout::missions_dir(state.path());
        MissionFiles::new(dir.clone(), None)
            .write(&book, &mut Vec::new())
            .unwrap();
        // The index after a delete of mission 2 and a create of mission 51,
        // with neither file changed yet.
        let deleted = book.missions.remove(1);
        let mut created = book.missions[0].clone();
        created.id = "mission-51".into();
        book.missions.push(created);
        book.next = 51;
        std::fs::write(
            hide_kit::layout::mission_index(&dir),
            encode_index(&book).unwrap(),
        )
        .unwrap();
        let store = MissionStore::new(state.path());
        let Restored::Read(read) = store.restore() else {
            panic!("the folder is read");
        };
        store.sweep(&read);
        assert!(read.get(&deleted.id).is_none() && read.get("mission-51").is_none());
        assert!(!hide_kit::layout::mission_file(&dir, &deleted.id).exists());
        assert_eq!((read.missions.len(), read.next), (49, 51));
    }

    /// Engineering #15: a start reads every open mission and every closed
    /// one that still asks someone, and the newest `LOADED_CLOSED_LIMIT` of
    /// the rest; the older ones are shelved by their lines, files kept.
    #[test]
    fn a_start_shelves_the_oldest_closed_missions_past_the_limit() {
        let state = tempfile::tempdir().unwrap();
        let (template, _) = real_shape();
        let mut book = MissionBook {
            next: 132,
            ..MissionBook::default()
        };
        for n in 1..=132u64 {
            let mut one = template.missions[0].clone();
            one.id = format!("mission-{n}");
            one.closed_at_unix_ms = Some(n);
            book.missions.push(one);
        }
        // The oldest still asks the operator; the newest is open.
        book.missions[0].decision = Some("Which base?".into());
        book.missions[131].status = MissionStatus::Running;
        book.missions[131].closed_at_unix_ms = None;
        // A legacy book over the former count limit also loads whole.
        std::fs::write(
            hide_kit::layout::missions(state.path()),
            serde_json::to_vec(&json!({
                "schema_version": 4, "next": 132, "missions": book.missions,
            }))
            .unwrap(),
        )
        .unwrap();
        let Restored::Moving(book) = MissionStore::new(state.path()).restore() else {
            panic!("all 132 legacy missions are read");
        };
        assert_eq!((book.missions.len(), book.next), (132, 132));
        let dir = hide_kit::layout::missions_dir(state.path());
        MissionFiles::new(dir.clone(), None)
            .write(&book, &mut Vec::new())
            .unwrap();
        let read = moved(state.path());
        assert_eq!(
            read.shelved
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            ["mission-2", "mission-3"]
        );
        assert_eq!(read.missions.len(), 2 + LOADED_CLOSED_LIMIT);
        assert!(read.get("mission-1").is_some() && read.get("mission-132").is_some());
        let shelved = read_shelved(&dir, None, "mission-2").unwrap();
        assert_eq!(Some(&shelved.mission), book.get("mission-2"));
    }
    /// A failed close must not mark an older, still-open on-disk body as
    /// resting: startup would otherwise shelve live work behind 128 closes.
    #[test]
    fn a_close_whose_body_write_fails_is_still_loaded_as_open_on_restart() {
        let state = tempfile::tempdir().unwrap();
        let (template, _) = real_shape();
        let mut book = MissionBook {
            next: 129,
            ..Default::default()
        };
        for n in 1..=129 {
            let mut mission = template.missions[0].clone();
            mission.id = format!("mission-{n}");
            mission.closed_at_unix_ms = Some(n);
            book.missions.push(mission);
        }
        book.missions[0].status = MissionStatus::Running;
        book.missions[0].closed_at_unix_ms = None;
        let dir = hide_kit::layout::missions_dir(state.path());
        let mut files = MissionFiles::new(dir.clone(), None);
        files.write(&book, &mut Vec::new()).unwrap();
        let body = hide_kit::layout::mission_file(&dir, "mission-1");
        let saved = std::fs::read(&body).unwrap();
        std::fs::remove_file(&body).unwrap();
        std::fs::create_dir(&body).unwrap();
        std::fs::write(body.join("block"), b"").unwrap();
        let mut closed = book.missions.remove(0);
        closed.status = MissionStatus::Done;
        closed.closed_at_unix_ms = Some(0);
        book.shelved.push(MissionLine::of(&closed));
        assert!(files.write(&book, &mut vec![closed]).is_err());
        std::fs::remove_dir_all(&body).unwrap();
        std::fs::write(&body, saved).unwrap();
        let restarted = moved(state.path());
        assert_eq!(
            restarted.get("mission-1").unwrap().status,
            MissionStatus::Running
        );
        assert!(restarted.shelf("mission-1").is_none());
    }

    /// The index never turns an arbitrary stored id into a path traversal
    /// or a reserved Windows filename; corruption stays unread and intact.
    #[test]
    fn an_invalid_or_repeated_id_leaves_the_index_unread_without_writing_outside_the_folder() {
        let state = tempfile::tempdir().unwrap();
        let dir = hide_kit::layout::missions_dir(state.path());
        let (mut book, _) = real_shape();
        let mut files = MissionFiles::new(dir.clone(), None);
        for id in ["../outside", "index", "mission-01", "mission-2"] {
            book.missions[0].id = id.into();
            assert!(files.write(&book, &mut Vec::new()).is_err());
            let mut line = MissionLine::of(&book.missions[0]);
            line.id = id.into();
            let lines = if id == "mission-2" {
                vec![line.clone(), line]
            } else {
                vec![line]
            };
            let bytes = serde_json::to_vec(&Index {
                schema_version: INDEX_SCHEMA,
                next: 50,
                missions: lines,
            })
            .unwrap();
            let path = hide_kit::layout::mission_index(&dir);
            std::fs::write(&path, &bytes).unwrap();
            assert!(matches!(read(state.path()), Restored::Unreadable(_)));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        assert!(!state.path().join("outside.json").exists());
    }
}
