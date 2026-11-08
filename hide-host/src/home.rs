//! Retiring Hide's old Home folder on one machine (PRD retire-hide-home).
//!
//! Hide used to keep `~/hide`: one symlink per registered project, a
//! Hide-owned `AGENTS.md`, a `CLAUDE.md` that links or imports it, and the
//! marker `.hide-home.json` recording every link Hide made. Nothing makes the
//! folder any more; [`retire`] removes what Hide made, once, and leaves
//! everything else where it is.
//!
//! Only what the marker lists is Hide's: a link is removed while it is still
//! a symlink to the target the marker recorded, `AGENTS.md` while it is still
//! the guide Hide wrote, `CLAUDE.md` while it is still Hide's link or import,
//! and then the marker. A link the operator changed, a file they added or a
//! folder with no marker at all is never touched (D-03, D-05, B9).
//!
//! A rerun converges: the marker stays while anything else of Hide's is
//! being removed, so a retirement that stopped halfway finds the rest by it.
//! The last step is one rename: when only the marker is left and nothing keeps
//! the folder (no pane or session was recorded in it), `~/hide` is renamed to
//! `~/.hide-home-retiring-<token>`, and the marker and that folder are removed
//! there. Before the rename Hide records the name and the folder's identity in
//! its own store (`hide_kit::layout::home_retirement`); that record, and
//! the folder still being the recorded one, is the only claim on a folder of
//! that name, so a stop after the rename is finished by the next run, which
//! then finds no `~/hide`, while a `~/hide` with no marker is never touched,
//! empty or not. A file that arrives while the folder is set aside sends the
//! folder back to `~/hide` whole, marker included; if `~/hide` exists again it
//! is not replaced, the folder and the record stay and the call fails. A
//! failure at any step is returned, with the marker still on disk or the
//! folder set aside, so the next run retries it.
//!
//! Two retirements of one account's folder run one after the other: each
//! holds an exclusive lock on the account's home folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_platform::fs::atomic::{self, DurableAccess};
use hide_platform::fs::lock::{self, Mode, Waited};
use hide_platform::fs::{identity, link};
use hide_platform::path::to_wire_lossy;
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, HostError, HostResult};
pub use hide_node_link::home::HomeRetired;

/// The old Home folder's name inside the account's home directory.
pub const HOME_DIR_NAME: &str = "hide";
/// Present in a folder Hide made: which links are Hide's.
pub const MARKER_FILE: &str = ".hide-home.json";
const AGENTS_FILE: &str = "AGENTS.md";
const CLAUDE_FILE: &str = "CLAUDE.md";
const MARKER_VERSION: u32 = 1;
const RECORD_VERSION: u32 = 1;
/// The guide an earlier build wrote, piece by piece: the head, one of two
/// introductions (a Home with links, or on Windows one without), either the
/// line for no project or one bullet per project, and the tail. A file that is
/// not exactly this is the operator's.
const GUIDE_HEAD: &str = "# Hide Home\n\nThis folder is Hide's Home on this device.\n";
const GUIDE_INTRO_LINKS: &str = "Each entry is a link to a project registered in Hide.\n\n";
const GUIDE_INTRO_LINES: &str =
    "Each line names a project registered in Hide and its folder; work in that folder.\n\n";
const GUIDE_NO_PROJECT: &str = "No projects are registered on this device yet.\n";
const GUIDE_TAIL: &str = "\nWork inside a project follows that project's own AGENTS.md or CLAUDE.md.\nWork that belongs to no project can live here.\nHide rewrites this file when projects change, so edits here are not kept.\n";
/// Where the folder waits, renamed in one step, while the last of it is
/// removed. The name carries a random token and is recorded in Hide's own
/// store (`Retiring`) before the rename, with the folder's identity; a name of
/// this shape without that record is the operator's.
const RETIRING_PREFIX: &str = ".hide-home-retiring-";
/// The one line a `CLAUDE.md` that is no link held on Windows.
const CLAUDE_IMPORT: &str = "@AGENTS.md\n";
/// How long the search for a session recorded in the folder may take; a store
/// it could not finish reading counts as holding one.
const SESSION_SEARCH: Duration = Duration::from_secs(10);
/// How long a retirement waits for another one on the same folder, short of
/// the core's wait for the whole call.
const LOCK_WAIT: Duration = Duration::from_secs(20);

/// The links Hide made, as the marker file stores them.
#[derive(Deserialize)]
struct Marker {
    version: u32,
    links: BTreeMap<String, String>,
}

fn unreadable_marker() -> HostError {
    HostError::new(
        ErrorCode::Io,
        "Hide's old Home marker (~/hide/.hide-home.json) is unreadable, so Hide cannot tell which entries it made; ~/hide is left as it is",
    )
}

fn io_error(error: &std::io::Error, what: impl std::fmt::Display) -> HostError {
    HostError::io(error, format!("{what}: {error}"))
}

fn reserved(name: &str) -> bool {
    [AGENTS_FILE, CLAUDE_FILE, MARKER_FILE]
        .iter()
        .any(|file| file.to_lowercase() == name.to_lowercase())
}

