//! Pane holder version folders (PRD engine-bundle-2-pane-holder D-16).
//!
//! A holder outlives the node that started it and the app it came from, so it
//! never runs from the app: each build's holder (and on Windows the pinned
//! ConPTY runtime beside it, `conpty/`) is copied once into its own folder
//! under `~/.hide/holders`, named for its protocol version and a hash of its
//! content, so two builds never share one and a folder's name says what is
//! in it. New panes start from the newest folder; a pane started from an
//! older one keeps running from it.
//!
//! Three things happen to a folder and never at the same time, by one lock
//! file in the root: [`prepare`] makes or finds a folder and holds the lock
//! shared ([`Reservation`]) until the caller's holder runs from it, so a
//! folder that is about to be used counts as used; [`prune`] holds it
//! exclusively while it removes folders. A folder is removed only once it is
//! claimed by a rename and then found unused: on Windows no file in it may
//! be held by a process (a holder, an OpenConsole the ConPTY runtime started,
//! or a holder too old for this build to speak to), and on macOS and Linux
//! no process may run its holder. A claimed folder found in use is given its
//! name back.
//!
//! The runtime copies (`crate::runtime`) live under `~/.hide/runtime`, a
//! different root with its own lock, so neither cleanup touches the other's.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::lock::{Lock, Mode, Waited, lock_file};
use sha2::{Digest, Sha256};

use crate::layout;

/// The file a finished folder holds, naming the folder. A folder without it
/// was interrupted while it was made, and is never run.
pub const MARKER: &str = ".hide-holder";

/// The holder program's name in a folder.
pub fn program_name() -> String {
    format!("hide-holder{}", std::env::consts::EXE_SUFFIX)
}

/// The ConPTY runtime's folder beside the holder (Windows).
pub const CONPTY: &str = "conpty";

/// The lock file in the root.
const LOCK: &str = ".lock";

/// How long a prepare waits for a prune, and a prune for the prepares.
const LOCK_WAIT: Duration = Duration::from_secs(30);

/// How many hex digits of the content hash name a folder.
const HASH_PREFIX: usize = 16;

/// How many folders may remain after a prune before it is reported.
pub const FOLDERS_CAP: usize = 8;

/// What a version folder is made from: the holder program and, on Windows,
/// the pinned ConPTY runtime folder that goes beside it.
#[derive(Clone, Debug)]
pub struct Source {
    pub program: PathBuf,
    pub conpty: Option<PathBuf>,
}

/// The files a folder holds, each by the name it has in the folder and where
/// it comes from, in name order.
fn files(source: &Source) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = vec![(program_name(), source.program.clone())];
    if let Some(conpty) = &source.conpty {
        let mut found = Vec::new();
        collect(conpty, CONPTY, &mut found)?;
        files.extend(found);
    }
    files.sort();
    Ok(files)
}

fn collect(dir: &Path, name: &str, found: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let child = format!("{name}/{}", entry.file_name().to_string_lossy());
        if kind.is_dir() {
            collect(&entry.path(), &child, found)?;
        } else if kind.is_file() {
            found.push((child, entry.path()));
        } else {
            return Err(io::Error::other(format!(
                "{} is not a file or a folder",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// The folder name for `source` at protocol `version`: `v<version>-<hash>`,
/// the hash taken over every file's name and bytes.
pub fn version_name(version: u16, source: &Source) -> io::Result<String> {
    let mut digest = Sha256::new();
    for (name, path) in files(source)? {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        let bytes = fs::read(&path)?;
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    let hash: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("v{version}-{}", &hash[..HASH_PREFIX]))
}

/// A folder made or found by [`prepare`], held as used until dropped: drop
/// it once the holder started from it runs.
#[derive(Debug)]
pub struct Reservation {
    pub dir: PathBuf,
    pub program: PathBuf,
    _lock: Lock,
}

/// Makes or finds the version folder of `source` under `home` and reserves
/// it. A folder is made in a staging folder of its own and moved to its name
/// whole, so a folder by that name is complete or absent.
pub fn prepare(home: &Path, version: u16, source: &Source) -> Result<Reservation, String> {
    let root = layout::holders_root(home);
    hide_platform::fs::private::create_dir_all(&root)
        .map_err(|error| format!("{} could not be made: {error}", root.display()))?;
    let lock = lock(&root, Mode::Shared)?.ok_or_else(|| {
        format!(
            "a cleanup of {} held it for {} s",
            root.display(),
            LOCK_WAIT.as_secs()
        )
    })?;
    let name = version_name(version, source)
        .map_err(|error| format!("the holder's files could not be read: {error}"))?;
    let dir = layout::holder_dir(home, &name);
    if !is_finished(&dir, &name) {
        make(&root, &dir, &name, source)?;
    }
    Ok(Reservation {
        program: dir.join(program_name()),
        dir,
        _lock: lock,
    })
}

/// Whether `dir` is a finished folder named `name`: its marker names it and
/// its program is there.
pub fn is_finished(dir: &Path, name: &str) -> bool {
    fs::read_to_string(dir.join(MARKER)).is_ok_and(|named| named == name)
        && dir.join(program_name()).is_file()
}

fn make(root: &Path, dir: &Path, name: &str, source: &Source) -> Result<(), String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let staging = root.join(format!(
        ".staging-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let made = copy(source, &staging, name).and_then(|()| {
        hide_platform::fs::atomic::rename_no_replace_path(&staging, dir).or_else(|error| {
            // Another prepare made it first: theirs is the same content.
            if error.kind() == io::ErrorKind::AlreadyExists && is_finished(dir, name) {
                Ok(())
            } else {
                Err(error)
            }
        })
    });
    let _ = fs::remove_dir_all(&staging);
    made.map_err(|error| format!("{} could not be made: {error}", dir.display()))
}

fn copy(source: &Source, staging: &Path, name: &str) -> io::Result<()> {
    hide_platform::fs::private::create_dir(staging)?;
    for (relative, from) in files(source)? {
        let to = staging.join(&relative);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&from, &to)?;
        // A read-only file could never be asked whether a process holds it,
        // nor removed with its folder (Windows).
        let mut permissions = fs::metadata(&to)?.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)] // Windows' read-only attribute only
            permissions.set_readonly(false);
            fs::set_permissions(&to, permissions)?;
        }
    }
    fs::write(staging.join(MARKER), name)
}

