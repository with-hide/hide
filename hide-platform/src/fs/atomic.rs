//! Replacing a file, and moving one, without exposing partially written bytes.
//!
//! The path of a file that is being replaced names either the old file or the
//! new one at every moment and never a truncated one, on every system.
//! What differs is what else the system gives: macOS and Linux can exchange
//! two names in one step and can rename without replacing, which a save that
//! checks what it displaced is built on; Windows has `ReplaceFileW`, which
//! replaces and keeps the old file under another name, and `MoveFileExW`,
//! which refuses to replace. [`EXCHANGE_IS_ATOMIC`] says which of the two a
//! build has.
//!
//! Atomic visibility and durable acknowledgement are separate contracts.
//! [`write_file_durable`] checks the operating system's persistence barrier;
//! the older writers retain their best-effort parent sync.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::identity::{self, FileId};
use super::permissions::Permissions;
use super::{Access, Handle};

/// Whether [`exchange`] swaps the two names in one step. Where it does not
/// (Windows), the name that is replaced still never goes missing or shows a
/// partial file, but the name that receives the old file is empty for a moment
/// in between, and a crash in that moment leaves the old file under a
/// `.hide-swap-` name beside it instead of lost.
pub const EXCHANGE_IS_ATOMIC: bool = cfg!(any(target_os = "macos", target_os = "linux"));

/// A failed removal of the writer's own temporary file, separate from the
/// original write failure. The named temporary is retained for recovery.
#[derive(Debug)]
pub struct TempCleanupError {
    pub path: PathBuf,
    pub source: io::Error,
}

impl std::fmt::Display for TempCleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "temporary cleanup failed at {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for TempCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The point at which a durable write failed. Neither uncertain outcome
/// means rollback: the caller must stop using an assumed old version until
/// it has validated the actual file and established durability again.
#[derive(Debug)]
pub enum DurableWriteError {
    /// Replacement was not attempted, so the destination was not changed.
    BeforeReplace {
        source: io::Error,
        cleanup: Option<TempCleanupError>,
    },
    /// The replacement operation failed. Some systems can change names
    /// before reporting failure, so the destination's version is uncertain.
    ReplacementUncertain {
        source: io::Error,
        cleanup: Option<TempCleanupError>,
    },
    /// Replacement succeeded but its parent barrier failed. The installed
    /// file has this identity; its crash persistence has not been confirmed.
    ReplacedNotDurable { file_id: FileId, source: io::Error },
}

impl DurableWriteError {
    fn before_replace(source: io::Error) -> Self {
        Self::BeforeReplace {
            source,
            cleanup: None,
        }
    }

    // The legacy API intentionally retains its original io::Error contract.
    fn into_io_error(self) -> io::Error {
        match self {
            Self::BeforeReplace { source, .. }
            | Self::ReplacementUncertain { source, .. }
            | Self::ReplacedNotDurable { source, .. } => source,
        }
    }
}

impl std::fmt::Display for DurableWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (phase, source, cleanup) = match self {
            Self::BeforeReplace { source, cleanup } => ("before replacement", source, cleanup),
            Self::ReplacementUncertain { source, cleanup } => {
                ("replacement uncertain", source, cleanup)
            }
            Self::ReplacedNotDurable { source, .. } => {
                return write!(formatter, "replaced but durability unconfirmed: {source}");
            }
        };
        write!(formatter, "{phase}: {source}")?;
        if let Some(cleanup) = cleanup {
            write!(formatter, "; {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DurableWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::BeforeReplace { source, .. }
            | Self::ReplacementUncertain { source, .. }
            | Self::ReplacedNotDurable { source, .. } => source,
        })
    }
}

/// Writes `contents` to `path` whole or not at all: the bytes go to a file
/// beside it, are flushed, and replace `path` in one step. Parent sync and
/// temporary cleanup are best effort. A replacement error does not promise
/// the old destination is unchanged on every system.
/// Returns the identity of the file now at `path`.
///
/// `path` itself is replaced, so a link there is replaced by a file; a caller
/// that means to write through a link resolves it first with
/// [`identity::canonical`].
pub fn write_file(path: &Path, contents: &[u8], access: Access) -> io::Result<FileId> {
    write(path, contents, access, WriteMode::Visibility).map_err(DurableWriteError::into_io_error)
}

