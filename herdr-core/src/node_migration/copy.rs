//! The brain state's files as a core move carries them (PRD
//! core-host-node-move Q8, Q9): a quiesced copy beside the source folder, a
//! manifest of every file's digest, the check that the machine taking the
//! core can load the copy, placing it, and setting a folder's brain state
//! aside once the core has left it.
//!
//! Only stores [`MOVES_WITH_CORE`] names are copied, with Hide AI's
//! settings as [`AI_SETTINGS`]; SQLite stores are copied with `VACUUM INTO`
//! after the core stopped, so the copy is one consistent file whatever its
//! write-ahead log held. The source folder is only ever read.
//!
//! A core's machine always holds Hide AI's settings, since the move checks
//! them, so a copy always carries them. A machine taking the core that holds
//! settings choosing the same keeps its own file, and the copy's stays in
//! the folder it was placed from: that is how [`unplace`] knows the file at
//! `ai_settings` is not the copy's.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use hide_platform::path::RelPath;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AI_SETTINGS, MARKER_FILE, MOVES_WITH_CORE, Refusal, SESSION_SEARCH};

/// The most files a copy may hold (the Factory's files folder is the only
/// one that grows with use); a folder past it is refused, not truncated.
const MAX_FILES: usize = 100_000;

/// Every file of a copy, by its `/`-separated path under the copy. One
/// read from the other machine names only paths a copy carries
/// ([`carried`]), since each becomes a path on this machine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Listed")]
pub struct Manifest {
    pub files: BTreeMap<String, FileDigest>,
}

/// A manifest as the other machine wrote it, before its paths are checked.
#[derive(Deserialize)]
struct Listed {
    files: BTreeMap<String, FileDigest>,
}

impl TryFrom<Listed> for Manifest {
    type Error = String;

    fn try_from(listed: Listed) -> Result<Self, String> {
        if listed.files.len() > MAX_FILES {
            return Err(format!("names more than {MAX_FILES} files"));
        }
        for path in listed.files.keys() {
            carried(path)?;
        }
        Ok(Self {
            files: listed.files,
        })
    }
}

/// `path` as a path below a copy, when it is one a copy carries: names
/// only, none of them empty, `.` or `..`, each one this system can hold,
/// and under a store [`MOVES_WITH_CORE`] names or Hide AI's settings.
pub fn carried(path: &str) -> Result<RelPath, String> {
    let refused = |reason: &dyn std::fmt::Display| format!("{path:?}: {reason}");
    let relative = RelPath::parse(path).map_err(|error| refused(&error))?;
    relative.to_native().map_err(|error| refused(&error))?;
    let store = relative.names().next();
    if !store.is_some_and(|name| name == AI_SETTINGS || MOVES_WITH_CORE.contains(&name)) {
        return Err(refused(&"is not a file a copy carries"));
    }
    Ok(relative)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileDigest {
    pub size: u64,
    pub sha256: String,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|file| file.size).sum()
    }

    /// The files `other` holds differently from this manifest or lacks.
    pub fn differs(&self, other: &Manifest) -> Vec<String> {
        self.files
            .iter()
            .filter(|(path, digest)| other.files.get(*path) != Some(*digest))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

fn refuse(file: &Path, reason: impl std::fmt::Display) -> Refusal {
    Refusal {
        file: file.to_path_buf(),
        reason: reason.to_string(),
    }
}

/// Copies the brain state of `state_dir` and the settings at `ai_settings`
/// into the empty or absent folder `staging`, and answers the copy's
/// manifest. Run only while no core writes `state_dir`.
pub fn stage(state_dir: &Path, staging: &Path, ai_settings: &Path) -> Result<Manifest, Refusal> {
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(|error| refuse(staging, error))?;
    }
    hide_platform::fs::private::create_dir_all(staging).map_err(|error| refuse(staging, error))?;
    for name in MOVES_WITH_CORE {
        let from = state_dir.join(name);
        let to = staging.join(name);
        let metadata = match std::fs::symlink_metadata(&from) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(refuse(&from, error)),
        };
        // A global instruction kept elsewhere through a link moves as the
        // text it points to, since the link would point nowhere on the
        // receiving machine; a link the core does not read through is left
        // behind. Any other link is refused below.
        let linked = metadata.is_symlink() && *name == hide_kit::layout::GLOBAL_INSTRUCTION;
        let from = if !linked {
            from
        } else if let Some(file) = crate::global_instruction::linked_file(state_dir) {
            file
        } else {
            crate::diagnostic!(serde_json::json!({
                "component": "core_move",
                "kind": "global_instruction.link_left",
            }));
            continue;
        };
        if *name == SESSION_SEARCH && metadata.is_file() {
            stage_retention(&from, &to)?;
        } else if metadata.is_dir() {
            copy_folder(&from, &to)?;
        } else if !metadata.is_file() && !linked {
            return Err(refuse(&from, "is neither a file nor a folder"));
        } else if name.ends_with(".sqlite3") {
            vacuum_into(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|error| refuse(&from, error))?;
            hide_platform::fs::private::restrict_to_owner(&to)
                .map_err(|error| refuse(&to, error))?;
        }
    }
    let settings = staging.join(AI_SETTINGS);
    match std::fs::symlink_metadata(ai_settings) {
        Ok(metadata) if metadata.is_file() => {
            std::fs::copy(ai_settings, &settings).map_err(|error| refuse(ai_settings, error))?;
            hide_platform::fs::private::restrict_to_owner(&settings)
                .map_err(|error| refuse(&settings, error))?;
        }
        Ok(_) => return Err(refuse(ai_settings, "is not a file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(refuse(ai_settings, "is not there: Hide AI is not set up"));
        }
        Err(error) => return Err(refuse(ai_settings, error)),
    }
    digest(staging)
}

