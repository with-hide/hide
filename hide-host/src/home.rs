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
//! `~/.hide-home-retiring`, and the marker and that folder are removed there.
//! The rename is the durable record that the retirement is past turning back:
//! a stop after it is finished by the next run, which then finds no `~/hide`,
//! while a `~/hide` with no marker is never touched, empty or not. A failure
//! at any step is returned, with the marker still on disk or the folder set
//! aside, so the next run retries it.
//!
//! Two retirements of one account's folder run one after the other: each
//! holds an exclusive lock on the account's home folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_platform::fs::lock::{self, Mode, Waited};
use hide_platform::fs::{identity, link};
use hide_platform::path::to_wire_lossy;
use serde::Deserialize;

use crate::error::{ErrorCode, HostError, HostResult};
pub use hide_node_link::home::HomeRetired;

/// The old Home folder's name inside the account's home directory.
pub const HOME_DIR_NAME: &str = "hide";
/// Present in a folder Hide made: which links are Hide's.
pub const MARKER_FILE: &str = ".hide-home.json";
const AGENTS_FILE: &str = "AGENTS.md";
const CLAUDE_FILE: &str = "CLAUDE.md";
const MARKER_VERSION: u32 = 1;
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
/// removed: the progress record that lets a stopped retirement finish.
const RETIRING_DIR: &str = ".hide-home-retiring";
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
/// it, for a caller that recorded a pane there. Nothing is removed when the
/// marker cannot be read.
pub fn retire(user_home: &Path, keep_empty: bool) -> HostResult<HomeRetired> {
    let _lock = RetireLock::take(user_home)?;
    finish_leftover(user_home)?;
    let home_path = user_home.join(HOME_DIR_NAME);
    let mut retired = HomeRetired {
        home: to_wire_lossy(&home_path),
        kept: false,
        marked: false,
        removed: Vec::new(),
        changed: Vec::new(),
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

    let Some(marker) = read_marker(&home_path)? else {
        retired.kept = true;
        return Ok(retired);
    };
    retired.marked = true;

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
        remove_file(&home_path, AGENTS_FILE, &mut retired)?;
    }
    if is_hide_claude_file(&home_path.join(CLAUDE_FILE))? {
        remove_file(&home_path, CLAUDE_FILE, &mut retired)?;
    }

    // Only the marker left, and nothing keeps the folder: it is renamed away
    // in one step, and the rename is the record that the retirement is past
    // the point of turning back, so a stop after it is finished by the next
    // run (`finish_leftover`) and the folder reads as gone.
    let removable = !keep_empty
        && only_the_marker_is_left(&home_path)?
        && !recorded_session(user_home, &home_path)?;
    if !removable {
        // The marker goes last, so a retirement that stops before here is
        // found again by it.
        remove_file(&home_path, MARKER_FILE, &mut retired)?;
        retired.kept = true;
        return Ok(retired);
    }
    let retiring = user_home.join(RETIRING_DIR);
    std::fs::rename(&home_path, &retiring)
        .map_err(|error| io_error(&error, "~/hide could not be set aside"))?;
    finish_leftover(user_home)?;
    retired.removed.push(MARKER_FILE.to_owned());
    Ok(retired)
}