/// Who may use a file [`write_file_durable`] writes: the current account
/// alone, as [`Access::Private`] and [`Access::PrivateExecutable`] say. A
/// durable write never keeps the access of the file it replaces: Windows
/// keeps an access list only through `ReplaceFileW`, which has no
/// write-through form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableAccess {
    Private,
    PrivateExecutable,
}

impl From<DurableAccess> for Access {
    fn from(access: DurableAccess) -> Self {
        match access {
            DurableAccess::Private => Access::Private,
            DurableAccess::PrivateExecutable => Access::PrivateExecutable,
        }
    }
}

/// Writes a whole file and returns its identity only after the operating
/// system acknowledges the persistence barrier: file sync, a same-folder
/// rename over the destination, then a fallible open and flush of the
/// folder ([`sync_dir`]). On Windows the rename is `MoveFileExW` with the
/// replace and write-through flags, the form the `atomicwrites` crate uses.
///
/// The parent must already exist, its ancestry must have an established
/// durability boundary, and the caller must keep that trusted parent stable.
/// This function neither creates nor syncs a chain of ancestors. An OS
/// acknowledgement is not proof against every storage device's power loss,
/// and sync calls have no promised wall-clock cancellation bound.
///
/// An uncertain error preserves the destination for validated recovery;
/// it never restores old bytes or deletes the destination. A temporary that
/// could not be removed is reported separately with its path and cause.
/// As with [`write_file`], a link at the destination is replaced, not followed.
pub fn write_file_durable(
    path: &Path,
    contents: &[u8],
    access: DurableAccess,
) -> Result<FileId, DurableWriteError> {
    write(path, contents, access.into(), WriteMode::Durable)
}

#[derive(Clone, Copy)]
enum WriteMode {
    Visibility,
    Durable,
}

fn write(
    path: &Path,
    contents: &[u8],
    access: Access,
    mode: WriteMode,
) -> Result<FileId, DurableWriteError> {
    let name = path
        .file_name()
        .ok_or_else(|| DurableWriteError::before_replace(io::ErrorKind::InvalidInput.into()))?;
    let folder = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let kept = match access {
        Access::KeepOrPrivate => match fs::File::open(path) {
            Ok(existing) => {
                Some(Permissions::of(&existing).map_err(DurableWriteError::before_replace)?)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(DurableWriteError::before_replace(error)),
        },
        Access::Private | Access::PrivateExecutable => None,
    };
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for _ in 0..8 {
        let temporary_path = folder.join(super::temporary_name(name, "hide"));
        let mut file = match sys::create_temporary(&temporary_path, access, kept.is_some()) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last = error;
                continue;
            }
            Err(error) => return Err(DurableWriteError::before_replace(error)),
        };
        let mut temporary = OwnedTemporary::new(temporary_path);
        let prepared: io::Result<FileId> = (|| {
            #[cfg(test)]
            if matches!(mode, WriteMode::Durable) {
                os_faults::fail(os_faults::Point::FileWrite)?;
            }
            file.write_all(contents)?;
            if let Some(kept) = &kept {
                kept.apply(&file)?;
            }
            #[cfg(test)]
            if matches!(mode, WriteMode::Durable) {
                os_faults::fail(os_faults::Point::FileSync)?;
            }
            file.sync_all()?;
            let id = identity::file_id_of(&file)?;
            drop(file);
            Ok(id)
        })();
        let id = prepared.map_err(|source| DurableWriteError::BeforeReplace {
            source,
            cleanup: temporary.cleanup(mode),
        })?;
        let replaced = match mode {
            WriteMode::Visibility => sys::replace(&temporary.path, path, kept.is_some()),
            WriteMode::Durable => sys::replace_durable(&temporary.path, path),
        };
        replaced.map_err(|source| DurableWriteError::ReplacementUncertain {
            source,
            cleanup: temporary.cleanup(mode),
        })?;
        temporary.disarm();
        match mode {
            WriteMode::Visibility => sync_path_parent(folder),
            WriteMode::Durable => {
                sync_durable_parent(folder).map_err(|source| {
                    DurableWriteError::ReplacedNotDurable {
                        file_id: id,
                        source,
                    }
                })?;
            }
        }
        return Ok(id);
    }
    Err(DurableWriteError::before_replace(last))
}