/// The old session search file as a move carries it: its `policy` table
/// alone, each project's record retention (PRD overview-device-tasks D-20),
/// so the conversation copies it once held stay behind. A file without the
/// table, or with no row in it, is not carried, which is the default for
/// every project on the new machine too. The file is only read.
fn stage_retention(from: &Path, to: &Path) -> Result<(), Refusal> {
    let source = rusqlite::Connection::open_with_flags(
        from,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| refuse(from, error))?;
    let has_table: bool = source
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='policy')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| refuse(from, error))?;
    if !has_table {
        return Ok(());
    }
    let rows = source
        .prepare("SELECT project, days FROM policy")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| refuse(from, error))?;
    if rows.is_empty() {
        return Ok(());
    }
    let mut copy = rusqlite::Connection::open(to).map_err(|error| refuse(to, error))?;
    let tx = copy.transaction().map_err(|error| refuse(to, error))?;
    tx.execute_batch("CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);")
        .map_err(|error| refuse(to, error))?;
    for (project, days) in &rows {
        tx.execute(
            "INSERT INTO policy VALUES(?1, ?2)",
            rusqlite::params![project, days],
        )
        .map_err(|error| refuse(to, error))?;
    }
    tx.commit().map_err(|error| refuse(to, error))?;
    drop(copy);
    hide_platform::fs::private::restrict_to_owner(to).map_err(|error| refuse(to, error))
}

/// A store's consistent copy, its write-ahead log folded in.
fn vacuum_into(from: &Path, to: &Path) -> Result<(), Refusal> {
    let connection = rusqlite::Connection::open_with_flags(
        from,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| refuse(from, error))?;
    connection
        .execute("VACUUM INTO ?1", [to.to_string_lossy().as_ref()])
        .map_err(|error| refuse(from, error))?;
    hide_platform::fs::private::restrict_to_owner(to).map_err(|error| refuse(to, error))
}

fn copy_folder(from: &Path, to: &Path) -> Result<(), Refusal> {
    hide_platform::fs::private::create_dir_all(to).map_err(|error| refuse(to, error))?;
    for entry in std::fs::read_dir(from).map_err(|error| refuse(from, error))? {
        let entry = entry.map_err(|error| refuse(from, error))?;
        let kind = entry
            .file_type()
            .map_err(|error| refuse(&entry.path(), error))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|error| refuse(&entry.path(), error))?;
            hide_platform::fs::private::restrict_to_owner(&target)
                .map_err(|error| refuse(&target, error))?;
        } else {
            return Err(refuse(&entry.path(), "is neither a file nor a folder"));
        }
    }
    Ok(())
}

/// The manifest of every file under `root`.
pub fn digest(root: &Path) -> Result<Manifest, Refusal> {
    let mut manifest = Manifest::default();
    let mut folders = vec![PathBuf::new()];
    while let Some(relative) = folders.pop() {
        let folder = root.join(&relative);
        for entry in std::fs::read_dir(&folder).map_err(|error| refuse(&folder, error))? {
            let entry = entry.map_err(|error| refuse(&folder, error))?;
            let kind = entry
                .file_type()
                .map_err(|error| refuse(&entry.path(), error))?;
            let name = relative.join(entry.file_name());
            if kind.is_dir() {
                folders.push(name);
                continue;
            }
            if !kind.is_file() {
                return Err(refuse(&entry.path(), "is neither a file nor a folder"));
            }
            if manifest.files.len() == MAX_FILES {
                return Err(refuse(root, format!("holds more than {MAX_FILES} files")));
            }
            let path = hide_platform::path::relative(root, &entry.path())
                .map_err(|error| refuse(&entry.path(), error))?;
            manifest
                .files
                .insert(path.into_string(), file_digest(&entry.path())?);
        }
    }
    Ok(manifest)
}