/// Removes what Hide made in `<user_home>/hide` (see the module
/// documentation). `keep_empty` keeps the folder even when nothing is left in
/// it, for a caller that recorded a pane there. `settle` is the second call
/// of a folder that stays: the first leaves the marker (the record that Hide
/// made the folder) until the caller has saved what the answer means, and
/// this one removes it. Nothing is removed when the marker cannot be read.
pub fn retire(user_home: &Path, keep_empty: bool, settle: bool) -> HostResult<HomeRetired> {
    let _lock = RetireLock::take(user_home)?;
    finish_leftovers(user_home)?;
    let home_path = user_home.join(HOME_DIR_NAME);
    let mut retired = HomeRetired {
        home: to_wire_lossy(&home_path),
        kept: false,
        marked: false,
        removed: Vec::new(),
        changed: Vec::new(),
        failure: None,
    };

    match std::fs::symlink_metadata(&home_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(retired),
        Err(error) => return Err(io_error(&error, "~/hide could not be read")),
        Ok(metadata) if !metadata.is_dir() => {
            retired.kept = true;
            return Ok(retired);
        }
        Ok(_) => {}
    }
    // The canonical spelling is what the core matches its project against.
    if let Ok(canonical) = identity::canonical(&home_path) {
        retired.home = to_wire_lossy(&canonical);
    }

    // The marker's presence is the evidence that Hide made the folder; a
    // failure before it is found says nothing about the folder, and is the
    // call's.
    if !marker_present(&home_path)? {
        retired.kept = true;
        return Ok(retired);
    }
    retired.marked = true;
    if let Err(error) = clean(user_home, &home_path, keep_empty, settle, &mut retired) {
        // The folder is Hide's and the cleanup could not finish: reported with
        // the folder as it is, so a later call retries.
        retired.failure = Some(error.message);
        retired.kept = std::fs::symlink_metadata(&home_path).is_ok();
    }
    Ok(retired)
}

/// Whether the folder holds the marker file; a read failure is an error.
fn marker_present(home_path: &Path) -> HostResult<bool> {
    match std::fs::symlink_metadata(home_path.join(MARKER_FILE)) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(&error, "~/hide could not be read")),
    }
}

/// The cleanup of a folder that holds Hide's marker.
fn clean(
    user_home: &Path,
    home_path: &Path,
    keep_empty: bool,
    settle: bool,
    retired: &mut HomeRetired,
) -> HostResult<()> {
    let Some(marker) = read_marker(home_path)? else {
        return Err(unreadable_marker());
    };
    for (name, recorded) in &marker.links {
        let entry = home_path.join(name);
        match std::fs::symlink_metadata(&entry) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&error, format!("{name} could not be read"))),
            Ok(_) if link::is_link_to(&entry, Path::new(recorded)) => {
                link::remove_link(&entry).map_err(|error| {
                    io_error(&error, format!("The link {name} could not be removed"))
                })?;
                retired.removed.push(name.clone());
            }
            // The operator changed it since: theirs now.
            Ok(_) => retired.changed.push(name.clone()),
        }
    }

    if is_hide_guide(&home_path.join(AGENTS_FILE), &marker)? {
        remove_file(home_path, AGENTS_FILE, retired)?;
    }
    if is_hide_claude_file(&home_path.join(CLAUDE_FILE))? {
        remove_file(home_path, CLAUDE_FILE, retired)?;
    }

    // Only the marker left, and nothing keeps the folder: it is renamed away
    // in one step, and the rename is the record that the retirement is past
    // the point of turning back, so a stop after it is finished by the next
    // run (`finish_leftovers`) and the folder reads as gone.
    let removable = !keep_empty
        && only_the_marker_is_left(home_path)?
        && !recorded_session(user_home, home_path)?;
    if !removable {
        // A folder that stays keeps its marker until the caller has saved
        // what the answer means (`settle`), so a stop in between is asked
        // again with the same answer.
        if settle {
            remove_file(home_path, MARKER_FILE, retired)?;
        }
        retired.kept = true;
        return Ok(());
    }
    set_aside(user_home, home_path)?;
    finish_leftovers(user_home)?;
    if std::fs::symlink_metadata(home_path).is_ok() {
        // Something arrived between the look and the rename: it went back
        // with the folder, untouched, and the next run decides again.
        return Err(HostError::new(
            ErrorCode::Busy,
            "~/hide changed while it was being retired; it is left as it is",
        ));
    }
    retired.removed.push(MARKER_FILE.to_owned());
    Ok(())
}

/// A fresh name to set the folder aside under.
fn staging_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("{RETIRING_PREFIX}{:016x}", hasher.finish())
}

/// A rename that never replaces what is at the new name: the platform's own
/// where it has one, a look first where the system does not replace a folder
/// anyway.
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    match hide_platform::fs::atomic::rename_no_replace_path(from, to) {
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(from, to)
        }
        other => other,
    }
}

/// What Hide records, in its own store, before it sets `~/hide` aside: the
/// name it will set it aside under, the identity of the folder that is being
/// set aside, and the marker's text. Only a folder of that name whose identity
/// is the recorded one is Hide's to finish; a name that merely looks like a
/// staging name, or a folder that took the name since, is never touched. The
/// marker's text is kept so that a folder with a file that arrived goes back
/// to `~/hide` with its marker, as the folder Hide's cleanup stopped on.
#[derive(Deserialize, Serialize)]
struct Retiring {
    version: u32,
    staging: String,
    volume: String,
    index: String,
    marker: String,
}