/// Owns only the exclusively created temporary, never the destination.
/// Normal failures report removal errors; Drop is the unwinding fallback.
struct OwnedTemporary {
    path: PathBuf,
    armed: bool,
}

impl OwnedTemporary {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cleanup(&mut self, mode: WriteMode) -> Option<TempCleanupError> {
        // Do not retry a failed removal in Drop: the reported residue stays
        // available for the caller's recovery and the original cause survives.
        self.disarm();
        #[cfg(not(test))]
        let _ = mode;
        #[cfg(test)]
        let fault = if matches!(mode, WriteMode::Durable) {
            os_faults::fail(os_faults::Point::Cleanup)
        } else {
            Ok(())
        };
        #[cfg(not(test))]
        let fault: io::Result<()> = Ok(());
        match fault.and_then(|()| fs::remove_file(&self.path)) {
            Ok(()) => None,
            // A failed replacement can already have moved the temporary.
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(source) => Some(TempCleanupError {
                path: self.path.clone(),
                source,
            }),
        }
    }
}

impl Drop for OwnedTemporary {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sync_durable_parent(folder: &Path) -> io::Result<()> {
    #[cfg(test)]
    os_faults::fail(os_faults::Point::ParentOpen)?;
    let folder = super::open_dir(folder)?;
    #[cfg(test)]
    os_faults::fail(os_faults::Point::ParentSync)?;
    sys::sync_dir(&folder)
}

/// Renames the file `from` over `to`, replacing it in one step. Returns the
/// identity of the file now at `to`, which is the one `from` had.
pub fn replace_file(from: &Path, to: &Path) -> io::Result<FileId> {
    // The entry itself: a link moves as a link, and its id is its own.
    let id = identity::file_id_nofollow(from)?;
    fs::rename(from, to)?;
    if let Some(folder) = to.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        sync_path_parent(folder);
    }
    Ok(id)
}

/// Makes an open folder's entries durable: `fsync` on Unix, and on Windows
/// `FlushFileBuffers` on the same folder opened again with the right to add
/// an entry, which a flush asks of its handle (etcd's `fileutil.OpenDir`
/// takes the same route). A Windows folder this account may add no entry to
/// cannot be flushed by it and is `PermissionDenied`, never a quiet `Ok`.
pub fn sync_dir(dir: &impl Handle) -> io::Result<()> {
    sys::sync_dir(dir)
}

fn sync_path_parent(folder: &Path) {
    if let Ok(folder) = fs::File::open(folder) {
        let _ = sys::sync_dir(&folder);
    }
}

/// Swaps two names in the open folder `dir`: afterwards `left` holds what
/// `right` held and the other way round. Both must exist. A filesystem
/// that cannot is `ErrorKind::Unsupported`, and nothing changed.
///
/// On Windows the swap is a replace followed by a rename and is not one step;
/// see [`EXCHANGE_IS_ATOMIC`].
pub fn exchange(dir: &impl Handle, left: &OsStr, right: &OsStr) -> io::Result<()> {
    sys::exchange(dir, left, right)
}

/// [`rename_no_replace`] for a caller that has two paths and no open folders:
/// a finished clone moved to its name, a staged folder moved to `~/hide`.
pub fn rename_no_replace_path(from: &Path, to: &Path) -> io::Result<()> {
    sys::rename_no_replace_path(from, to)
}

/// Renames `from` in `from_dir` to `to` in `to_dir`, and refuses when `to`
/// exists, even when it appears between the caller's look and this call
/// (`ErrorKind::AlreadyExists`). `ErrorKind::CrossesDevices` when the two
/// folders are on different volumes.
pub fn rename_no_replace(
    from_dir: &impl Handle,
    from: &OsStr,
    to_dir: &impl Handle,
    to: &OsStr,
) -> io::Result<()> {
    sys::rename_no_replace(from_dir, from, to_dir, to)
}