fn file_digest(path: &Path) -> Result<FileDigest, Refusal> {
    let mut file = std::fs::File::open(path).map_err(|error| refuse(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| refuse(path, error))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok(FileDigest {
        size,
        sha256: hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}

/// Whether the copy at `dir` belongs to `node`: its `node.json` names it,
/// as the move's change of owner made it. A copy for another node is
/// refused here, not when the core it would start refuses the folder.
pub fn owned_by(dir: &Path, node: &str) -> Result<(), Refusal> {
    let marker = dir.join(MARKER_FILE);
    match super::read_marker(&marker).map_err(|reason| refuse(&marker, reason))? {
        Some(owner) if owner == node => Ok(()),
        Some(owner) => Err(refuse(
            &marker,
            format!("the copy belongs to node {owner}, not to {node}"),
        )),
        None => Err(refuse(&marker, "the copy names no owner")),
    }
}

/// Whether this build can load the copy at `dir`: the UI state reads as a
/// state this build keeps, every other JSON store is a JSON object, and every
/// SQLite store passes its integrity check. The first store that does not
/// is named.
pub fn check_loadable(dir: &Path) -> Result<(), Refusal> {
    let settings = dir.join(AI_SETTINGS);
    match crate::ai::hide_ai_settings_at(&settings) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(refuse(&settings, "is not in the copy")),
        Err(reason) => return Err(refuse(&settings, reason)),
    }
    for name in MOVES_WITH_CORE {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        if *name == super::CORE_STATE {
            let (_, _, disposition) = crate::persistence::load(&path);
            if disposition == crate::persistence::LoadDisposition::Corrupt {
                let why = crate::persistence::unreadable(&path).unwrap_or_default();
                return Err(refuse(
                    &path,
                    format!("is not a UI state this build reads: {why}"),
                ));
            }
        } else if name.ends_with(".json") {
            let bytes = std::fs::read(&path).map_err(|error| refuse(&path, error))?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|error| refuse(&path, error))?;
            if !value.is_object() {
                return Err(refuse(&path, "is not a JSON object"));
            }
            // The ledger is the one store whose loader refuses what it does
            // not accept and leaves the bytes alone, so the core would start
            // with every letter, watch and registration unavailable.
            if *name == super::DELIVERY_LEDGER {
                crate::delivery::ledger::from_bytes(&bytes).map_err(|code| {
                    refuse(
                        &path,
                        format!("is not a delivery ledger this build loads: {code}"),
                    )
                })?;
            }
        } else if name.ends_with(".sqlite3") {
            // Immutable, so the check writes no log or index beside a copy
            // whose files must stay as they were sent.
            let uri = immutable_uri(&path).ok_or_else(|| refuse(&path, "is not named in UTF-8"))?;
            let connection = rusqlite::Connection::open_with_flags(
                uri,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )
            .map_err(|error| refuse(&path, error))?;
            let answer: String = connection
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .map_err(|error| refuse(&path, error))?;
            if answer != "ok" {
                return Err(refuse(
                    &path,
                    format!("failed its integrity check: {answer}"),
                ));
            }
        }
    }
    Ok(())
}

/// `path` as an SQLite URI that opens it immutable: the name's UTF-8 bytes,
/// with every byte of 0x80 or more and `%`, `?` and `#` escaped, since
/// SQLite turns each escape back into that byte. None for a name that is
/// not UTF-8.
fn immutable_uri(path: &Path) -> Option<String> {
    let mut uri = String::from("file:");
    for byte in path.to_str()?.bytes() {
        match byte {
            b'%' | b'?' | b'#' | 0x80.. => uri.push_str(&format!("%{byte:02X}")),
            _ => uri.push(char::from(byte)),
        }
    }
    uri.push_str("?immutable=1");
    Some(uri)
}

/// A store's write-ahead log and index, which SQLite replays into a store
/// of the same name.
fn sidecars(name: &str) -> impl Iterator<Item = String> + '_ {
    let store = name.ends_with(".sqlite3");
    ["-wal", "-shm"]
        .into_iter()
        .filter(move |_| store)
        .map(move |suffix| format!("{name}{suffix}"))
}

/// The brain stores `state_dir` already holds, which a move never replaces
/// or merges (PRD core-host-node-move Q6); a store's log counts, since it
/// would replay into the store placed beside it.
pub fn brain_present(state_dir: &Path) -> Vec<String> {
    MOVES_WITH_CORE
        .iter()
        .chain(super::REBUILT_AFTER_MOVE)
        .filter(|name| !matches!(**name, "mobile.json" | "phones.json"))
        .flat_map(|name| std::iter::once((*name).to_owned()).chain(sidecars(name)))
        .filter(|name| std::fs::symlink_metadata(state_dir.join(name)).is_ok())
        .collect()
}

fn remove_if_present(path: &Path) -> Result<(), Refusal> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(refuse(path, error)),
    }
}

