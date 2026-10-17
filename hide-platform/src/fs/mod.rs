//! Files and folders: private ones, locks, atomic replacement, links and
//! identity.
//!
//! Callers hold folders and files in whatever form their crate uses (a
//! `cap_std::fs::Dir` on a checkout, a `File` elsewhere), so every function
//! that works on an open one takes [`Handle`], the standard library's borrowed
//! descriptor (`AsFd` on Unix, `AsHandle` on Windows). A function that takes a
//! `Path` is the same operation through a name. Nothing here depends on
//! `cap-std`.

pub mod atomic;
pub mod holders;
pub mod identity;
pub mod in_use;
pub mod link;
pub mod lock;
pub mod permissions;
pub mod private;
pub mod space;

/// A borrowed open file or folder.
#[cfg(unix)]
pub use std::os::fd::AsFd as Handle;
/// A borrowed open file or folder.
#[cfg(windows)]
pub use std::os::windows::io::AsHandle as Handle;

/// Opens the folder `path` as a handle, which is what a lock, an exchange or a
/// durable rename of its entries takes. Windows opens a folder only when
/// asked for backup semantics, which a plain `File::open` does not.
pub fn open_dir(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS.
        options.custom_flags(0x0200_0000);
    }
    let folder = options.open(path)?;
    if !folder.metadata()?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "the path is not a folder",
        ));
    }
    Ok(folder)
}

/// Opens the directory entry itself. A link, pipe or device cannot redirect
/// the open or leave it waiting for a writer.
pub fn open_dir_nofollow(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000 | 0x0200_0000);
    }
    let folder = options.open(path)?;
    if !folder.metadata()?.is_dir()
        || identity::file_id_of(&folder)? != identity::file_id_nofollow(path)?
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the entry is not a directory",
        ));
    }
    Ok(folder)
}

/// Opens `path` for reading only when the name itself is a regular file. A
/// link at the name is not followed (an error on Unix, `InvalidInput` on
/// Windows, which opens the link as itself), and a folder, a pipe or a device
/// neither blocks the open nor is accepted (`InvalidInput`).
pub fn open_regular(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT, and FILE_FLAG_BACKUP_SEMANTICS so a
        // folder opens and is refused below like anything else.
        options.custom_flags(0x0020_0000 | 0x0200_0000);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path is not a regular file",
        ));
    }
    Ok(file)
}

/// The open file or folder behind `handle` as a `File`, through a duplicate
/// descriptor, so its metadata can be read without disturbing the caller's.
#[cfg(unix)]
pub(crate) fn duplicate(handle: &impl Handle) -> std::io::Result<std::fs::File> {
    Ok(std::fs::File::from(handle.as_fd().try_clone_to_owned()?))
}

/// A readable descriptor for the folder `dir` holds. `cap-std` opens its
/// folders with `O_PATH` on Linux, and the kernel answers `EBADF` to `flock`
/// and `fsync` on such a descriptor, so a lock or a flush goes through a fresh
/// one; macOS has no `O_PATH` and the same call works there.
#[cfg(unix)]
pub(crate) fn reopen_dir_for_io(dir: &impl Handle) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: the descriptor is an open folder borrowed for the call, and the
    // path is a NUL-terminated ".".
    let fd = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: openat returned a new descriptor that nothing else owns.
    Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

/// The open folder `dir` holds, opened again with the right to add an entry
/// to it, which is what flushing its entries asks of a handle
/// (`NtFlushBuffersFileEx` answers access denied to a handle with neither
/// write nor append access). The open is `NtCreateFile` of an empty name
/// relative to `dir`, which is the same folder and never what its path names
/// now; `ReOpenFile` would say the same but refuses every folder. It asks no
/// backup intent, so a privilege never opens what the folder's access list
/// does not grant. A folder this account may add neither a file nor a folder
/// to is `PermissionDenied`.
#[cfg(windows)]
pub(crate) fn reopen_dir_for_flush(dir: &impl Handle) -> std::io::Result<std::fs::File> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_DIRECTORY_FILE, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{RtlNtStatusToDosError, UNICODE_STRING};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    // A zero-length name; a null one is refused as an invalid name.
    let name = UNICODE_STRING {
        Length: 0,
        MaximumLength: 0,
        Buffer: std::ptr::null_mut(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: dir.as_handle().as_raw_handle(),
        ObjectName: &name,
        ..Default::default()
    };
    let mut refused = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    // Either right lets the flush through; a folder can grant one alone.
    for access in [FILE_ADD_SUBDIRECTORY, FILE_ADD_FILE] {
        let mut handle = std::ptr::null_mut();
        let mut status_block = IO_STATUS_BLOCK::default();
        // SAFETY: the root handle is open and borrowed for the call, and
        // every pointer is to a local that outlives it.
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                access | SYNCHRONIZE,
                &attributes,
                &mut status_block,
                std::ptr::null(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
                std::ptr::null(),
                0,
            )
        };
        if status >= 0 {
            // SAFETY: the call returned a new handle that nothing else owns.
            return Ok(unsafe { std::fs::File::from_raw_handle(handle) });
        }
        // SAFETY: the conversion reads only its argument.
        refused =
            std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32);
        if refused.kind() != std::io::ErrorKind::PermissionDenied {
            break;
        }
    }
    Err(refused)
}

/// The path the open file or folder is at now, as the system spells it
/// (with the `\\?\` prefix).
#[cfg(windows)]
pub(crate) fn path_of(dir: &impl Handle) -> std::io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    let raw = dir.as_handle().as_raw_handle();
    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: the handle is open and borrowed, and `buffer` holds the
        // length passed.
        let length = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW(
                raw,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // On success the count excludes the terminating NUL, so it is less
        // than the buffer; a count as large is the size the path needs.
        if length < buffer.len() {
            return Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                &buffer[..length],
            )));
        }
        buffer.resize(length, 0);
    }
}

/// A handle to `path` that asks no access, which is all identity and size
/// need and which no other open file can refuse. With `follow` false a link
/// at the name is opened as itself.
#[cfg(windows)]
pub(crate) fn open_for_query(
    path: &std::path::Path,
    follow: bool,
) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(flags)
        .open(path)
}

/// `path` as the NUL-terminated wide string Windows calls take.
#[cfg(windows)]
pub(crate) fn wide(path: &std::path::Path) -> std::io::Result<widestring::U16CString> {
    widestring::U16CString::from_os_str(path.as_os_str())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
}

/// Who may use a file or folder this crate creates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    /// The current account alone: mode 0600 (0700 for a folder) on Unix, an
    /// access list naming only the current account on Windows.
    Private,
    /// [`Access::Private`] for a file that runs: 0700 on Unix. Windows runs a
    /// file by its extension, so it is the same as `Private` there.
    PrivateExecutable,
    /// What the file being replaced has, or `Private` when there is none.
    KeepOrPrivate,
}

/// A name that is not yet taken in `dir`, built from `stem`, this process and
/// a counter. The caller creates the file exclusively, so a clash is only a
/// reason to ask again.
pub(crate) fn temporary_name(stem: &std::ffi::OsStr, tag: &str) -> std::ffi::OsString {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = std::ffi::OsString::from(".");
    name.push(stem);
    name.push(format!(
        ".{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    name
}
