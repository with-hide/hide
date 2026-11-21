//! A folder held open so a program started in it runs in that folder, not in
//! whatever its path names by the time the program starts.
//!
//! A caller that checked a folder by its open handle (under home, say) and
//! then starts a program there by path has a gap: the folder, or one above
//! it, can be swapped for a link between the check and the start, and the
//! program then writes wherever the link leads. A [`HeldFolder`] closes it
//! once, on every system:
//!
//! - On Unix the program's first act is `fchdir` to the held handle, so its
//!   working folder is the folder that was checked whatever the path names.
//! - On Windows a working folder is a path, so each folder from `base` down
//!   to the held one is held open without `FILE_SHARE_DELETE`: none of them
//!   can be renamed, removed or replaced by a junction while held, so the
//!   path keeps naming the folder that was checked.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Handle;

/// A folder held for programs started in it; see the module.
pub struct HeldFolder {
    path: PathBuf,
    sys: sys::Held,
}

impl HeldFolder {
    /// Holds `folder`, an open handle on the folder at `path`, which lies
    /// under `base`. On Windows each folder from `base` down to `path` is
    /// held too, and the folder the path names then has to be `folder`
    /// itself, or this is refused (`ErrorKind::Other` naming the path).
    pub fn new(folder: &impl Handle, path: PathBuf, base: &Path) -> io::Result<Self> {
        let sys = sys::Held::new(folder, &path, base)?;
        Ok(Self { path, sys })
    }

    /// The folder's path, as the caller named it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Makes `command` start in this folder. Any working folder set on it
    /// before is replaced.
    pub fn start_in(&self, command: &mut Command) {
        self.sys.start_in(command, &self.path);
    }
}

#[cfg(unix)]
mod sys {
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::Command;

    use super::Handle;

    pub(super) struct Held(OwnedFd);

    impl Held {
        pub(super) fn new(folder: &impl Handle, _path: &Path, _base: &Path) -> io::Result<Self> {
            Ok(Self(OwnedFd::from(crate::fs::duplicate(folder)?)))
        }

        pub(super) fn start_in(&self, command: &mut Command, path: &Path) {
            // The path is the folder's name for anything that reads the
            // command; `fchdir` below runs after it and wins.
            command.current_dir(path);
            let fd = self.0.as_raw_fd();
            // SAFETY: `fchdir` is async-signal-safe and the closure allocates
            // nothing. The descriptor is this process's and stays open until
            // `Held` drops, which outlives the spawn that borrows `self`; the
            // child's copy is open until its `exec` (close-on-exec).
            unsafe {
                command.pre_exec(move || {
                    if libc::fchdir(fd) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::File;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Component, Path};
    use std::process::Command;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    use super::Handle;
    use crate::fs::identity;

    /// Every folder from the base down, held without `FILE_SHARE_DELETE`.
    pub(super) struct Held(#[allow(dead_code)] Vec<File>);

    impl Held {
        pub(super) fn new(folder: &impl Handle, path: &Path, base: &Path) -> io::Result<Self> {
            let below = path.strip_prefix(base).map_err(|_| {
                io::Error::other(format!(
                    "{} is not under {}",
                    path.display(),
                    base.display()
                ))
            })?;
            let mut held = vec![hold(base)?];
            let mut at = base.to_path_buf();
            for component in below.components() {
                let Component::Normal(name) = component else {
                    return Err(io::Error::other(format!(
                        "{} is not a plain path under {}",
                        path.display(),
                        base.display()
                    )));
                };
                at.push(name);
                held.push(hold(&at)?);
            }
            // Held now, the path can no longer move; it has to name the
            // folder the caller checked.
            let last = held.last().expect("the base is held");
            if identity::file_id_of(last)? != identity::file_id_of(folder)? {
                return Err(io::Error::other(format!(
                    "{} no longer names the folder that was checked",
                    path.display()
                )));
            }
            Ok(Self(held))
        }

        pub(super) fn start_in(&self, command: &mut Command, path: &Path) {
            command.current_dir(path);
        }
    }

    /// A folder opened to list it, which takes part in sharing checks, and
    /// shared for reading and writing but not for deleting: a rename or a
    /// removal of it asks for delete access and is refused while this is open.
    fn hold(path: &Path) -> io::Result<File> {
        std::fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }
}