/// A [`place`] that did not finish: why, and the names of the copy it
/// moved in and could not take back out (none when the folder is as
/// `place` found it).
#[derive(Debug)]
pub struct NotPlaced {
    pub refusal: Refusal,
    pub left: Vec<String>,
}

/// Moves the copy at `incoming` into `state_dir`, its Hide AI settings to
/// `ai_settings`, `node.json` last, and answers what it placed. All or
/// nothing: a folder that already holds brain state, or settings choosing
/// otherwise than the copy's, is refused before anything moves, and a move
/// that fails partway takes back what it moved, so a refused place leaves
/// only the machine's own files there, which its caller must never take as
/// the copy's.
pub fn place(
    incoming: &Path,
    state_dir: &Path,
    ai_settings: &Path,
) -> Result<Vec<String>, NotPlaced> {
    let refused = |refusal: Refusal| NotPlaced {
        refusal,
        left: Vec::new(),
    };
    let present = brain_present(state_dir);
    if !present.is_empty() {
        return Err(refused(refuse(
            state_dir,
            format!("already holds brain state: {}", present.join(", ")),
        )));
    }
    if !incoming.join(MARKER_FILE).is_file() {
        return Err(refused(refuse(incoming, "holds no node.json")));
    }
    let settings = incoming.join(AI_SETTINGS);
    let keep_own = match (
        crate::ai::hide_ai_settings_at(&settings),
        crate::ai::hide_ai_settings_at(ai_settings),
    ) {
        (Ok(Some(_)), Ok(None)) => false,
        (Ok(Some(copy)), Ok(Some(own))) if copy == own => true,
        (Ok(Some(_)), Ok(Some(_))) => {
            return Err(refused(refuse(
                ai_settings,
                "holds Hide AI settings unlike the copy's",
            )));
        }
        (Ok(None), _) => return Err(refused(refuse(&settings, "is not in the copy"))),
        (Err(reason), _) => return Err(refused(refuse(&settings, reason))),
        (_, Err(reason)) => return Err(refused(refuse(ai_settings, reason))),
    };
    let at = |name: &str| {
        if name == AI_SETTINGS {
            ai_settings.to_path_buf()
        } else {
            state_dir.join(name)
        }
    };
    let mut placed: Vec<String> = Vec::new();
    let names = MOVES_WITH_CORE
        .iter()
        .copied()
        .filter(|name| *name != MARKER_FILE)
        .chain((!keep_own).then_some(AI_SETTINGS))
        .chain(std::iter::once(MARKER_FILE));
    for name in names {
        let one = if name == AI_SETTINGS {
            place_settings(&settings, ai_settings)
        } else {
            place_one(incoming, state_dir, name)
        };
        match one {
            Ok(true) => placed.push(name.to_owned()),
            Ok(false) => {}
            Err(refusal) => {
                let left = placed
                    .into_iter()
                    .rev()
                    .filter(|name| std::fs::rename(at(name), incoming.join(name)).is_err())
                    .collect();
                return Err(NotPlaced { refusal, left });
            }
        }
    }
    Ok(placed)
}

fn place_settings(from: &Path, to: &Path) -> Result<bool, Refusal> {
    if let Some(folder) = to.parent() {
        hide_platform::fs::private::create_dir_all(folder)
            .map_err(|error| refuse(folder, error))?;
    }
    hide_platform::fs::atomic::rename_no_replace_path(from, to)
        .map_err(|error| refuse(to, error))?;
    Ok(true)
}

fn place_one(incoming: &Path, state_dir: &Path, name: &str) -> Result<bool, Refusal> {
    let from = incoming.join(name);
    if std::fs::symlink_metadata(&from).is_err() {
        return Ok(false);
    }
    let to = state_dir.join(name);
    // A phone pairing or Mobile setting the target kept for itself is
    // replaced by the core's; every brain store was refused above.
    if matches!(name, "mobile.json" | "phones.json") && to.exists() {
        std::fs::remove_file(&to).map_err(|error| refuse(&to, error))?;
    }
    hide_platform::fs::atomic::rename_no_replace_path(&from, &to)
        .map_err(|error| refuse(&to, error))?;
    Ok(true)
}