/// Locks the root's lock file in `mode`, waiting up to [`LOCK_WAIT`];
/// `None` when the other side held it all that time.
fn lock(root: &Path, mode: Mode) -> Result<Option<Lock>, String> {
    let path = root.join(LOCK);
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{} could not be opened: {error}", path.display()))?;
    match lock_file(file, mode, LOCK_WAIT, &|| false) {
        Ok(Waited::Locked(lock)) => Ok(Some(lock)),
        Ok(_) => Ok(None),
        Err(error) => Err(format!("{} could not be locked: {error}", path.display())),
    }
}

/// What a prune did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    /// Folders nothing used, now gone.
    pub removed: Vec<String>,
    /// Folders a process still uses, kept.
    pub in_use: Vec<String>,
    /// Folders that could not be judged or removed, with the reason; each is
    /// kept, or left under its claimed name for the next prune.
    pub failed: Vec<(String, String)>,
    /// How many folders remain when that is more than [`FOLDERS_CAP`].
    pub over_cap: Option<usize>,
    /// Starts held the lock for the whole wait, so nothing was looked at.
    pub busy: bool,
}

/// Removes every folder under `home`'s holder root that is not named in
/// `keep` and that nothing uses. Waits for starts that hold folders reserved
/// and holds new ones off while it runs.
pub fn prune(home: &Path, keep: &[&str]) -> Pruned {
    let root = layout::holders_root(home);
    let mut pruned = Pruned::default();
    if !root.is_dir() {
        return pruned;
    }
    let _lock = match lock(&root, Mode::Exclusive) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            pruned.busy = true;
            return pruned;
        }
        Err(error) => {
            pruned.failed.push((LOCK.to_owned(), error));
            return pruned;
        }
    };
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) => {
            pruned
                .failed
                .push((root.display().to_string(), error.to_string()));
            return pruned;
        }
    };
    let mut remaining = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == LOCK || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if keep.contains(&name.as_str()) {
            remaining += 1;
            continue;
        }
        // A staging folder left by a prepare that died: no prepare runs
        // while this lock is held, and nothing runs from a staging folder.
        if name.starts_with(".staging-") {
            if let Err(error) = fs::remove_dir_all(entry.path()) {
                pruned.failed.push((name, error.to_string()));
            }
            continue;
        }
        // A folder an earlier prune claimed and could not finish is already
        // claimed; its original name is what follows the prefix.
        let (claimed, original) = if let Some(rest) = name.strip_prefix(".removing-") {
            (
                entry.path(),
                rest.rsplit_once('-')
                    .map_or(rest, |(name, _)| name)
                    .to_owned(),
            )
        } else {
            let claimed = root.join(format!(".removing-{name}-{}", std::process::id()));
            if let Err(error) = fs::rename(entry.path(), &claimed) {
                // Windows refuses to rename a folder a running program was
                // started from.
                match used(&entry.path()) {
                    Ok(true) => pruned.in_use.push(name),
                    _ => pruned
                        .failed
                        .push((name, format!("could not be claimed: {error}"))),
                }
                remaining += 1;
                continue;
            }
            (claimed, name.clone())
        };
        match used(&claimed) {
            Ok(false) => match fs::remove_dir_all(&claimed) {
                Ok(()) => pruned.removed.push(original),
                Err(error) => {
                    pruned.failed.push((original, error.to_string()));
                    remaining += 1;
                }
            },
            used => {
                // In use, or not known not to be: it keeps its name.
                let back = root.join(&original);
                let restored = fs::rename(&claimed, &back);
                match (used, restored) {
                    (Ok(true), Ok(())) => pruned.in_use.push(original),
                    (Ok(true), Err(error)) => pruned.failed.push((
                        original,
                        format!("in use, and its name could not be given back: {error}"),
                    )),
                    (Err(error), _) => pruned.failed.push((
                        original,
                        format!("whether it is used could not be told: {error}"),
                    )),
                    (Ok(false), _) => unreachable!("an unused folder is removed above"),
                }
                remaining += 1;
            }
        }
    }
    if remaining > FOLDERS_CAP {
        pruned.over_cap = Some(remaining);
    }
    pruned
}

/// Whether a process uses the folder `dir`.
fn used(dir: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    {
        // Every program and library started from it, the holder, an
        // OpenConsole and an older holder alike, holds its file.
        hide_platform::fs::in_use::first_in_use(dir).map(|found| found.is_some())
    }
    #[cfg(not(windows))]
    {
        // Only the holder runs from a folder on macOS and Linux; a process
        // runs the file whatever its path is now.
        let program = dir.join(program_name());
        let file = match hide_platform::fs::identity::file_id(&program) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        hide_platform::process::running(&file).map(|running| !running.is_empty())
    }
}
