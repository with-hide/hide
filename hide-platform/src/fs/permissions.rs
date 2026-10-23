//! What an open file allows its owner, kept across a replacement.

use std::fs;
use std::io;
use std::path::Path;

use super::Handle;

/// The permissions of one file as a replacement has to keep them: the mode
/// bits on Unix, the read-only flag on Windows (whose access list a
/// replacement keeps by replacing through the system's own call, see
/// [`super::atomic`]).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Permissions {
    inner: sys::Inner,
}

impl Permissions {
    /// The permissions of the open file.
    pub fn of(handle: &impl Handle) -> io::Result<Self> {
        Ok(Self {
            inner: sys::of(handle)?,
        })
    }

    /// Whether the owner may change the file's contents.
    pub fn owner_can_write(&self) -> bool {
        sys::owner_can_write(&self.inner)
    }

    /// Gives `file` exactly these permissions, whatever the creating process's
    /// umask took off.
    pub fn apply(&self, file: &fs::File) -> io::Result<()> {
        sys::apply(&self.inner, file)
    }

    /// The mode bits, `None` where the system has none.
    pub fn unix_mode(&self) -> Option<u32> {
        sys::unix_mode(&self.inner)
    }
}

/// Whether this account may add or replace an entry in the folder `dir`,
/// which decides whether a command that puts a file there needs `sudo`.
/// Unix asks the kernel (`access` for write and search); Windows has no such
/// question short of reading the access list, so it adds and removes an empty
/// file of its own there.
pub fn account_can_add_to(dir: &Path) -> io::Result<bool> {
    sys::account_can_add_to(dir)
}

/// Whether the file at `path` is one the system will run. Unix asks for an
/// execute bit; Windows has no such bit and runs a file by its extension, so
/// it asks for one of the extensions the shell launches.
pub fn is_executable(path: &Path) -> io::Result<bool> {
    sys::is_executable(path)
}

#[cfg(unix)]
mod sys {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    use super::Handle;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(super) struct Inner(u32);

    pub(super) fn of(handle: &impl Handle) -> io::Result<Inner> {
        let metadata = crate::fs::duplicate(handle)?.metadata()?;
        Ok(Inner(metadata.mode() & 0o7777))
    }

    pub(super) fn owner_can_write(inner: &Inner) -> bool {
        inner.0 & 0o200 != 0
    }

    pub(super) fn apply(inner: &Inner, file: &fs::File) -> io::Result<()> {
        file.set_permissions(fs::Permissions::from_mode(inner.0))
    }

    pub(super) fn unix_mode(inner: &Inner) -> Option<u32> {
        Some(inner.0)
    }

    pub(super) fn is_executable(path: &Path) -> io::Result<bool> {
        Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
    }

    pub(super) fn account_can_add_to(dir: &Path) -> io::Result<bool> {
        use std::os::unix::ffi::OsStrExt;
        if !fs::metadata(dir)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder"));
        }
        let name = std::ffi::CString::new(dir.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: `name` is a NUL-terminated path that outlives the call.
        if unsafe { libc::access(name.as_ptr(), libc::W_OK | libc::X_OK) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EACCES | libc::EROFS | libc::EPERM) => Ok(false),
            _ => Err(error),
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsStr;
    use std::fs;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_READONLY, GetFileInformationByHandle,
    };

    use super::Handle;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(super) struct Inner {
        read_only: bool,
    }

    pub(super) fn of(handle: &impl Handle) -> io::Result<Inner> {
        let raw = handle.as_handle().as_raw_handle();
        let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: `raw` is an open handle borrowed for the call and `info` is
        // a writable structure of the size the call expects.
        if unsafe { GetFileInformationByHandle(raw, info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and filled the structure.
        let attributes = unsafe { info.assume_init() }.dwFileAttributes;
        Ok(Inner {
            read_only: attributes & FILE_ATTRIBUTE_READONLY != 0,
        })
    }

    pub(super) fn owner_can_write(inner: &Inner) -> bool {
        !inner.read_only
    }

    pub(super) fn apply(inner: &Inner, file: &fs::File) -> io::Result<()> {
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(inner.read_only);
        file.set_permissions(permissions)
    }

    pub(super) fn unix_mode(_: &Inner) -> Option<u32> {
        None
    }

    /// The extensions `CreateProcess` and the shell run directly.
    const RUNNABLE: [&str; 6] = ["exe", "com", "bat", "cmd", "ps1", "msc"];

    pub(super) fn account_can_add_to(dir: &Path) -> io::Result<bool> {
        if !fs::metadata(dir)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder"));
        }
        let probe = dir.join(format!(".hide-write-check-{}", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => {
                drop(file);
                fs::remove_file(&probe)?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn is_executable(path: &Path) -> io::Result<bool> {
        fs::metadata(path)?;
        Ok(path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                RUNNABLE
                    .iter()
                    .any(|runnable| extension.eq_ignore_ascii_case(runnable))
            }))
    }
}
