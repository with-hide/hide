//! The one-time move of this Mac's state folder from `~/.local/state/hide`
//! to `~/.hide/state` (PRD hide-home-layout D-05).
//!
//! `hide connect` runs it before it looks for a daemon, and only for the
//! default state folder: a folder HIDE_STATE_DIR or XDG_STATE_HOME chose is
//! never moved (D-04). The whole folder is renamed at once, so there is never
//! a copy in two places or a half-moved folder; a daemon running from the old
//! folder is stopped first, because it keeps writing there, and the folder
//! moves only once its instance lock is free: a daemon that holds it but
//! could not be proven to be the one its state names is never signalled, and
//! nothing moves. When the new folder already exists nothing is merged: the
//! old folder stays as it is and the daemon's boot logs both paths
//! (`log_left_behind`). Every later run finds no old folder and does nothing.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use hide_platform::fs::lock::{self, Lock, Mode, Waited};

/// Process identity can disappear before its final descriptors close. Wait
/// only after stopping a proven daemon, and still require the actual lock.
const STOPPED_LOCK_WITHIN: Duration = Duration::from_secs(2);

/// What the move did.
#[derive(Debug, Eq, PartialEq)]
pub enum Moved {
    /// There was no old folder (or it was not a folder this account owns).
    Nothing,
    /// The old folder was renamed into place.
    Moved { stopped_pid: Option<u32> },
    /// Both folders exist; the new one is used and the old one stays.
    LeftBehind,
}

/// Moves `legacy` to `target` once. `stop` ends the daemon a legacy state
/// file names when that pid is proven to be the daemon, and returns its pid; a
/// daemon that will not stop, or any process still holding the folder's
/// instance lock afterwards, is an error and nothing moves.
pub fn move_legacy(
    legacy: &Path,
    target: &Path,
    stop: impl FnOnce(&Path) -> Result<Option<u32>, String>,
) -> Result<Moved, String> {
    if !own_folder(legacy)? {
        return Ok(Moved::Nothing);
    }
    // Checked before any lock is taken, so a legacy folder left beside the
    // new one is not touched at all (B5).
    if std::fs::symlink_metadata(target).is_ok() {
        return Ok(Moved::LeftBehind);
    }
    // Before anything is made or read in it.
    not_shared(legacy)?;
    // The legacy folder's own connect lock: an older `hide connect` takes the
    // same one, so neither starts a daemon there while it moves. The folder
    // itself is never made, so one another connect just moved is not made
    // again.
    let _lock = match lock_in_folder(&legacy.join("connect.lock")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Moved::Nothing),
        Err(error) => return Err(format!("{} could not be locked: {error}", legacy.display())),
    };
    // Rechecked under the lock: another connect may have moved it meanwhile.
    if !own_folder(legacy)? {
        return Ok(Moved::Nothing);
    }
    if std::fs::symlink_metadata(target).is_ok() {
        return Ok(Moved::LeftBehind);
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", target.display()))?;
    private_parent(parent)?;
    let stopped_pid = stop(legacy)?;
    // The lock is the liveness test: a pid or a `/health` answer can be stale
    // or slow, but a running daemon always holds it. Held across the rename,
    // so no older daemon starts in the folder meanwhile.
    let within = if stopped_pid.is_some() {
        STOPPED_LOCK_WITHIN
    } else {
        Duration::ZERO
    };
    let _instance = match instance_lock(legacy, within) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            if let Some(pid) = stopped_pid {
                return Err(format!(
                    "{} still holds its instance lock after daemon {pid} was stopped and {}s elapsed, so nothing was moved; connect again after the lock is released",
                    legacy.display(),
                    STOPPED_LOCK_WITHIN.as_secs()
                ));
            }
            return Err(format!(
                "a hided still runs from {} and was not proven to be the one its state names, so nothing was moved; quit it and connect again",
                legacy.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "{} could not be locked: {error}",
                crate::state_file::lock_path(legacy).display()
            ));
        }
    };
    // The folder keeps its mode through the rename and holds the daemon's
    // token, so it is made as private as a new one first; a failure here
    // leaves nothing moved and the next connect tries again.
    std::fs::set_permissions(legacy, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("{} could not be made private: {error}", legacy.display()))?;
    std::fs::rename(legacy, target).map_err(|error| {
        format!(
            "{} could not be moved to {}: {error}",
            legacy.display(),
            target.display()
        )
    })?;
    Ok(Moved::Moved { stopped_pid })
}