/// Undoes [`place`] after the core started on the copy has stopped: every
/// store and the settings it placed go back into `incoming`, where a retry
/// finds them by digest, and what that core rebuilt is removed, so the
/// folder holds no brain state.
pub fn unplace(
    state_dir: &Path,
    incoming: &Path,
    ai_settings: &Path,
) -> Result<Vec<String>, Refusal> {
    // With no copy here at all, settings at `ai_settings` are the machine's
    // own: only a copy that lacks its settings had them placed.
    let copy_here = incoming.is_dir();
    hide_platform::fs::private::create_dir_all(incoming)
        .map_err(|error| refuse(incoming, error))?;
    let mut moved = Vec::new();
    for name in MOVES_WITH_CORE.iter().filter(|name| **name != MARKER_FILE) {
        if unplace_one(state_dir, incoming, name)? {
            moved.push((*name).to_owned());
        }
    }
    let settings = incoming.join(AI_SETTINGS);
    if copy_here
        && std::fs::symlink_metadata(&settings).is_err()
        && std::fs::symlink_metadata(ai_settings).is_ok()
    {
        std::fs::rename(ai_settings, &settings).map_err(|error| refuse(ai_settings, error))?;
        moved.push(AI_SETTINGS.to_owned());
    }
    // The marker last, so a folder never holds a marker over stores that
    // are gone.
    if unplace_one(state_dir, incoming, MARKER_FILE)? {
        moved.push(MARKER_FILE.to_owned());
    }
    for name in super::REBUILT_AFTER_MOVE {
        for file in std::iter::once((*name).to_owned()).chain(sidecars(name)) {
            remove_if_present(&state_dir.join(file))?;
        }
    }
    Ok(moved)
}

fn unplace_one(state_dir: &Path, incoming: &Path, name: &str) -> Result<bool, Refusal> {
    let from = state_dir.join(name);
    if std::fs::symlink_metadata(&from).is_err() {
        return Ok(false);
    }
    let to = incoming.join(name);
    match std::fs::symlink_metadata(&to) {
        Ok(found) if found.is_dir() => {
            std::fs::remove_dir_all(&to).map_err(|error| refuse(&to, error))?;
        }
        Ok(_) => std::fs::remove_file(&to).map_err(|error| refuse(&to, error))?,
        Err(_) => {}
    }
    // The copy goes back as it was sent: a log the core started on it left
    // holds that core's writes, and one beside the copy a reader's.
    for sidecar in sidecars(name) {
        remove_if_present(&to.with_file_name(&sidecar))?;
    }
    std::fs::rename(&from, &to).map_err(|error| refuse(&from, error))?;
    for sidecar in sidecars(name) {
        remove_if_present(&state_dir.join(sidecar))?;
    }
    Ok(true)
}