/// Finishes a retirement that stopped after setting the folder aside: the
/// folder holds only the marker, which goes, and then the folder.
fn finish_leftover(user_home: &Path) -> HostResult<()> {
    let retiring = user_home.join(RETIRING_DIR);
    match std::fs::symlink_metadata(&retiring) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(&error, "The folder set aside could not be read")),
        Ok(metadata) if !metadata.is_dir() => {
            return Err(HostError::new(
                ErrorCode::Io,
                "The entry set aside for ~/hide's retirement is not a folder; it is left as it is",
            ));
        }
        Ok(_) => {}
    }
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
    // A folder that holds anything else is not what was set aside; it stays.
    std::fs::remove_dir(&retiring)
        .map_err(|error| io_error(&error, "The folder set aside could not be removed"))
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
/// the marker's links, once, with its recorded target; on Windows, where the
/// marker holds none, any `- name -> folder` lines), and its tail. A file the
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
    let mut seen = BTreeMap::new();
    for line in body.split_inclusive('\n') {
        let Some((name, target)) = line
            .strip_prefix("- ")
            .and_then(|line| line.strip_suffix('\n'))
            .and_then(|line| line.split_once(" -> "))
        else {
            return Ok(false);
        };
        if name.is_empty() || target.is_empty() || seen.insert(name, target).is_some() {
            return Ok(false);
        }
    }
    if seen.is_empty() {
        return Ok(false);
    }
    Ok(!links_intro
        || (seen.len() == marker.links.len()
            && seen
                .iter()
                .all(|(name, target)| marker.links.get(*name).is_some_and(|got| got == target))))
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
        let retired = retire(dir.path(), false).unwrap();
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
        let retired = retire(dir.path(), false).unwrap();
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
        let retired = retire(dir.path(), false).unwrap();
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
        let retired = retire(dir.path(), false).unwrap();
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
        let retired = retire(dir.path(), false).unwrap();
        assert!(retired.kept && !retired.marked && retired.removed.is_empty());
        assert_eq!(names(&home), [AGENTS_FILE]);
    }

    /// B11: a marker that cannot be read is an error and removes nothing.
    #[test]
    fn an_unreadable_marker_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join(MARKER_FILE), "{ not json").unwrap();
        assert!(retire(dir.path(), false).is_err());
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
        assert!(retire(dir.path(), false).is_err());
    }

    /// B10: a second run changes nothing and reports the same state.
    #[test]
    fn retiring_again_ends_in_the_same_state() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::write(home.join("notes.md"), "mine").unwrap();
        let first = retire(dir.path(), false).unwrap();
        let second = retire(dir.path(), false).unwrap();
        assert!(first.kept && second.kept);
        assert!(second.removed.is_empty() && !second.marked);
        assert_eq!(names(&home), ["notes.md"]);
        let gone = tempfile::tempdir().unwrap();
        old_home(gone.path(), &["alpha"]);
        retire(gone.path(), false).unwrap();
        let again = retire(gone.path(), false).unwrap();
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
        let retired = retire(dir.path(), false).unwrap();
        assert!(!retired.kept, "{retired:?}");
        assert!(!home.exists());
    }

    /// A pane recorded in the folder keeps it, empty as it is.
    #[test]
    fn an_empty_folder_stays_when_the_caller_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        let retired = retire(dir.path(), true).unwrap();
        assert!(retired.kept);
        assert!(names(&home).is_empty());
    }

    /// B6, D-03: a guide the operator touched in any way is theirs: a title
    /// only, a sentence added under the title, one added at the end, a project
    /// line the marker does not list, or the lines in another order.
    #[test]
    fn a_guide_the_operator_changed_in_any_way_is_kept() {
        let exact = |dir: &Path| guide_text(dir, &["alpha", "beta"]);
        let variants: [(&str, Box<dyn Fn(&Path) -> String>); 6] = [
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
            let retired = retire(dir.path(), false).unwrap();
            assert!(retired.kept, "{what}: {retired:?}");
            assert_eq!(names(&home), [AGENTS_FILE], "{what}");
        }
        // The same folder with the exact guide goes whole.
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha", "beta"]);
        assert!(!retire(dir.path(), false).unwrap().kept);
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
        let retiring = dir.path().join(RETIRING_DIR);
        std::fs::rename(&home, &retiring).unwrap();
        let retired = retire(dir.path(), false).unwrap();
        assert!(!retired.kept && !retired.marked, "{retired:?}");
        assert!(!retiring.exists() && !home.exists());

        let other = tempfile::tempdir().unwrap();
        let empty = other.path().join(HOME_DIR_NAME);
        std::fs::create_dir(&empty).unwrap();
        let retired = retire(other.path(), false).unwrap();
        assert!(retired.kept && !retired.marked);
        assert!(empty.is_dir());
    }

    /// B11: a folder that cannot be set aside is a failure with the marker
    /// still there, so a later run retires it.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_removed_is_a_failure_and_retries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = old_home(dir.path(), &["alpha"]);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = retire(dir.path(), false);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if std::fs::create_dir(dir.path().join("probe")).is_ok() && result.is_ok() {
            return; // running as a user the mode does not bind
        }
        assert!(result.is_err(), "{result:?}");
        assert!(home.join(MARKER_FILE).exists());
        let retired = retire(dir.path(), false).unwrap();
        assert!(!retired.kept && !home.exists());
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
