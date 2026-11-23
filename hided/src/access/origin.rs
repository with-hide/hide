//! Kernel proof of a caller under a holder, including another machine's
//! holders on this account. No guest build's internal state is read.

use std::path::{Path, PathBuf};

use hide_platform::fs::identity::canonical;
use hide_platform::process;

pub(super) const UNAVAILABLE: &str = "caller_origin_unavailable";

/// Bounds the work of proving one live ancestry, as the platform's lineage
/// does. At the bound there is no complete proof, so the ask is refused;
/// no process or record is dropped. This runs only at bootstrap, off-lock.
const MAX_ANCESTORS: usize = 32;

fn directory(path: &Path) -> Result<(), &'static str> {
    if std::fs::metadata(path).map_err(|_| UNAVAILABLE)?.is_dir() {
        Ok(())
    } else {
        Err(UNAVAILABLE)
    }
}

fn root(path: &Path) -> Result<Option<PathBuf>, &'static str> {
    match canonical(path) {
        Ok(path) => {
            directory(&path)?;
            Ok(Some(path))
        }
        // No installation can run from a root that has never existed. A
        // running image whose root was removed fails its own canonical read.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Windows also reports PATH_NOT_FOUND when a prefix is a file.
            // Prove the nearest existing prefix is a directory; a broken
            // link, non-directory or unreadable prefix is not an empty root.
            for ancestor in path.ancestors() {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(_) => {
                        directory(ancestor)?;
                        return Ok(None);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(UNAVAILABLE),
                }
            }
            Err(UNAVAILABLE)
        }
        Err(_) => Err(UNAVAILABLE),
    }
}

/// Every Hide holder of this home counts as a pane. Guest holders use the
/// host-owned guest placement root, including development builds and work
/// retained after removal. Helper holders inherited at a split use the
/// home's ordinary holder root. These are placement boundaries, not a
/// parser for any file or folder inside a guest's node.
pub(super) fn in_holder(peer: i32, home: &Path, state: &Path) -> Result<bool, &'static str> {
    let mut pid = u32::try_from(peer)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or(UNAVAILABLE)?;
    let roots = [
        root(&hide_kit::layout::holders_root(home))?,
        root(&hide_kit::layout::guests(state))?,
    ];
    let mut started = process::start_time(pid).map_err(|_| UNAVAILABLE)?;
    for _ in 0..MAX_ANCESTORS {
        let same_account = process::same_account(pid).map_err(|_| UNAVAILABLE)?;
        if process::start_time(pid).map_err(|_| UNAVAILABLE)? != started {
            return Err(UNAVAILABLE);
        }
        // A root sshd above the account's shell is outside this account's
        // holders. Setuid intermediates retaining our real/saved identity
        // do not form this boundary. Unknown identity never permits an ask.
        if !same_account {
            return Ok(false);
        }
        let image = process::image_path(pid).map_err(|_| UNAVAILABLE)?;
        let image = canonical(&image).map_err(|_| UNAVAILABLE)?;
        if process::start_time(pid).map_err(|_| UNAVAILABLE)? != started {
            return Err(UNAVAILABLE);
        }
        if roots.iter().flatten().any(|root| image.starts_with(root)) {
            return Ok(true);
        }
        let parent = process::parent_of(pid);
        if process::start_time(pid).map_err(|_| UNAVAILABLE)? != started {
            return Err(UNAVAILABLE);
        }
        let parent = match parent {
            Ok(parent) => parent,
            // Windows retains a dead parent's old pid. No live ancestry
            // remains beyond it, as for the accepted detached-process case.
            #[cfg(windows)]
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            // Windows will not describe another account's process to this
            // one: the SSH service's own process, which runs as SYSTEM and
            // started this account's login. That is the account boundary a
            // root sshd is on Unix, once the system confirms the parent is
            // not this account's; a parent this account owns, or one that
            // cannot be read at all, stays refused.
            #[cfg(windows)]
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return match process::started_by_other_account(pid) {
                    Ok(true) => Ok(false),
                    Ok(false) | Err(_) => Err(UNAVAILABLE),
                };
            }
            Err(_) => return Err(UNAVAILABLE),
        };
        if parent <= 1 {
            return Ok(false);
        }
        let parent_started = process::start_time(parent).map_err(|_| UNAVAILABLE)?;
        if parent == pid || parent_started > started {
            return Err(UNAVAILABLE);
        }
        pid = parent;
        started = parent_started;
    }
    Err(UNAVAILABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_process_is_not_an_outside_terminal() {
        let root = tempfile::tempdir().unwrap();
        for pid in [-1, 0, i32::MAX] {
            assert_eq!(in_holder(pid, root.path(), root.path()), Err(UNAVAILABLE));
        }
    }

    #[test]
    fn unreadable_ownership_is_not_an_empty_guest_list() {
        let root = tempfile::tempdir().unwrap();
        // ENOTDIR, as opposed to an installation never having been made.
        let home = root.path().join("not-a-home");
        std::fs::write(&home, "broken ownership").unwrap();
        assert_eq!(
            in_holder(std::process::id() as i32, &home, root.path()),
            Err(UNAVAILABLE)
        );
        assert_eq!(super::root(&home), Err(UNAVAILABLE));
        assert_eq!(super::root(&root.path().join("never-installed")), Ok(None));
        #[cfg(unix)]
        {
            let broken = root.path().join("broken-link");
            std::os::unix::fs::symlink(root.path().join("gone"), &broken).unwrap();
            assert_eq!(super::root(&broken.join("holders")), Err(UNAVAILABLE));
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_proved_foreign_ancestor_needs_no_access_to_its_image() {
        // PID 1 is a real other-account process for an unprivileged test.
        // It exercises the boundary used at a normal SSH login's root
        // sshd, without starting a privileged server on the workstation.
        // SAFETY: getuid takes no arguments and cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        assert_eq!(in_holder(1, root.path(), root.path()), Ok(false));
    }
}