fn record_path(user_home: &Path) -> PathBuf {
    hide_kit::layout::home_retirement(user_home)
}

/// Sets `~/hide` aside under a fresh name, after recording that name and the
/// folder's identity. A rename that fails (the name was taken) drops the
/// record again, so it never grants a claim on what is at that name.
fn set_aside(user_home: &Path, home_path: &Path) -> HostResult<()> {
    let staging = staging_name();
    write_record(user_home, home_path, &staging)?;
    if let Err(error) = rename_no_replace(home_path, &user_home.join(&staging)) {
        let _ = std::fs::remove_file(record_path(user_home));
        return Err(io_error(&error, "~/hide could not be set aside"));
    }
    Ok(())
}

fn write_record(user_home: &Path, home_path: &Path, staging: &str) -> HostResult<()> {
    let marker =
        std::fs::read_to_string(home_path.join(MARKER_FILE)).map_err(|_| unreadable_marker())?;
    let id = identity::file_id_nofollow(home_path)
        .map_err(|error| io_error(&error, "~/hide could not be read"))?;
    let record = record_path(user_home);
    if let Some(folder) = record.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| io_error(&error, "Hide's own folder could not be made"))?;
    }
    let bytes = serde_json::to_vec(&Retiring {
        version: RECORD_VERSION,
        staging: staging.to_owned(),
        volume: id.volume().to_string(),
        index: id.index().to_string(),
        marker,
    })
    .map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))?;
    atomic::write_file_durable(&record, &bytes, DurableAccess::Private)
        .map(|_| ())
        .map_err(|error| {
            HostError::new(
                ErrorCode::Io,
                format!("The record of the retirement could not be written: {error}"),
            )
        })
}

fn is_staging_name(name: &str) -> bool {
    name.strip_prefix(RETIRING_PREFIX).is_some_and(|token| {
        token.len() == 16
            && token
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

/// The record of a retirement that stopped after it began, if there is one:
/// a record that is no plain file, or that cannot be read, is refused.
fn read_record(user_home: &Path) -> HostResult<Option<Retiring>> {
    let path = record_path(user_home);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(io_error(
                &error,
                "The record of the retirement could not be read",
            ));
        }
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(bad_record()),
    }
    let bytes = std::fs::read(&path).map_err(|_| bad_record())?;
    let record: Retiring = serde_json::from_slice(&bytes).map_err(|_| bad_record())?;
    if record.version != RECORD_VERSION || !is_staging_name(&record.staging) {
        return Err(bad_record());
    }
    Ok(Some(record))
}

fn bad_record() -> HostError {
    HostError::new(
        ErrorCode::Io,
        "Hide's record of the ~/hide retirement is unreadable; nothing is removed",
    )
}

/// Finishes the retirement that stopped after `~/hide` was set aside: only the
/// folder the record names, and only while it still is the folder that was set
/// aside. An empty one, or one holding only a marker, goes; one that also
/// holds something else is the old folder with a file that arrived, and goes
/// back to `~/hide` whole, marker included. A record whose folder is gone, or
/// is not the recorded one (the rename never happened, or something else took
/// the name), grants nothing and is dropped.
fn finish_leftovers(user_home: &Path) -> HostResult<()> {
    let Some(record) = read_record(user_home)? else {
        return Ok(());
    };
    let retiring = user_home.join(&record.staging);
    match identity::file_id_nofollow(&retiring) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(&error, "The folder set aside could not be read")),
        Ok(id)
            if id.volume().to_string() == record.volume
                && id.index().to_string() == record.index
                && std::fs::symlink_metadata(&retiring).is_ok_and(|m| m.is_dir()) =>
        {
            finish_leftover(user_home, &retiring, &record)?;
        }
        // Not the folder that was set aside: not Hide's to touch.
        Ok(_) => {}
    }
    std::fs::remove_file(record_path(user_home))
        .map_err(|error| io_error(&error, "The record of the retirement could not be removed"))
}

fn list(folder: &Path) -> HostResult<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(folder)
        .map_err(|error| io_error(&error, "The folder set aside could not be listed"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error(&error, "The folder set aside could not be listed"))?;
        names.push(entry.file_name());
    }
    Ok(names)
}

fn finish_leftover(user_home: &Path, retiring: &Path, record: &Retiring) -> HostResult<()> {
    if list(retiring)?.iter().all(|name| name == MARKER_FILE) {
        match std::fs::remove_file(retiring.join(MARKER_FILE)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error(
                    &error,
                    "The marker set aside could not be removed",
                ));
            }
        }
        match std::fs::remove_dir(retiring) {
            Ok(()) => return Ok(()),
            // A file arrived after the look: the folder goes back below.
            Err(error) => {
                if list(retiring)?.is_empty() {
                    return Err(io_error(
                        &error,
                        "The folder set aside could not be removed",
                    ));
                }
            }
        }
    }
    // Back to its own address, with the marker it had, so the next call reads
    // it as the folder Hide's cleanup was on. A `~/hide` that exists again is
    // never replaced: the folder and the record stay, and the call fails.
    if !retiring.join(MARKER_FILE).exists() {
        std::fs::write(retiring.join(MARKER_FILE), &record.marker)
            .map_err(|error| io_error(&error, "The marker could not be put back"))?;
    }
    rename_no_replace(retiring, &user_home.join(HOME_DIR_NAME))
        .map_err(|error| io_error(&error, "The folder set aside could not go back to ~/hide"))
}