/// Logged once per daemon start while an old folder is left beside the one
/// in use (B5); nothing reaches the screen.
pub fn log_left_behind(legacy: Option<&Path>, state_dir: &Path) {
    let Some(legacy) = legacy else { return };
    if std::fs::symlink_metadata(legacy).is_ok() {
        hide_core::diagnostic!(serde_json::json!({
            "component": "hided",
            "kind": "state.legacy_left",
            "legacy": legacy.display().to_string(),
            "state_dir": state_dir.display().to_string(),
            "message": "both state folders exist; the new one is used and the old one is left untouched",
        }));
    }
}

/// Whether `path` is a real folder this account owns. A link or another
/// account's folder is not moved, whatever it leads to.
fn own_folder(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && metadata.uid() == euid()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{} could not be read: {error}", path.display())),
    }
}

/// Refuses a legacy folder another account can write to before anything in
/// it is read: its state file names the pid that `stop` signals.
fn not_shared(legacy: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(legacy)
        .map_err(|error| format!("{} could not be read: {error}", legacy.display()))?;
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} can be written by other accounts, so Hide does not move it; run `chmod go-w {}` and connect again",
            legacy.display(),
            legacy.display()
        ));
    }
    Ok(())
}

/// `~/.hide`, made 0700 when missing; an existing one must be a real folder
/// this account owns that no other account can write to, since the state
/// folder holds the daemon's token.
fn private_parent(parent: &Path) -> Result<(), String> {
    match std::fs::DirBuilder::new().mode(0o700).create(parent) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!(
                "{} could not be created: {error}",
                parent.display()
            ));
        }
    }
    let metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("{} could not be read: {error}", parent.display()))?;
    if !metadata.is_dir() || metadata.uid() != euid() || metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "{} is not a private folder of this account, so the state folder is not moved into it",
            parent.display()
        ));
    }
    Ok(())
}

/// Locks `path`, making the file inside its existing folder but never the
/// folder: a missing folder is `NotFound`.
fn lock_in_folder(path: &Path) -> io::Result<Lock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    lock::lock_file_blocking(file, Mode::Exclusive)
}