/// Moves every brain store of `state_dir` and the settings at
/// `ai_settings` into `moved-out/<intent>`, replacing an earlier move's
/// (Q9), and answers what it moved.
pub fn set_aside(
    state_dir: &Path,
    intent: &str,
    ai_settings: &Path,
) -> Result<Vec<String>, Refusal> {
    let root = hide_kit::layout::moved_out(state_dir);
    let target = root.join(intent);
    if root.exists() {
        for entry in std::fs::read_dir(&root).map_err(|error| refuse(&root, error))? {
            let entry = entry.map_err(|error| refuse(&root, error))?;
            if entry.file_name() != intent {
                std::fs::remove_dir_all(entry.path())
                    .map_err(|error| refuse(&entry.path(), error))?;
            }
        }
    }
    hide_platform::fs::private::create_dir_all(&target).map_err(|error| refuse(&target, error))?;
    let mut moved = Vec::new();
    // The marker first: a folder with stores and no marker is converted as
    // a pre-node folder, one with a marker and no stores starts empty.
    let names = std::iter::once(MARKER_FILE)
        .chain(
            MOVES_WITH_CORE
                .iter()
                .copied()
                .filter(|name| *name != MARKER_FILE),
        )
        .chain(super::REBUILT_AFTER_MOVE.iter().copied())
        .filter(|name| !matches!(*name, "mobile.json" | "phones.json"));
    for name in names {
        for suffix in ["", "-wal", "-shm"] {
            let file = format!("{name}{suffix}");
            let from = state_dir.join(&file);
            if std::fs::symlink_metadata(&from).is_err() {
                continue;
            }
            let to = target.join(&file);
            if std::fs::symlink_metadata(&to).is_ok() {
                return Err(refuse(&to, "is already set aside"));
            }
            std::fs::rename(&from, &to).map_err(|error| refuse(&from, error))?;
            moved.push(file);
        }
    }
    if std::fs::symlink_metadata(ai_settings).is_ok() {
        let to = target.join(AI_SETTINGS);
        if std::fs::symlink_metadata(&to).is_ok() {
            return Err(refuse(&to, "is already set aside"));
        }
        std::fs::rename(ai_settings, &to).map_err(|error| refuse(ai_settings, error))?;
        moved.push(AI_SETTINGS.to_owned());
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SETTINGS: &str = r#"{"provider":"claude"}"#;

    /// Where a machine whose state folder is `dir` keeps Hide AI's
    /// settings: outside that folder.
    fn settings(dir: &Path) -> PathBuf {
        dir.join("account/hide/ai.json")
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(settings(dir.path()).parent().unwrap()).unwrap();
        std::fs::write(settings(dir.path()), SETTINGS).unwrap();
        std::fs::write(dir.path().join(MARKER_FILE), r#"{"version":1,"node":"n"}"#).unwrap();
        std::fs::write(
            dir.path().join("labels.json"),
            r#"{"version":1,"targets":{}}"#,
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("factory-files")).unwrap();
        std::fs::write(dir.path().join("factory-files/a.prd"), "prd").unwrap();
        let store = rusqlite::Connection::open(dir.path().join("links.sqlite3")).unwrap();
        store
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x); INSERT INTO t VALUES (1);")
            .unwrap();
        // Left open: its last write is still in the log, which a plain copy
        // of the file would miss.
        std::mem::forget(store);
        std::fs::write(dir.path().join("hided.json"), "{}").unwrap();
        let search = rusqlite::Connection::open(dir.path().join("session-search.sqlite3")).unwrap();
        search
            .execute_batch(
                "CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);
                 INSERT INTO policy VALUES('p', 30);
                 CREATE TABLE messages(id INTEGER PRIMARY KEY, body TEXT);
                 INSERT INTO messages VALUES(1, 'a copied conversation');",
            )
            .unwrap();
        drop(search);
        dir
    }

    #[test]
    fn a_place_refused_partway_takes_back_what_it_moved_and_nothing_else() {
        let incoming = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        for name in ["labels.json", "mobile.json", MARKER_FILE] {
            std::fs::write(incoming.path().join(name), name).unwrap();
        }
        std::fs::write(incoming.path().join(AI_SETTINGS), SETTINGS).unwrap();
        // The folder's own Mobile setting, which a place replaces, cannot
        // be removed: the place stops after it moved the labels in.
        std::fs::create_dir(state.path().join("mobile.json")).unwrap();
        std::fs::write(state.path().join("mobile.json/kept"), "own").unwrap();
        let not_placed = place(incoming.path(), state.path(), &settings(state.path())).unwrap_err();
        assert_eq!(not_placed.refusal.file, state.path().join("mobile.json"));
        assert!(not_placed.left.is_empty());
        assert!(!state.path().join("labels.json").exists());
        assert_eq!(
            std::fs::read_to_string(incoming.path().join("labels.json")).unwrap(),
            "labels.json"
        );
        assert!(state.path().join("mobile.json/kept").is_file());
        assert!(!settings(state.path()).exists());
    }

    /// A global instruction kept in the operator's dotfiles through a link
    /// moves as the text the core reads, a regular file on the receiving
    /// machine; a link the core does not read through stays behind and the
    /// move goes on.
    #[cfg(unix)]
    #[test]
    fn a_linked_global_instruction_moves_as_its_text_and_a_link_the_core_skips_stays_behind() {
        let name = hide_kit::layout::GLOBAL_INSTRUCTION;
        let source = folder();
        let dotfiles = tempfile::tempdir().unwrap();
        let text = dotfiles.path().join("agents.md");
        std::fs::write(&text, "Answer in Korean.\n").unwrap();
        std::os::unix::fs::symlink(&text, source.path().join(name)).unwrap();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        let moved = staging.join(name);
        assert!(std::fs::symlink_metadata(&moved).unwrap().is_file());
        assert_eq!(
            std::fs::read_to_string(&moved).unwrap(),
            "Answer in Korean.\n"
        );
        assert!(manifest.files.contains_key(name));

        let source = folder();
        let image = dotfiles.path().join("agents.png");
        std::fs::write(&image, "not text").unwrap();
        std::os::unix::fs::symlink(&image, source.path().join(name)).unwrap();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        assert!(!staging.join(name).exists());
        assert!(!manifest.files.contains_key(name));
        assert!(manifest.files.contains_key("labels.json"));
    }

    /// A manifest's paths become paths on the machine that reads it, so one
    /// that names anything but a carried file below the copy is unreadable,
    /// and the refusal names the path.
    #[test]
    fn a_manifest_read_from_the_other_machine_names_only_files_a_copy_carries() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert_eq!(
            serde_json::from_slice::<Manifest>(&bytes).unwrap(),
            manifest
        );
        for path in [
            "/var/elsewhere/.zshrc",
            "..",
            "factory-files/../../outside",
            "factory-files//a.prd",
            "factory-files/./a.prd",
            "factory-files\\..\\..\\outside",
            "notes.txt",
            "",
        ] {
            let listed = json!({"files": {path: {"size": 1, "sha256": "00"}}});
            let refused = serde_json::from_value::<Manifest>(listed).unwrap_err();
            assert!(
                refused.to_string().contains(&format!("{path:?}")),
                "{path}: {refused}"
            );
        }
    }

    /// The old session search file travels as its record retention alone
    /// (PRD overview-device-tasks D-20): the copy holds the `policy` rows
    /// and none of the conversations, the source file is only read, and a
    /// file with no retention to carry is not carried.
    #[test]
    fn the_old_search_file_moves_as_its_record_retention_only() {
        let source = folder();
        let search = source.path().join("session-search.sqlite3");
        let before = std::fs::read(&search).unwrap();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        assert_eq!(
            std::fs::read(&search).unwrap(),
            before,
            "the source is only read"
        );
        let copy = rusqlite::Connection::open(staging.join("session-search.sqlite3")).unwrap();
        let tables: Vec<String> = copy
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, ["policy"]);
        let days: i64 = copy
            .query_row("SELECT days FROM policy WHERE project='p'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(days, 30);
        check_loadable(&staging).unwrap();

        rusqlite::Connection::open(&search)
            .unwrap()
            .execute_batch("DELETE FROM policy;")
            .unwrap();
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        assert!(!staging.join("session-search.sqlite3").exists());
    }

    #[test]
    fn a_copy_belongs_only_to_the_node_its_marker_names() {
        let copy = folder();
        owned_by(copy.path(), "n").unwrap();
        assert!(
            owned_by(copy.path(), "other")
                .unwrap_err()
                .reason
                .contains("node n")
        );
        std::fs::remove_file(copy.path().join(MARKER_FILE)).unwrap();
        assert!(owned_by(copy.path(), "n").is_err());
    }

    #[test]
    fn a_staging_copy_holds_the_brain_state_and_only_it() {
        let source = folder();
        let analytics_id = "223e4567-e89b-42d3-a456-426614174000";
        std::fs::write(
            hide_kit::layout::product_analytics_id(source.path()),
            analytics_id,
        )
        .unwrap();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        let names: Vec<&str> = manifest.files.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                AI_SETTINGS,
                "factory-files/a.prd",
                "labels.json",
                "links.sqlite3",
                "node.json",
                "product-analytics-id",
                "session-search.sqlite3"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(hide_kit::layout::product_analytics_id(&staging)).unwrap(),
            analytics_id
        );
        let copy = rusqlite::Connection::open(staging.join("links.sqlite3")).unwrap();
        let rows: i64 = copy
            .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "the log's last write is in the copy");
        assert_eq!(digest(&staging).unwrap(), manifest);
        check_loadable(&staging).unwrap();

        std::fs::remove_file(settings(source.path())).unwrap();
        let refusal = stage(source.path(), &staging, &settings(source.path())).unwrap_err();
        assert_eq!(
            refusal.file,
            settings(source.path()),
            "a core always holds them"
        );
    }

    #[test]
    fn a_damaged_store_is_named_by_the_load_check() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        std::fs::write(staging.join("labels.json"), "[1]").unwrap();
        let refusal = check_loadable(&staging).unwrap_err();
        assert_eq!(refusal.file, staging.join("labels.json"));
        std::fs::write(staging.join("labels.json"), "{}").unwrap();
        std::fs::write(staging.join("links.sqlite3"), "not a database").unwrap();
        assert_eq!(
            check_loadable(&staging).unwrap_err().file,
            staging.join("links.sqlite3")
        );
    }

    /// The delivery ledger is held to the reader the core loads it with: a
    /// copy whose ledger the core would refuse (and, refusing, start with
    /// every letter unavailable) is a copy the move does not place.
    #[test]
    fn a_ledger_the_core_would_refuse_is_named_by_the_load_check() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        let ledger = staging.join(super::super::DELIVERY_LEDGER);
        crate::delivery::ledger::save(&ledger, &crate::delivery::ledger::Ledger::default())
            .unwrap();
        check_loadable(&staging).unwrap();

        let mut broken: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
        broken["next_id"] = json!(0);
        std::fs::write(&ledger, serde_json::to_vec(&broken).unwrap()).unwrap();
        let refusal = check_loadable(&staging).unwrap_err();
        assert_eq!(refusal.file, ledger);
        assert!(refusal.reason.contains("ledger_unavailable"), "{refusal}");
    }

    #[test]
    fn a_copy_under_a_folder_named_outside_ascii_passes_the_load_check() {
        let source = folder();
        let staging = source.path().join("상태 é/move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        check_loadable(&staging).unwrap();
        std::fs::write(staging.join("links.sqlite3"), "not a database").unwrap();
        assert_eq!(
            check_loadable(&staging).unwrap_err().file,
            staging.join("links.sqlite3"),
            "the check read the store at that name, not another"
        );
    }

    #[test]
    fn the_load_check_leaves_the_copy_as_it_was_sent() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        // The owner change opens the copied store as the core does, which
        // leaves it in WAL mode.
        rusqlite::Connection::open(staging.join("links.sqlite3"))
            .unwrap()
            .execute_batch("PRAGMA journal_mode=WAL;")
            .unwrap();
        let manifest = digest(&staging).unwrap();
        let names = |dir: &Path| {
            let mut names: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        let before = names(&staging);
        check_loadable(&staging).unwrap();
        assert_eq!(names(&staging), before);
        assert_eq!(digest(&staging).unwrap(), manifest);
    }

    #[test]
    fn a_database_log_without_its_store_is_brain_state_and_never_returns_with_a_copy() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("links.sqlite3-wal"), "stale").unwrap();
        assert_eq!(brain_present(target.path()), vec!["links.sqlite3-wal"]);
        std::fs::remove_file(target.path().join("links.sqlite3-wal")).unwrap();
        place(&staging, target.path(), &settings(target.path())).unwrap();
        // The core started on the copy opened its stores.
        std::fs::write(target.path().join("links.sqlite3-wal"), "pending").unwrap();
        std::fs::write(staging.join("links.sqlite3-shm"), "a reader's").unwrap();
        unplace(target.path(), &staging, &settings(target.path())).unwrap();
        assert!(brain_present(target.path()).is_empty());
        assert!(!staging.join("links.sqlite3-shm").exists());
    }

    #[test]
    fn placing_refuses_a_folder_with_brain_state_and_puts_the_marker_last() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        stage(source.path(), &staging, &settings(source.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("core-state.json"), "{}").unwrap();
        let refusal = place(&staging, target.path(), &settings(target.path()))
            .unwrap_err()
            .refusal;
        assert!(refusal.reason.contains("core-state.json"), "{refusal}");
        assert!(staging.join(MARKER_FILE).exists(), "nothing moved");
        std::fs::remove_file(target.path().join("core-state.json")).unwrap();
        let placed = place(&staging, target.path(), &settings(target.path())).unwrap();
        assert_eq!(placed.last().map(String::as_str), Some(MARKER_FILE));
        assert!(target.path().join("factory-files/a.prd").is_file());
    }

    #[test]
    fn unplacing_leaves_no_brain_state_and_returns_the_copy_for_a_retry() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        place(&staging, target.path(), &settings(target.path())).unwrap();
        // What the core started on the copy rebuilt.
        std::fs::write(target.path().join("github-snapshot.json"), "{}").unwrap();
        unplace(target.path(), &staging, &settings(target.path())).unwrap();
        assert!(brain_present(target.path()).is_empty());
        assert_eq!(digest(&staging).unwrap(), manifest);
    }

    /// Q7, Q6: the settings go where the machine taking the core keeps
    /// them and come back to the copy on an unplace; settings of its own
    /// that choose the same stay, and other settings are never replaced.
    #[test]
    fn hide_ai_settings_move_with_the_copy_and_never_replace_other_settings() {
        let source = folder();
        let staging = source.path().join("move-staging/i1");
        let manifest = stage(source.path(), &staging, &settings(source.path())).unwrap();
        let target = tempfile::tempdir().unwrap();
        let theirs = settings(target.path());

        let placed = place(&staging, target.path(), &theirs).unwrap();
        assert!(placed.contains(&AI_SETTINGS.to_owned()));
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), SETTINGS);
        unplace(target.path(), &staging, &theirs).unwrap();
        assert!(!theirs.exists());
        assert_eq!(digest(&staging).unwrap(), manifest);

        // Their own file, spelled differently, choosing the same.
        let own = r#"{ "provider": "claude" }"#;
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, own).unwrap();
        let placed = place(&staging, target.path(), &theirs).unwrap();
        assert!(!placed.contains(&AI_SETTINGS.to_owned()));
        unplace(target.path(), &staging, &theirs).unwrap();
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), own);
        assert_eq!(digest(&staging).unwrap(), manifest);

        std::fs::write(&theirs, r#"{"provider":"codex"}"#).unwrap();
        let refusal = place(&staging, target.path(), &theirs).unwrap_err();
        assert_eq!(refusal.refusal.file, theirs);
        assert!(refusal.left.is_empty());
        assert!(brain_present(target.path()).is_empty(), "nothing moved");
        assert_eq!(digest(&staging).unwrap(), manifest);
    }

    #[test]
    fn setting_aside_keeps_only_the_latest_move_and_leaves_the_rest() {
        let source = folder();
        set_aside(source.path(), "i1", &settings(source.path())).unwrap();
        let moved_out = hide_kit::layout::moved_out(source.path());
        assert!(moved_out.join("i1/links.sqlite3").is_file());
        assert!(moved_out.join("i1").join(AI_SETTINGS).is_file());
        assert!(!settings(source.path()).exists());
        assert!(moved_out.join("i1/session-search.sqlite3").is_file());
        assert!(source.path().join("hided.json").is_file());
        assert!(brain_present(source.path()).is_empty());
        std::fs::write(source.path().join(MARKER_FILE), "{}").unwrap();
        set_aside(source.path(), "i2", &settings(source.path())).unwrap();
        assert!(!moved_out.join("i1").exists());
        assert!(moved_out.join("i2").join(MARKER_FILE).is_file());
    }
}