fn remove_file(home: &Path, name: &str, retired: &mut HomeRetired) -> HostResult<()> {
    match std::fs::remove_file(home.join(name)) {
        Ok(()) => {
            retired.removed.push(name.to_owned());
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(
            &error,
            format!("~/hide/{name} could not be removed"),
        )),
    }
}

/// The marker of an existing folder, or `None` when it has none (a folder
/// Hide did not make, or one an earlier retirement finished with).
fn read_marker(home_path: &Path) -> HostResult<Option<Marker>> {
    let marker_path = home_path.join(MARKER_FILE);
    match std::fs::symlink_metadata(&marker_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(&error, "~/hide could not be read")),
    }
    let bytes = std::fs::read(&marker_path).map_err(|_| unreadable_marker())?;
    let marker: Marker = serde_json::from_slice(&bytes).map_err(|_| unreadable_marker())?;
    // A recorded name is joined onto the folder to remove it; one that could
    // leave the folder or name Hide's own files is not a marker of ours.
    let unsafe_name = |name: &String| {
        name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
            || name.contains('\0')
            || reserved(name)
    };
    if marker.version != MARKER_VERSION || marker.links.keys().any(unsafe_name) {
        return Err(unreadable_marker());
    }
    Ok(Some(marker))
}

/// Whether `path` is the guide an earlier build wrote, whole: its head, one
/// introduction, the line for no project or one bullet per project (each of
/// the marker's links, in the order of their names, with their recorded
/// targets; on Windows, where the marker holds none, any `- name -> folder`
/// lines in that order), and its tail. A file the
/// operator touched in any way is theirs.
fn is_hide_guide(path: &Path, marker: &Marker) -> HostResult<bool> {
    let Some(text) = read_ours(path)? else {
        return Ok(false);
    };
    let Some(rest) = text.strip_prefix(GUIDE_HEAD) else {
        return Ok(false);
    };
    let (links_intro, rest) = if let Some(rest) = rest.strip_prefix(GUIDE_INTRO_LINKS) {
        (true, rest)
    } else if let Some(rest) = rest.strip_prefix(GUIDE_INTRO_LINES) {
        (false, rest)
    } else {
        return Ok(false);
    };
    let Some(body) = rest.strip_suffix(GUIDE_TAIL) else {
        return Ok(false);
    };
    if body == GUIDE_NO_PROJECT {
        return Ok(!links_intro || marker.links.is_empty());
    }
    // The generator wrote one bullet per project in the order of the names, so
    // the same names, in that order, once each.
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for line in body.split_inclusive('\n') {
        let Some((name, target)) = line
            .strip_prefix("- ")
            .and_then(|line| line.strip_suffix('\n'))
            .and_then(|line| line.split_once(" -> "))
        else {
            return Ok(false);
        };
        if name.is_empty()
            || target.is_empty()
            || seen.last().is_some_and(|(last, _)| *last >= name)
        {
            return Ok(false);
        }
        seen.push((name, target));
    }
    if seen.is_empty() {
        return Ok(false);
    }
    Ok(!links_intro
        || (seen.len() == marker.links.len()
            && seen
                .iter()
                .zip(&marker.links)
                .all(|((name, target), (key, recorded))| {
                    name == key && *target == to_wire_lossy(Path::new(recorded))
                })))
}

/// The text of a regular file Hide may have written, `None` when there is no
/// such file or it is not text (so it is not ours); any other failure to read
/// it is an error, never a reason to leave it or to remove it.
fn read_ours(path: &Path) -> HostResult<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(&error, "A file in ~/hide could not be read")),
        Ok(metadata) if !metadata.is_file() => return Ok(None),
        Ok(_) => {}
    }
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(io_error(&error, "A file in ~/hide could not be read")),
    }
}

fn is_hide_claude_file(path: &Path) -> HostResult<bool> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(&error, "~/hide/CLAUDE.md could not be read")),
        Ok(metadata) if metadata.is_file() => {
            Ok(read_ours(path)?.is_some_and(|text| text == CLAUDE_IMPORT))
        }
        Ok(_) => Ok(link::is_link_to(path, Path::new(AGENTS_FILE))),
    }
}