/// The legacy folder's instance lock, held through the rename. A missing
/// lock file is created inside the folder, never a missing folder itself.
fn instance_lock(legacy: &Path, within: Duration) -> io::Result<Lock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(crate::state_file::lock_path(legacy))?;
    match lock::lock_file(file, Mode::Exclusive, within, &|| false)? {
        Waited::Locked(lock) => Ok(lock),
        Waited::TimedOut | Waited::Cancelled => Err(io::ErrorKind::WouldBlock.into()),
    }
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_with(home: &Path) -> std::path::PathBuf {
        let legacy = home.join(".local/state/hide");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("core-state.json"), "{\"projects\":[1]}").unwrap();
        legacy
    }

    #[test]
    fn the_whole_folder_moves_once_and_a_second_run_does_nothing() {
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        let moved = move_legacy(&legacy, &target, |_| Ok(Some(42))).unwrap();
        assert_eq!(
            moved,
            Moved::Moved {
                stopped_pid: Some(42)
            }
        );
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("core-state.json")).unwrap(),
            "{\"projects\":[1]}"
        );
        let mode = std::fs::metadata(home.path().join(".hide")).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let mode = std::fs::metadata(&target).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700, "the moved folder is made private");
        let again = move_legacy(&legacy, &target, |_| panic!("nothing to stop")).unwrap();
        assert_eq!(again, Moved::Nothing);
        assert!(
            !legacy.exists(),
            "a second run must not make the old folder again"
        );
    }

    #[test]
    fn an_existing_new_folder_is_used_and_the_old_one_left_whole() {
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        std::fs::create_dir_all(&target).unwrap();
        let moved = move_legacy(&legacy, &target, |_| panic!("no daemon is stopped")).unwrap();
        assert_eq!(moved, Moved::LeftBehind);
        assert!(legacy.join("core-state.json").is_file());
        assert!(
            !legacy.join("connect.lock").exists(),
            "the old folder is not touched"
        );
        assert!(!target.join("core-state.json").exists());
    }

    #[test]
    fn a_process_still_holding_the_instance_lock_leaves_everything_in_place() {
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        // A daemon not proven to be the one its state names: `stop` finds
        // nothing to signal.
        let held = crate::state_file::acquire_lock(&legacy).unwrap();
        let error = move_legacy(&legacy, &target, |_| Ok(None)).unwrap_err();
        assert!(error.contains("still runs"), "{error}");
        assert!(legacy.join("core-state.json").is_file());
        assert!(!target.exists());
        drop(held);
        assert_eq!(
            move_legacy(&legacy, &target, |_| Ok(None)).unwrap(),
            Moved::Moved { stopped_pid: None }
        );
    }

    #[test]
    fn a_stopped_daemons_folder_waits_for_its_instance_lock_before_moving() {
        use std::sync::mpsc;
        use std::time::Duration;

        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        let held = crate::state_file::acquire_lock(&legacy).unwrap();
        let (stopped, stopping) = mpsc::channel();
        let (finished, outcome) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let result = move_legacy(&legacy, &target, |_| {
                    stopped.send(()).unwrap();
                    Ok(Some(42))
                });
                finished.send(result).unwrap();
            });
            stopping.recv_timeout(Duration::from_secs(2)).unwrap();
            // Process identity can disappear before the kernel closes its
            // descriptors. The real lock, not the pid, permits the rename.
            let before_release = outcome.recv_timeout(Duration::from_millis(50));
            assert!(legacy.join("core-state.json").is_file());
            assert!(!target.exists());
            drop(held);
            assert!(matches!(
                before_release,
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            assert_eq!(
                outcome
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .unwrap(),
                Moved::Moved {
                    stopped_pid: Some(42)
                }
            );
            assert!(!legacy.exists());
            assert_eq!(
                std::fs::read_to_string(target.join("core-state.json")).unwrap(),
                "{\"projects\":[1]}"
            );
        });
    }

    #[test]
    fn a_stopped_daemons_lock_that_never_releases_leaves_the_folder_unmoved() {
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        let _held = crate::state_file::acquire_lock(&legacy).unwrap();
        let error = move_legacy(&legacy, &target, |_| Ok(Some(42))).unwrap_err();
        assert!(error.contains("after daemon 42 was stopped"), "{error}");
        assert!(legacy.join("core-state.json").is_file());
        assert!(!target.exists());
    }

    #[test]
    fn a_folder_other_accounts_can_write_is_not_moved() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o777)).unwrap();
        let target = home.path().join(".hide/state");
        let error = move_legacy(&legacy, &target, |_| panic!("nothing is signalled")).unwrap_err();
        assert!(error.contains("other accounts"), "{error}");
        assert!(error.contains("chmod go-w"), "{error}");
        assert!(
            !legacy.join("connect.lock").exists(),
            "nothing is made in it"
        );
        assert!(legacy.join("core-state.json").is_file());
        // Beside an existing new folder it is only left, as any old folder is.
        std::fs::create_dir_all(&target).unwrap();
        assert_eq!(
            move_legacy(&legacy, &target, |_| panic!("nothing is signalled")).unwrap(),
            Moved::LeftBehind
        );
    }

    #[test]
    fn a_daemon_that_will_not_stop_leaves_everything_in_place() {
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let target = home.path().join(".hide/state");
        let error =
            move_legacy(&legacy, &target, |_| Err("pid 7 did not stop".to_owned())).unwrap_err();
        assert!(error.contains("pid 7"));
        assert!(legacy.join("core-state.json").is_file());
        assert!(!target.exists());
    }

    #[test]
    fn a_linked_old_folder_is_never_moved() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = home.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(home.path().join(".local/state")).unwrap();
        let legacy = home.path().join(".local/state/hide");
        std::os::unix::fs::symlink(&elsewhere, &legacy).unwrap();
        let target = home.path().join(".hide/state");
        assert_eq!(
            move_legacy(&legacy, &target, |_| panic!("nothing to stop")).unwrap(),
            Moved::Nothing
        );
        assert!(elsewhere.exists() && !target.exists());
    }

    #[test]
    fn a_hide_folder_others_can_write_is_not_moved_into() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let legacy = legacy_with(home.path());
        let hide = home.path().join(".hide");
        std::fs::create_dir_all(&hide).unwrap();
        std::fs::set_permissions(&hide, std::fs::Permissions::from_mode(0o777)).unwrap();
        let error =
            move_legacy(&legacy, &hide.join("state"), |_| panic!("not reached")).unwrap_err();
        assert!(error.contains("not a private folder"), "{error}");
        assert!(legacy.join("core-state.json").is_file());
    }
}