#[cfg(unix)]
mod sys {
    use std::ffi::{CString, OsStr};
    use std::fs;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use super::{Access, Handle};

    pub(super) fn create_temporary(
        path: &Path,
        access: Access,
        _inherit: bool,
    ) -> io::Result<fs::File> {
        let mode = match access {
            Access::PrivateExecutable => 0o700,
            Access::Private | Access::KeepOrPrivate => 0o600,
        };
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)
    }

    pub(super) fn replace(from: &Path, to: &Path, _keep_acl: bool) -> io::Result<()> {
        fs::rename(from, to)
    }

    pub(super) fn replace_durable(from: &Path, to: &Path) -> io::Result<()> {
        #[cfg(test)]
        super::os_faults::fail(super::os_faults::Point::Replacement)?;
        fs::rename(from, to)?;
        #[cfg(test)]
        super::os_faults::fail(super::os_faults::Point::ReplacementAcknowledgement)?;
        Ok(())
    }

    pub(super) fn sync_dir(dir: &impl Handle) -> io::Result<()> {
        // A folder handle that cannot be flushed (cap-std's `O_PATH` one) is
        // reopened readable first.
        let file = crate::fs::reopen_dir_for_io(dir)?;
        // The plain flush: `sync_all` is `F_FULLFSYNC` on macOS, which on a
        // folder costs milliseconds more than the `fsync` a save always paid.
        // SAFETY: the descriptor belongs to `file`, which outlives the call.
        if unsafe { libc::fsync(std::os::fd::AsRawFd::as_raw_fd(&file)) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn name(value: &OsStr) -> io::Result<CString> {
        CString::new(value.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name holds a NUL"))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn exchange(dir: &impl Handle, left: &OsStr, right: &OsStr) -> io::Result<()> {
        let (left, right) = (name(left)?, name(right)?);
        let fd = dir.as_fd().as_raw_fd();
        // SAFETY: both names are NUL-terminated and outlive the call, and the
        // descriptor is an open folder borrowed for it.
        let result =
            unsafe { libc::renameatx_np(fd, left.as_ptr(), fd, right.as_ptr(), libc::RENAME_SWAP) };
        finish_exchange(result)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn exchange(dir: &impl Handle, left: &OsStr, right: &OsStr) -> io::Result<()> {
        let (left, right) = (name(left)?, name(right)?);
        let fd = dir.as_fd().as_raw_fd();
        // SAFETY: as on macOS.
        let result = unsafe {
            libc::renameat2(fd, left.as_ptr(), fd, right.as_ptr(), libc::RENAME_EXCHANGE)
        };
        finish_exchange(result)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn exchange(_: &impl Handle, _: &OsStr, _: &OsStr) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// The codes a filesystem without an atomic exchange answers with, said
    /// as one kind so a caller does not read errno.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn finish_exchange(result: i32) -> io::Result<()> {
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // `ENOTSUP` and `EOPNOTSUPP` are one number on Linux and two on macOS.
        let unsupported = error.raw_os_error().is_some_and(|code| {
            [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL, libc::ENOSYS].contains(&code)
        });
        Err(if unsupported {
            io::Error::from(io::ErrorKind::Unsupported)
        } else {
            error
        })
    }

    #[cfg(target_os = "macos")]
    pub(super) fn rename_no_replace(
        from_dir: &impl Handle,
        from: &OsStr,
        to_dir: &impl Handle,
        to: &OsStr,
    ) -> io::Result<()> {
        let (from, to) = (name(from)?, name(to)?);
        // SAFETY: both names are NUL-terminated and outlive the call, and both
        // descriptors are open folders borrowed for it.
        let result = unsafe {
            libc::renameatx_np(
                from_dir.as_fd().as_raw_fd(),
                from.as_ptr(),
                to_dir.as_fd().as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        finish_rename(result)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn rename_no_replace(
        from_dir: &impl Handle,
        from: &OsStr,
        to_dir: &impl Handle,
        to: &OsStr,
    ) -> io::Result<()> {
        let (from, to) = (name(from)?, name(to)?);
        // SAFETY: as on macOS.
        let result = unsafe {
            libc::renameat2(
                from_dir.as_fd().as_raw_fd(),
                from.as_ptr(),
                to_dir.as_fd().as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        finish_rename(result)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn rename_no_replace(
        _: &impl Handle,
        _: &OsStr,
        _: &impl Handle,
        _: &OsStr,
    ) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn rename_no_replace_path(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (name(from.as_os_str())?, name(to.as_os_str())?);
        // SAFETY: both are NUL-terminated paths that outlive the call.
        finish_rename(unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) })
    }

    #[cfg(target_os = "linux")]
    pub(super) fn rename_no_replace_path(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (name(from.as_os_str())?, name(to.as_os_str())?);
        // SAFETY: both are NUL-terminated paths that outlive the call.
        finish_rename(unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) fn rename_no_replace_path(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn finish_rename(result: i32) -> io::Result<()> {
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsStr;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::ptr::null;

    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    };

    use super::{Access, Handle};
    use crate::fs::{path_of, wide};

    pub(super) fn create_temporary(
        path: &Path,
        access: Access,
        inherit: bool,
    ) -> io::Result<fs::File> {
        // A temporary that will replace a file through `ReplaceFileW` takes
        // that file's access list in the replacement, so it needs none of its
        // own; any other is private from its first moment.
        let _ = access;
        if inherit {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
        } else {
            crate::fs::private::create_new_file(path)
        }
    }

    pub(super) fn replace(from: &Path, to: &Path, keep_acl: bool) -> io::Result<()> {
        if keep_acl {
            return replace_file_w(to, from, None);
        }
        fs::rename(from, to)
    }

    pub(super) fn replace_durable(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (wide(from)?, wide(to)?);
        #[cfg(test)]
        super::os_faults::fail(super::os_faults::Point::Replacement)?;
        // SAFETY: both paths are NUL-terminated and outlive this call. The
        // temporary is beside the destination; copying across volumes is not
        // enabled. Its private access list moves with it.
        let done = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        #[cfg(test)]
        super::os_faults::fail(super::os_faults::Point::ReplacementAcknowledgement)?;
        Ok(())
    }

    pub(super) fn sync_dir(dir: &impl Handle) -> io::Result<()> {
        // A normal flush writes a folder's data and metadata and has the
        // storage flush its cache; only the data-sync flush refuses folders
        // (`NtFlushBuffersFileEx`).
        crate::fs::reopen_dir_for_flush(dir)?.sync_all()
    }

    /// `replacement` takes the name `replaced` had, with its attributes and
    /// access list, and `replaced`'s old file goes to `backup` when given.
    fn replace_file_w(
        replaced: &Path,
        replacement: &Path,
        backup: Option<&Path>,
    ) -> io::Result<()> {
        let (replaced, replacement) = (wide(replaced)?, wide(replacement)?);
        let backup = backup.map(wide).transpose()?;
        // SAFETY: every name is NUL-terminated and outlives the call; the two
        // reserved arguments are null as the call requires.
        let done = unsafe {
            ReplaceFileW(
                replaced.as_ptr(),
                replacement.as_ptr(),
                backup.as_ref().map_or(null(), |name| name.as_ptr()),
                0,
                null(),
                null(),
            )
        };
        if done == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// `ReplaceFileW` puts `left` under `right`'s name and `right`'s old file
    /// under a parked name, and a rename then moves that file to `left`'s.
    pub(super) fn exchange(dir: &impl Handle, left: &OsStr, right: &OsStr) -> io::Result<()> {
        let folder = path_of(dir)?;
        let parked = folder.join(crate::fs::temporary_name(right, "hide-swap"));
        replace_file_w(&folder.join(right), &folder.join(left), Some(&parked))?;
        let (from, to) = (wide(&parked)?, wide(&folder.join(left))?);
        // SAFETY: both names are NUL-terminated and outlive the call.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn rename_no_replace(
        from_dir: &impl Handle,
        from: &OsStr,
        to_dir: &impl Handle,
        to: &OsStr,
    ) -> io::Result<()> {
        rename_no_replace_path(&path_of(from_dir)?.join(from), &path_of(to_dir)?.join(to))
    }

    pub(super) fn rename_no_replace_path(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (wide(from)?, wide(to)?);
        // SAFETY: both names are NUL-terminated and outlive the call; with no
        // flags the call refuses an existing destination.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Test-only faults at OS I/O boundaries, scoped to the calling test thread.
/// Real files, permissions, identity and writer decisions remain in use.
#[cfg(test)]
mod os_faults {
    use std::cell::Cell;
    use std::io;

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Point {
        FileWrite,
        FileSync,
        Replacement,
        ReplacementAcknowledgement,
        Cleanup,
        ParentOpen,
        ParentSync,
    }

    thread_local! {
        static FAULTS: Cell<u8> = const { Cell::new(0) };
    }

    pub(super) struct Scope(u8);

    pub(super) fn install(points: &[Point]) -> Scope {
        let mask = points
            .iter()
            .fold(0, |mask, point| mask | (1 << *point as u32));
        Scope(FAULTS.with(|faults| faults.replace(mask)))
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            FAULTS.with(|faults| faults.set(self.0));
        }
    }

    #[cfg(unix)]
    pub(super) const IO_ERROR: i32 = libc::EIO;
    #[cfg(windows)]
    pub(super) const IO_ERROR: i32 = windows_sys::Win32::Foundation::ERROR_WRITE_FAULT as i32;
    #[cfg(unix)]
    const CLEANUP_ERROR: i32 = libc::EACCES;
    #[cfg(windows)]
    const CLEANUP_ERROR: i32 = windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32;

    pub(super) fn fail(point: Point) -> io::Result<()> {
        if FAULTS.with(|faults| faults.get() & (1 << point as u32) != 0) {
            Err(io::Error::from_raw_os_error(if point == Point::Cleanup {
                CLEANUP_ERROR
            } else {
                IO_ERROR
            }))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::os_faults::{self, Point};
    use super::*;

    fn names(folder: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_or_file_sync_failure_keeps_old_bytes_and_cleans_the_temporary() {
        for point in [Point::FileWrite, Point::FileSync] {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("ledger.json");
            fs::write(&path, b"old").unwrap();
            let old_id = identity::file_id(&path).unwrap();
            let _fault = os_faults::install(&[point]);

            let error = write_file_durable(&path, b"new", DurableAccess::Private).unwrap_err();

            match &error {
                DurableWriteError::BeforeReplace { source, cleanup } => {
                    assert_eq!(source.raw_os_error(), Some(os_faults::IO_ERROR));
                    assert!(cleanup.is_none());
                }
                other => panic!("expected failure before replacement: {other}"),
            }
            assert!(std::error::Error::source(&error).is_some());
            assert_eq!(fs::read(&path).unwrap(), b"old");
            assert_eq!(identity::file_id(&path).unwrap(), old_id);
            assert_eq!(names(folder.path()), [OsString::from("ledger.json")]);
        }
    }

    #[test]
    fn parent_open_or_sync_failure_preserves_the_installed_file_for_recovery() {
        for point in [Point::ParentOpen, Point::ParentSync] {
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("ledger.json");
            fs::write(&path, b"old").unwrap();
            let old_id = identity::file_id(&path).unwrap();
            let fault = os_faults::install(&[point]);

            let error =
                write_file_durable(&path, b"new intent", DurableAccess::Private).unwrap_err();

            match error {
                DurableWriteError::ReplacedNotDurable { file_id, source } => {
                    assert_eq!(source.raw_os_error(), Some(os_faults::IO_ERROR));
                    assert_eq!(file_id, identity::file_id(&path).unwrap());
                    assert_ne!(file_id, old_id);
                }
                other => panic!("expected installed but unconfirmed file: {other}"),
            }
            assert_eq!(fs::read(&path).unwrap(), b"new intent");
            assert!(super::super::private::is_private(&path).unwrap());
            assert_eq!(names(folder.path()), [OsString::from("ledger.json")]);

            // Explicit recovery uses the retained intent, after the fault is
            // gone. The writer itself never retries or restores old bytes.
            drop(fault);
            let retained = fs::read(&path).unwrap();
            let recovered = write_file_durable(&path, &retained, DurableAccess::Private).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"new intent");
            assert_eq!(identity::file_id(&path).unwrap(), recovered);
        }
    }

    #[test]
    fn replacement_failure_is_uncertain_even_when_old_bytes_are_still_present() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ledger.json");
        fs::write(&path, b"old").unwrap();
        let _fault = os_faults::install(&[Point::Replacement]);

        let error = write_file_durable(&path, b"new", DurableAccess::Private).unwrap_err();

        match error {
            DurableWriteError::ReplacementUncertain { source, cleanup } => {
                assert_eq!(source.raw_os_error(), Some(os_faults::IO_ERROR));
                assert!(cleanup.is_none());
            }
            other => panic!("expected uncertain replacement: {other}"),
        }
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(names(folder.path()), [OsString::from("ledger.json")]);
    }

    #[test]
    fn failed_replacement_acknowledgement_never_rolls_back_or_deletes_new_bytes() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ledger.json");
        fs::write(&path, b"old").unwrap();
        let old_id = identity::file_id(&path).unwrap();
        // The real replacement takes place; only its OS acknowledgement is
        // faulted, modeling a failed operation with a changed namespace.
        let _fault = os_faults::install(&[Point::ReplacementAcknowledgement]);

        let error = write_file_durable(&path, b"new intent", DurableAccess::Private).unwrap_err();

        match error {
            DurableWriteError::ReplacementUncertain { source, cleanup } => {
                assert_eq!(source.raw_os_error(), Some(os_faults::IO_ERROR));
                assert!(cleanup.is_none());
            }
            other => panic!("expected uncertain replacement acknowledgement: {other}"),
        }
        assert_eq!(fs::read(&path).unwrap(), b"new intent");
        assert_ne!(identity::file_id(&path).unwrap(), old_id);
        assert_eq!(names(folder.path()), [OsString::from("ledger.json")]);
    }

    #[test]
    fn failed_cleanup_reports_retained_temp_and_preserves_the_primary_io_cause() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ledger.json");
        fs::write(&path, b"old").unwrap();
        let _fault = os_faults::install(&[Point::FileSync, Point::Cleanup]);

        let error = write_file_durable(&path, b"new", DurableAccess::Private).unwrap_err();

        match error {
            DurableWriteError::BeforeReplace {
                source,
                cleanup: Some(cleanup),
            } => {
                assert_eq!(source.raw_os_error(), Some(os_faults::IO_ERROR));
                assert_eq!(cleanup.source.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(cleanup.path.parent(), Some(folder.path()));
                assert_ne!(cleanup.path, path);
                assert_eq!(fs::read(&cleanup.path).unwrap(), b"new");
                assert!(super::super::private::is_private(&cleanup.path).unwrap());
                assert_eq!(names(folder.path()).len(), 2);
                // Remove only the reported owned residue, not the destination.
                fs::remove_file(cleanup.path).unwrap();
            }
            other => panic!("expected primary and cleanup failures: {other}"),
        }
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(names(folder.path()), [OsString::from("ledger.json")]);
    }

    #[test]
    fn strict_faults_are_thread_scoped_and_legacy_writes_keep_their_contract() {
        let folder = tempfile::tempdir().unwrap();
        let strict = folder.path().join("strict");
        let legacy = folder.path().join("legacy");
        let other_thread = folder.path().join("other-thread");
        let _fault = os_faults::install(&[Point::FileSync]);

        assert!(matches!(
            write_file_durable(&strict, b"strict", DurableAccess::Private),
            Err(DurableWriteError::BeforeReplace { .. })
        ));
        write_file(&legacy, b"legacy", Access::KeepOrPrivate).unwrap();
        let other_id = std::thread::spawn(move || {
            write_file_durable(&other_thread, b"other", DurableAccess::Private).unwrap()
        })
        .join()
        .unwrap();

        assert!(!strict.exists());
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy");
        assert_eq!(
            fs::read(folder.path().join("other-thread")).unwrap(),
            b"other"
        );
        assert_eq!(
            identity::file_id(&folder.path().join("other-thread")).unwrap(),
            other_id
        );
        assert_eq!(
            names(folder.path()),
            [OsString::from("legacy"), OsString::from("other-thread")]
        );
    }
}