/// Whether the marker is the only entry of the folder; a folder that cannot be
/// listed is an error, not an answer.
fn only_the_marker_is_left(folder: &Path) -> HostResult<bool> {
    let entries = std::fs::read_dir(folder)
        .map_err(|error| io_error(&error, "~/hide could not be listed"))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(&error, "~/hide could not be listed"))?;
        if entry.file_name() != MARKER_FILE {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether an agent recorded a session whose folder is `home_path`. Asked last,
/// and only of a folder that would otherwise go; stores it could not read are
/// an error, so the folder is not removed on a guess.
fn recorded_session(user_home: &Path, home_path: &Path) -> HostResult<bool> {
    let catalog = hide_session::SessionCatalog::new(user_home, "");
    let found = catalog.worked_folders(0, Instant::now() + SESSION_SEARCH);
    if !found.refusals.is_empty() {
        return Err(HostError::new(
            ErrorCode::Io,
            "The agents' session stores could not all be read, so ~/hide is kept",
        ));
    }
    let wanted = identity::canonical(home_path).unwrap_or_else(|_| home_path.to_path_buf());
    Ok(found.folders.iter().any(|folder| {
        let cwd: PathBuf = identity::canonical(&folder.cwd).unwrap_or_else(|_| folder.cwd.clone());
        cwd == wanted
    }))
}

/// An exclusive lock on the account's home folder, released when dropped
/// (and when the process ends).
struct RetireLock(#[allow(dead_code)] lock::Lock);

impl RetireLock {
    fn take(user_home: &Path) -> HostResult<Self> {
        Self::take_within(user_home, LOCK_WAIT)
    }

    fn take_within(user_home: &Path, wait: Duration) -> HostResult<Self> {
        let folder = hide_platform::fs::open_dir(user_home)
            .map_err(|error| io_error(&error, "The home folder could not be opened"))?;
        match lock::lock_dir(&folder, Mode::Exclusive, wait, &|| false) {
            Ok(Waited::Locked(held)) => Ok(Self(held)),
            Ok(Waited::TimedOut | Waited::Cancelled) => Err(HostError::new(
                ErrorCode::Busy,
                "Another retirement of ~/hide is still running; try again",
            )),
            Err(error) => Err(io_error(&error, "The home folder could not be locked")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What an earlier build left: the folder, its marker, a link per project
    /// and the guide files.
    fn old_home(user_home: &Path, projects: &[&str]) -> PathBuf {
        let home = user_home.join(HOME_DIR_NAME);
        std::fs::create_dir(&home).unwrap();
        let mut links = serde_json::Map::new();
        for name in projects {
            let project = user_home.join(name);
            std::fs::create_dir(&project).unwrap();
            link::create_link(&project, &home.join(name)).unwrap();
            links.insert(
                (*name).to_owned(),
                serde_json::Value::String(project.to_string_lossy().into_owned()),
            );
        }
        std::fs::write(
            home.join(MARKER_FILE),
            serde_json::json!({"version": 1, "links": links}).to_string(),
        )
        .unwrap();
        std::fs::write(home.join(AGENTS_FILE), guide_text(user_home, projects)).unwrap();
        if cfg!(windows) {
            std::fs::write(home.join(CLAUDE_FILE), CLAUDE_IMPORT).unwrap();
        } else {
            link::create_link(Path::new(AGENTS_FILE), &home.join(CLAUDE_FILE)).unwrap();
        }
        home
    }

    /// The guide an earlier build wrote for these projects, as its generator
    /// did.
    fn guide_text(user_home: &Path, projects: &[&str]) -> String {
        let mut text = format!("{GUIDE_HEAD}{GUIDE_INTRO_LINKS}");
        if projects.is_empty() {
            text.push_str(GUIDE_NO_PROJECT);
        }
        for name in projects {
            text.push_str(&format!("- {name} -> {}\n", user_home.join(name).display()));
        }
        text.push_str(GUIDE_TAIL);
        text
    }

    /// A name Hide sets a folder aside under.
    fn aside(user_home: &Path) -> PathBuf {
        user_home.join(format!("{RETIRING_PREFIX}0123456789abcdef"))
    }

    /// What a retirement that stopped right after the rename left: the record
    /// first, then `~/hide` under the recorded name.
    fn set_aside_as(user_home: &Path, home: &Path) -> PathBuf {
        let retiring = aside(user_home);
        let name = retiring.file_name().unwrap().to_str().unwrap();
        write_record(user_home, home, name).unwrap();
        std::fs::rename(home, &retiring).unwrap();
        retiring
    }

    fn empty_of_hides_entries(user_home: &Path, home: &Path) {
        for entry in ["alpha", AGENTS_FILE, CLAUDE_FILE] {
            let path = home.join(entry);
            if link::is_link_to(&path, &user_home.join("alpha")) {
                link::remove_link(&path).unwrap();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// B5: a folder that held only Hide's own entries goes, and so do they;
    /// the projects the links led to are untouched.
    #[test]
    fn a_folder_of_hides_own_entries_is_removed_whole() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha", "beta"]);
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.marked && !retired.kept, "{retired:?}");
        assert!(!home.exists());
        assert!(dir.path().join("alpha").is_dir() && dir.path().join("beta").is_dir());
        for expected in ["alpha", "beta", AGENTS_FILE, CLAUDE_FILE, MARKER_FILE] {
            assert!(
                retired.removed.iter().any(|name| name == expected),
                "{retired:?}"
            );
        }
    }

    /// B6: a file the operator kept stays, with the folder; Hide's entries go.
    #[test]
    fn what_the_operator_kept_stays_and_hides_entries_go() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join("notes.md"), "mine").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept);
        assert_eq!(names(&home), ["notes.md"]);
        assert_eq!(
            std::fs::read_to_string(home.join("notes.md")).unwrap(),
            "mine"
        );
    }

    /// B9: a link that now points elsewhere, or became a real folder, stays.
    #[test]
    fn a_link_the_operator_changed_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha", "beta"]);
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        link::remove_link(&home.join("alpha")).unwrap();
        link::create_link(&elsewhere, &home.join("alpha")).unwrap();
        link::remove_link(&home.join("beta")).unwrap();
        std::fs::create_dir(home.join("beta")).unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept);
        assert_eq!(retired.changed, ["alpha", "beta"]);
        assert_eq!(names(&home), ["alpha", "beta"]);
        assert!(link::is_link_to(&home.join("alpha"), &elsewhere));
    }

    /// An edited guide is the operator's; so is a `CLAUDE.md` that is theirs.
    #[test]
    fn an_edited_guide_and_an_own_claude_file_are_left() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &[]);
        std::fs::write(home.join(AGENTS_FILE), "my rules\n").unwrap();
        std::fs::remove_file(home.join(CLAUDE_FILE)).unwrap();
        std::fs::write(home.join(CLAUDE_FILE), "my claude rules\n").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept);
        assert_eq!(names(&home), [AGENTS_FILE, CLAUDE_FILE]);
    }

    /// B8: a folder without Hide's marker is untouched, whatever it holds.
    #[test]
    fn a_folder_without_the_marker_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(HOME_DIR_NAME);
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join(AGENTS_FILE), format!("{GUIDE_HEAD}mine")).unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept && !retired.marked && retired.removed.is_empty());
        assert_eq!(names(&home), [AGENTS_FILE]);
    }

    /// B11: a marker that cannot be read is an error and removes nothing.
    #[test]
    fn an_unreadable_marker_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join(MARKER_FILE), "{ not json").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(
            retired.marked && retired.kept && retired.failure.is_some(),
            "{retired:?}"
        );
        assert!(link::is_link_to(
            &home.join("alpha"),
            &dir.path().join("alpha")
        ));
        assert!(home.join(AGENTS_FILE).exists());
    }

    /// A marker that names a path outside the folder is not Hide's.
    #[test]
    fn a_marker_naming_a_path_outside_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &[]);
        std::fs::write(
            home.join(MARKER_FILE),
            r#"{"version":1,"links":{"../victim":"/x"}}"#,
        )
        .unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(
            retired.failure.is_some() && retired.removed.is_empty(),
            "{retired:?}"
        );
    }

    /// B10: a second run changes nothing and reports the same state.
    #[test]
    fn retiring_again_ends_in_the_same_state() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join("notes.md"), "mine").unwrap();
        let first = retire(dir.path(), false, true).unwrap();
        let second = retire(dir.path(), false, true).unwrap();
        assert!(first.kept && second.kept);
        assert!(second.removed.is_empty() && !second.marked);
        assert_eq!(names(&home), ["notes.md"]);
        let gone = tempfile::tempdir().unwrap();
        old_home(gone.path(), &["alpha"]);
        retire(gone.path(), false, true).unwrap();
        let again = retire(gone.path(), false, true).unwrap();
        assert!(!again.kept && again.removed.is_empty());
    }

    /// B10: a retirement that stopped after some links finds the rest by the
    /// marker still on disk.
    #[test]
    fn an_interrupted_retirement_finishes_on_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha", "beta"]);
        link::remove_link(&home.join("alpha")).unwrap();
        std::fs::remove_file(home.join(AGENTS_FILE)).unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(!retired.kept, "{retired:?}");
        assert!(!home.exists());
    }

    /// A pane recorded in the folder keeps it, empty as it is.
    #[test]
    fn an_empty_folder_stays_when_the_caller_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        let retired = retire(dir.path(), true, true).unwrap();
        assert!(retired.kept);
        assert!(names(&home).is_empty());
    }

    /// B6, D-03: a guide the operator touched in any way is theirs: a title
    /// only, a sentence added under the title, one added at the end, a project
    /// line the marker does not list, or the lines in another order.
    #[test]
    fn a_guide_the_operator_changed_in_any_way_is_kept() {
        let exact = |dir: &Path| guide_text(dir, &["alpha", "beta"]);
        type Variant<'a> = (&'static str, Box<dyn Fn(&Path) -> String + 'a>);
        let variants: [Variant<'_>; 6] = [
            ("title only", Box::new(|_| "# Hide Home\n".to_owned())),
            (
                "under the title",
                Box::new(|dir| exact(dir).replacen("\n", "\nmine\n", 1)),
            ),
            (
                "at the end",
                Box::new(|dir| format!("{}my line\n", exact(dir))),
            ),
            (
                "an unlisted project",
                Box::new(|dir| {
                    exact(dir).replace("\n\nWork inside", "- gamma -> /g\n\nWork inside")
                }),
            ),
            (
                "reordered",
                Box::new(|dir| {
                    let a = format!("- alpha -> {}\n", dir.join("alpha").display());
                    let b = format!("- beta -> {}\n", dir.join("beta").display());
                    exact(dir).replace(&format!("{a}{b}"), &format!("{b}{a}"))
                }),
            ),
            (
                "a doubled bullet",
                Box::new(|dir| exact(dir).replace("- beta ->", "- alpha ->")),
            ),
        ];
        for (what, make) in variants {
            let dir = tempfile::tempdir().unwrap();
            let home = old_home(dir.path(), &["alpha", "beta"]);
            std::fs::write(home.join(AGENTS_FILE), make(dir.path())).unwrap();
            let retired = retire(dir.path(), false, true).unwrap();
            assert!(retired.kept, "{what}: {retired:?}");
            assert_eq!(names(&home), [AGENTS_FILE], "{what}");
        }
        // The same folder with the exact guide goes whole.
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha", "beta"]);
        assert!(!retire(dir.path(), false, true).unwrap().kept);
        assert!(!home.exists());
    }

    /// B10: a stop after the folder was set aside is finished by the next
    /// run, which then reads the folder as gone; a marker-less empty folder
    /// is never removed (B8).
    #[test]
    fn a_retirement_stopped_after_setting_the_folder_aside_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        for entry in ["alpha", AGENTS_FILE, CLAUDE_FILE] {
            let path = home.join(entry);
            if link::is_link_to(&path, &dir.path().join("alpha")) {
                link::remove_link(&path).unwrap();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }
        let retiring = set_aside_as(dir.path(), &home);
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(!retired.kept && !retired.marked, "{retired:?}");
        assert!(!retiring.exists() && !home.exists());

        let other = tempfile::tempdir().unwrap();
        let empty = other.path().join(HOME_DIR_NAME);
        std::fs::create_dir(&empty).unwrap();
        let retired = retire(other.path(), false, true).unwrap();
        assert!(retired.kept && !retired.marked);
        assert!(empty.is_dir());
    }

    /// B11: a folder that cannot be set aside is a failure reported with the
    /// marker still there, so a later run retires it. The restriction is
    /// checked while it is on: where it does not bind the user (root), there
    /// is nothing to prove here.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_removed_is_a_failure_and_retries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let binds = std::fs::create_dir(dir.path().join("probe")).is_err();
        let result = retire(dir.path(), false, true);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if !binds {
            return; // the mode does not bind this user: nothing to prove
        }
        let retired = result.unwrap();
        assert!(retired.failure.is_some() && retired.kept, "{retired:?}");
        assert!(home.join(MARKER_FILE).exists());
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(!retired.kept && retired.failure.is_none() && !home.exists());
    }

    /// A folder that stays keeps its marker through the first call and loses
    /// it in the second, so the caller can save its answer in between, and a
    /// repeat of either changes nothing more.
    #[test]
    fn a_folder_that_stays_keeps_its_marker_until_it_is_settled() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join("notes.md"), "mine").unwrap();
        for _ in 0..2 {
            let first = retire(dir.path(), false, false).unwrap();
            assert!(first.kept && first.marked, "{first:?}");
            assert_eq!(names(&home), [MARKER_FILE, "notes.md"]);
        }
        let second = retire(dir.path(), false, true).unwrap();
        assert!(second.kept);
        assert_eq!(names(&home), ["notes.md"]);
    }

    /// B10: a stop after the marker of the folder set aside went (an empty
    /// folder at the recorded name) is finished, and the folder reads as gone.
    #[test]
    fn a_stop_after_the_marker_set_aside_went_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        empty_of_hides_entries(dir.path(), &home);
        let retiring = set_aside_as(dir.path(), &home);
        std::fs::remove_file(retiring.join(MARKER_FILE)).unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(!retired.kept && !retired.marked, "{retired:?}");
        assert!(!retiring.exists() && !record_path(dir.path()).exists());
    }

    /// D-03, B5: a name of the set-aside shape is the operator's unless Hide
    /// recorded it for the folder it set aside: empty, with a file, or with a
    /// Hide-looking marker, it stays, and so does a marker-less `~/hide`.
    #[test]
    fn a_folder_hide_did_not_record_is_left_alone_whatever_its_name_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        let empty = aside(dir.path());
        std::fs::create_dir(&empty).unwrap();
        let other = dir
            .path()
            .join(format!("{RETIRING_PREFIX}fedcba9876543210"));
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("notes.md"), "mine").unwrap();
        std::fs::write(other.join(MARKER_FILE), "{\"version\":1,\"links\":{}}").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(!retired.kept && retired.failure.is_none(), "{retired:?}");
        assert!(!home.exists() && empty.is_dir());
        assert_eq!(names(&other), [MARKER_FILE, "notes.md"]);
    }

    /// The record written before the rename grants nothing over a folder that
    /// is not the one it was written for: the rename never happened, or the
    /// name was taken by the operator's own folder first.
    #[test]
    fn a_record_does_not_claim_a_folder_that_is_not_the_one_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        empty_of_hides_entries(dir.path(), &home);
        let retiring = aside(dir.path());
        let name = retiring.file_name().unwrap().to_str().unwrap();
        // Recorded, then the name was taken by a folder of the operator's.
        write_record(dir.path(), &home, name).unwrap();
        std::fs::create_dir(&retiring).unwrap();
        std::fs::write(retiring.join("mine.md"), "mine").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(
            !retired.kept && retired.failure.is_none() && !home.exists(),
            "{retired:?}"
        );
        assert_eq!(names(&retiring), ["mine.md"]);
        assert!(!record_path(dir.path()).exists());

        // Recorded, then stopped before the rename: the folder is as it was.
        let again = tempfile::tempdir().unwrap();
        let home = old_home(again.path(), &["alpha"]);
        write_record(again.path(), &home, name).unwrap();
        let retired = retire(again.path(), false, true).unwrap();
        assert!(!retired.kept && retired.failure.is_none(), "{retired:?}");
        assert!(!home.exists() && !record_path(again.path()).exists());
    }

    /// A record that is no plain file, or names something that is not a
    /// staging name, is refused and nothing is removed.
    #[test]
    fn a_record_outside_the_owned_boundary_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        let record = record_path(dir.path());
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(
            &record,
            r#"{"version":1,"staging":"../hide","volume":"1","index":"1","marker":"{}"}"#,
        )
        .unwrap();
        assert!(retire(dir.path(), false, true).is_err());
        assert!(home.join(MARKER_FILE).exists());
        std::fs::remove_file(&record).unwrap();
        #[cfg(unix)]
        {
            let target = dir.path().join("elsewhere.json");
            std::fs::write(&target, "{}").unwrap();
            std::os::unix::fs::symlink(&target, &record).unwrap();
            assert!(retire(dir.path(), false, true).is_err());
            assert!(home.join(MARKER_FILE).exists() && target.exists());
        }
    }

    /// B8: a marker-less `~/hide` next to anything at all is reported as kept
    /// and unmarked, never as a failure of Hide's cleanup.
    #[test]
    fn a_folder_without_the_marker_is_no_failure_whatever_is_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(HOME_DIR_NAME);
        std::fs::create_dir(&home).unwrap();
        std::fs::write(dir.path().join(".hide-home-retiring"), "mine").unwrap();
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept && !retired.marked && retired.failure.is_none());
        assert!(home.is_dir());
    }

    /// A file that arrived between the look and the rename went with the
    /// folder; it goes back, whole, and the retirement stays pending.
    #[test]
    fn a_file_that_arrives_before_the_rename_goes_back_with_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        empty_of_hides_entries(dir.path(), &home);
        std::fs::write(home.join("late.md"), "mine").unwrap();
        let retiring = set_aside_as(dir.path(), &home);
        let retired = retire(dir.path(), false, true).unwrap();
        assert!(retired.kept && retired.marked, "{retired:?}");
        assert_eq!(names(&home), ["late.md"]);
        assert!(!retiring.exists() && !record_path(dir.path()).exists());
    }

    /// B6/B10/B11: the marker went, then a file arrived, then the run stopped
    /// before the core saved anything. The next runs put the folder back at
    /// `~/hide` with its marker and keep answering marked and kept, so the
    /// core still names the project; the user's file is there at once.
    #[test]
    fn a_file_that_arrives_after_the_marker_went_comes_back_marked() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        empty_of_hides_entries(dir.path(), &home);
        let retiring = set_aside_as(dir.path(), &home);
        std::fs::remove_file(retiring.join(MARKER_FILE)).unwrap();
        std::fs::write(retiring.join("late.md"), "mine").unwrap();
        for _ in 0..2 {
            let retired = retire(dir.path(), false, false).unwrap();
            assert!(retired.kept && retired.marked, "{retired:?}");
            assert_eq!(names(&home), [MARKER_FILE, "late.md"]);
            assert!(!retiring.exists() && !record_path(dir.path()).exists());
        }
        let settled = retire(dir.path(), false, true).unwrap();
        assert!(settled.kept);
        assert_eq!(names(&home), ["late.md"]);
    }

    /// A `~/hide` that exists again is never replaced: the folder set aside
    /// and the record stay, the call fails, and a run after the operator's
    /// folder is gone finishes the return.
    #[test]
    fn a_folder_that_cannot_go_back_stays_set_aside_until_its_place_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        empty_of_hides_entries(dir.path(), &home);
        std::fs::write(home.join("late.md"), "mine").unwrap();
        let retiring = set_aside_as(dir.path(), &home);
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("new.md"), "new").unwrap();
        assert!(retire(dir.path(), false, true).is_err());
        assert_eq!(names(&home), ["new.md"]);
        assert!(names(&retiring).contains(&"late.md".to_owned()));
        assert!(record_path(dir.path()).exists());

        std::fs::remove_file(home.join("new.md")).unwrap();
        std::fs::remove_dir(&home).unwrap();
        let retired = retire(dir.path(), false, false).unwrap();
        assert!(retired.kept && retired.marked, "{retired:?}");
        assert_eq!(names(&home), [MARKER_FILE, "late.md"]);
        assert!(!retiring.exists() && !record_path(dir.path()).exists());
    }

    /// A lock held elsewhere is reported as busy within the wait.
    #[test]
    fn a_lock_held_elsewhere_is_busy_after_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let folder = hide_platform::fs::open_dir(dir.path()).unwrap();
        let holder = lock::lock_dir(&folder, Mode::Exclusive, Duration::ZERO, &|| false).unwrap();
        assert!(matches!(holder, Waited::Locked(_)));
        let Err(error) = RetireLock::take_within(dir.path(), Duration::from_millis(300)) else {
            panic!("the lock was taken while another descriptor held it");
        };
        assert_eq!(error.code, ErrorCode::Busy);
        drop(holder);
        assert!(RetireLock::take_within(dir.path(), Duration::from_millis(300)).is_ok());
    }
}
