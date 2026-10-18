//! Saving an editor document without ever discarding a change someone else
//! made, and without ever leaving the original truncated.
//!
//! This is not a compare-and-swap: no filesystem call compares content and
//! replaces a file in one step. It is exchange-and-verify:
//!
//! 1. the file is checked (a regular file, not a symlink, one link, owned by
//!    this user, writable) and its bytes are hashed against the revision the
//!    caller read; a different hash is a conflict and nothing is written;
//! 2. the new bytes go to a temporary file in the same opened folder, with the
//!    original's permissions, and are synced;
//! 3. the temporary file and the original are exchanged by the platform layer
//!    (`hide_platform::fs::atomic::exchange`: one atomic call on macOS and
//!    Linux, a replace that keeps the old file under another name on
//!    Windows), so the path never names a truncated file. On macOS and Linux
//!    it names the old file or the new one at every moment; on Windows
//!    `ReplaceFileW` moves the old file aside before the new one takes its
//!    name, so for that moment the path names nothing, and a reader there is
//!    told the file does not exist;
//! 4. the file that was displaced is hashed again. If it is not the revision
//!    the caller read, someone changed it between step 1 and step 3: the two
//!    are exchanged back, their version stays at the path, and the save is a
//!    conflict. Only when it matches is the displaced file removed.
//!
//! So every change that was complete at the path before the exchange
//! survives, and the operator's draft survives in the caller whatever
//! happens. What this cannot see is a writer that already holds the old file
//! open and writes to it after step 4; a later save of that writer's content
//! will then be judged against the new revision like any other edit.
//! A filesystem that cannot exchange at all is refused before anything is
//! written (PRD S5.5 B13-B15).
//! A process that dies between steps 3 and 4 leaves the displaced original
//! beside the file under its `.hide-save-` name, so it is kept, not lost.
//!
//! Saves and revision reads in one folder hold an advisory lock on that
//! folder, exclusive for a save and shared for a read. Only Hide takes it, so
//! it orders Hide's own work: a revision read that settles a save whose
//! answer was lost (B14) waits for that save to finish, even when the save
//! runs in a helper process whose connection already ended.
//!
//! A file whose bytes already equal the draft is saved whatever revision the
//! caller expected: nothing would change, and a save whose answer was lost
//! and then repeated as a new intent finds its own content there.
//!
//! A symbolic link inside the checkout is saved through: its target, resolved
//! inside the opened root, is the file that is checked and exchanged, and the
//! link stays a link. A link that leaves the root is refused by the handle.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;

use cap_std::fs::{Dir, OpenOptions};
use hide_platform::fs::lock::{self, Mode, Waited};
use hide_platform::fs::permissions::Permissions;
use hide_platform::fs::{atomic, identity, private};

use crate::document::{MAX_EDITABLE_BYTES, read_bounded, revision_of};
use crate::error::{ErrorCode, HostError, HostResult};
use crate::root::open_parent;
pub use hide_node_link::save::Saved;

/// The points a test can stop a save at to change the world underneath it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveStage {
    /// The original was checked and the temporary file is written; the
    /// exchange has not happened.
    BeforeExchange,
    /// The exchange displaced a file other than the expected one, and this
    /// save is about to swap it back.
    BeforeSwapBack,
}

/// Saves `contents` to `relative` under `dir` if the file there still holds
/// `expected_revision`.
pub fn save(
    dir: &Dir,
    relative: &Path,
    contents: &[u8],
    expected_revision: &str,
) -> HostResult<Saved> {
    save_observed(
        dir,
        relative,
        contents,
        expected_revision,
        &mut |_, _, _| {},
    )
}

#[doc(hidden)]
pub fn save_observed(
    dir: &Dir,
    relative: &Path,
    contents: &[u8],
    expected_revision: &str,
    observe: &mut dyn FnMut(SaveStage, &Dir, &OsStr),
) -> HostResult<Saved> {
    if contents.len() as u64 > MAX_EDITABLE_BYTES {
        return Err(HostError::new(
            ErrorCode::TooLarge,
            "The document is larger than the editable size; the draft was preserved",
        ));
    }
    let resolved = resolve_link(dir, relative)?;
    let (parent, name) = open_parent(dir, &resolved)?;
    let _lock = lock_folder(&parent, true)?;
    let original = inspect_target(&parent, &name)?;
    let current = read_revision(&parent, &name)?;
    if current == revision_of(contents) {
        return Ok(Saved { revision: current });
    }
    if current != expected_revision {
        return Err(HostError::conflict(
            Some(current),
            "The file changed on disk; choose Reload or Keep Editing",
        ));
    }

    let temporary = write_temporary(&parent, &name, contents, &original.permissions)?;
    observe(SaveStage::BeforeExchange, &parent, &name);
    if let Err(error) = atomic::exchange(&parent, &temporary, &name) {
        let _ = parent.remove_file(Path::new(&temporary));
        return Err(if error.kind() == io::ErrorKind::Unsupported {
            HostError::new(
                ErrorCode::Unsupported,
                "This filesystem cannot replace a file atomically, so the file was not saved; the draft was preserved",
            )
        } else {
            HostError::io(
                &error,
                "The file could not be saved; the draft was preserved",
            )
        });
    }

    // The displaced file now sits under the temporary name.
    let displaced = read_revision(&parent, &temporary);
    if displaced.as_deref().ok() != Some(expected_revision) {
        let actual = displaced.ok();
        observe(SaveStage::BeforeSwapBack, &parent, &name);
        return match atomic::exchange(&parent, &temporary, &name) {
            Ok(()) => {
                // The swap back normally leaves this save's own bytes under
                // the temporary name. If a third writer replaced the path in
                // between, the temporary now holds that writer's file, which
                // is kept rather than removed.
                if read_revision(&parent, &temporary).as_deref().ok()
                    == Some(revision_of(contents).as_str())
                {
                    let _ = parent.remove_file(Path::new(&temporary));
                } else {
                    // The path holds the second writer's file again; its
                    // revision is what Keep Editing adopts.
                    return Err(HostError::conflict(
                        read_revision(&parent, &name).ok(),
                        format!(
                            "The file changed on disk twice while it was being saved; the other versions are kept, one beside it as {}, and the draft was preserved",
                            temporary.to_string_lossy()
                        ),
                    ));
                }
                Err(HostError::conflict(
                    actual,
                    "The file changed on disk while it was being saved; the other version was kept and the draft was preserved",
                ))
            }
            Err(error) => Err(HostError::conflict(
                read_revision(&parent, &name).ok(),
                format!(
                    "The file changed on disk while it was being saved and could not be put back ({error}); the other version is kept beside it as {}, and the draft was preserved",
                    temporary.to_string_lossy()
                ),
            )),
        };
    }
    parent.remove_file(Path::new(&temporary)).map_err(|error| {
        HostError::io(
            &error,
            format!(
                "The file was saved but its previous version could not be removed from {}",
                temporary.to_string_lossy()
            ),
        )
    })?;
    // A directory that cannot be flushed does not undo a save that landed;
    // the node's error stream records it.
    if let Err(error) = atomic::sync_dir(&parent) {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "save",
                "kind": "folder_flush_failed",
                "message": error.to_string(),
            })
        );
    }
    Ok(Saved {
        revision: revision_of(contents),
    })
}

struct Target {
    permissions: Permissions,
}

/// Opens the existing file at `name` to read it, without waiting on a
/// device that has no writer and without following a link swapped in since
/// it was inspected.
fn open_regular(parent: &Dir, name: &OsStr) -> io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW);
    }
    Ok(parent.open_with(Path::new(name), &options)?.into_std())
}

/// Refuses every target a replace-by-exchange could not save faithfully.
fn inspect_target(parent: &Dir, name: &OsStr) -> HostResult<Target> {
    let metadata = parent.symlink_metadata(Path::new(name)).map_err(|error| {
        HostError::io(
            &error,
            "The existing file could not be inspected; the draft was preserved",
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            "The file is a symbolic link, so saving would replace the link; open its target instead. The draft was preserved",
        ));
    }
    if !metadata.is_file() {
        return Err(HostError::new(
            ErrorCode::NotAFile,
            "The save target is no longer a regular file; the draft was preserved",
        ));
    }
    let inspect = |error: io::Error| {
        HostError::io(
            &error,
            "The existing file could not be inspected; the draft was preserved",
        )
    };
    let file = open_regular(parent, name).map_err(inspect)?;
    if identity::link_count(&file).map_err(inspect)? > 1 {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            "The file has other hard links, which a save would separate from it; the draft was preserved",
        ));
    }
    if !private::handle_owned_by_current_user(&file).map_err(inspect)? {
        return Err(HostError::new(
            ErrorCode::PermissionDenied,
            "The file belongs to another user, and saving would change its owner; the draft was preserved",
        ));
    }
    let permissions = Permissions::of(&file).map_err(inspect)?;
    if !permissions.owner_can_write() {
        return Err(HostError::new(
            ErrorCode::PermissionDenied,
            "The file is read-only on disk; the draft was preserved",
        ));
    }
    Ok(Target { permissions })
}

fn read_revision(parent: &Dir, name: &OsStr) -> HostResult<String> {
    let mut file = open_regular(parent, name).map_err(|error| {
        HostError::io(
            &error,
            "The existing file could not be read; the draft was preserved",
        )
    })?;
    let bytes = read_bounded(&mut file)?.ok_or_else(|| {
        HostError::new(
            ErrorCode::TooLarge,
            "The file grew beyond the editable size; the draft was preserved",
        )
    })?;
    Ok(revision_of(&bytes))
}

fn write_temporary(
    parent: &Dir,
    name: &OsStr,
    contents: &[u8],
    permissions: &Permissions,
) -> HostResult<OsString> {
    let mut last_error = None;
    for attempt in 0..8u32 {
        let mut temporary = OsString::from(".");
        temporary.push(name);
        temporary.push(format!(".hide-save-{}-{attempt}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if let Some(mode) = permissions.unix_mode() {
            use cap_std::fs::OpenOptionsExt;
            // Made with the original's bits so it is never wider than it,
            // even for the moment before they are applied exactly.
            options.mode(mode);
        }
        match parent.open_with(Path::new(&temporary), &options) {
            Ok(file) => {
                let mut file = file.into_std();
                let written = (|| {
                    // create applies the umask; the saved file keeps the
                    // original's exact permissions.
                    permissions.apply(&file)?;
                    file.write_all(contents)?;
                    file.sync_all()
                })();
                if let Err(error) = written {
                    let _ = parent.remove_file(Path::new(&temporary));
                    return Err(HostError::io(
                        &error,
                        "The file could not be saved; the draft was preserved",
                    ));
                }
                return Ok(temporary);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => last_error = Some(error),
            Err(error) => {
                return Err(HostError::io(
                    &error,
                    "The folder is not writable, so the file could not be saved safely; the draft was preserved",
                ));
            }
        }
    }
    Err(HostError::io(
        &last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::AlreadyExists)),
        "A temporary save file could not be created; the draft was preserved",
    ))
}

/// The content revision of `relative` now, read the same way a save reads it
/// and after any save in progress in its folder has finished.
pub fn current_revision(dir: &Dir, relative: &Path) -> HostResult<String> {
    let resolved = resolve_link(dir, relative)?;
    let (parent, name) = open_parent(dir, &resolved)?;
    let _lock = lock_folder(&parent, false)?;
    read_revision(&parent, &name)
}

/// `relative` itself, or when it names a symbolic link, the file the link
/// resolves to inside `dir`. `Dir::canonicalize` answers a path relative to
/// the handle and refuses one that leaves it.
fn resolve_link(dir: &Dir, relative: &Path) -> HostResult<std::path::PathBuf> {
    let is_link = dir
        .symlink_metadata(relative)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if !is_link {
        return Ok(relative.to_path_buf());
    }
    dir.canonicalize(relative).map_err(|error| {
        HostError::io(
            &error,
            "The link's target could not be resolved inside the checkout; the draft was preserved",
        )
    })
}

/// How long a save waits for another of Hide's saves in the same folder
/// before it answers busy instead of queueing forever.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// The folder's advisory lock, released when dropped (or when the process
/// holding it ends). `parent` is a handle `cap-std` opened, which the
/// platform layer locks through a descriptor the system accepts.
fn lock_folder(folder: &Dir, exclusive: bool) -> HostResult<lock::Lock> {
    let mode = if exclusive {
        Mode::Exclusive
    } else {
        Mode::Shared
    };
    match lock::lock_dir(folder, mode, LOCK_WAIT, &|| false) {
        Ok(Waited::Locked(held)) => Ok(held),
        Ok(Waited::TimedOut | Waited::Cancelled) => Err(HostError::new(
            ErrorCode::Busy,
            "Another save in this folder has not finished; the draft was preserved",
        )),
        Err(error) => Err(HostError::io(
            &error,
            "The folder could not be locked for the save; the draft was preserved",
        )),
    }
}
