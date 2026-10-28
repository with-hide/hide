//! Which file a name leads to, and whether two names lead to the same one.
//!
//! A path is a spelling: `/tmp` and `/private/tmp`, a link and its target, and
//! on a volume that ignores case `Readme` and `README` all name one file. What
//! identifies the file is the pair a filesystem keeps for it, which [`FileId`]
//! carries. [`canonical`] gives the spelling to show and to compare as text.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Handle;

/// The identity a filesystem keeps for a file: the volume it lives on and its
/// number there. Two names are one file exactly when their ids are equal, and
/// the id survives a rename.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileId {
    volume: u64,
    index: u128,
}

impl FileId {
    pub(crate) fn new(volume: u64, index: u128) -> Self {
        Self { volume, index }
    }

    /// The volume number: a device on Unix, a serial number on Windows.
    pub fn volume(&self) -> u64 {
        self.volume
    }

    /// The file's number on its volume: an inode on Unix, a 128-bit file id on
    /// Windows.
    pub fn index(&self) -> u128 {
        self.index
    }
}

/// The id of the file `path` leads to, links followed.
pub fn file_id(path: &Path) -> io::Result<FileId> {
    sys::id_of_path(path, true)
}

/// The id of the entry `path` names, a link's own and not its target's.
pub fn file_id_nofollow(path: &Path) -> io::Result<FileId> {
    sys::id_of_path(path, false)
}

/// The id of the entry `name` in the open folder `dir`, a link's own and not
/// its target's. On Unix the folder handle resolves the name, so a folder
/// renamed away from its path cannot redirect it; Windows has no such call
/// and resolves the folder's present path first.
pub fn entry_id(dir: &impl Handle, name: &std::ffi::OsStr) -> io::Result<FileId> {
    sys::entry_id(dir, name)
}

/// The id of an open file or folder.
pub fn file_id_of(handle: &impl Handle) -> io::Result<FileId> {
    sys::id_of_handle(handle)
}

/// How many names the open file has. A file with more than one is shared
/// with another name that a replacement of this one would separate from it.
pub fn link_count(handle: &impl Handle) -> io::Result<u64> {
    sys::link_count(handle)
}

/// What a file looks like now, to tell whether it changed between two looks:
/// its length, when its contents were last written and when anything about
/// it last changed. Equal stamps mean no change the filesystem recorded came
/// between them; the change time catches a write whose author set the write
/// time back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    len: u64,
    written: (i64, i64),
    changed: (i64, i64),
}

/// The stamp of an open file.
pub fn stamp_of(handle: &impl Handle) -> io::Result<Stamp> {
    sys::stamp_of(handle)
}

/// Whether the two paths lead to one file.
pub fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    Ok(file_id(left)? == file_id(right)?)
}

/// The path with every link followed and every spelling made the filesystem's
/// own, so two paths to one place compare equal as text. On Windows the
/// `\\?\` prefix the system adds is removed whenever the path means the same
/// without it.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    sys::canonical(path)
}

/// The part of `path` below `folder`, judged by identity rather than by
/// text: the rest of `path` past its nearest ancestor, itself included, that
/// leads to `folder`, so another spelling of the folder still counts (a link
/// to it, another letter case on a volume that ignores case, macOS's
/// `/System/Volumes/Data/Users/me` for `/Users/me`). Empty when `path` leads
/// to `folder` itself, `None` when no ancestor does; an ancestor that cannot
/// be looked up is not the folder.
pub fn below(path: &Path, folder: &Path) -> Option<PathBuf> {
    let folder = file_id(folder).ok()?;
    path.ancestors()
        .find(|ancestor| file_id(ancestor).is_ok_and(|id| id == folder))
        .and_then(|ancestor| path.strip_prefix(ancestor).ok())
        .map(Path::to_path_buf)
}

/// Whether `outer` is `inner` or a folder holding it, judged by identity:
/// `inner`'s own path below some ancestor of it, followed from `outer`,
/// leads to `inner`. So `/System/Volumes/Data` holds a home under `/Users`
/// on macOS though no text says so.
pub fn holds(outer: &Path, inner: &Path) -> bool {
    let (Ok(target), Ok(real)) = (file_id(inner), canonical(inner)) else {
        return false;
    };
    real.ancestors().any(|ancestor| {
        real.strip_prefix(ancestor)
            .is_ok_and(|rest| file_id(&outer.join(rest)).is_ok_and(|id| id == target))
    })
}

/// Whether an absolute path names a place on one of this machine's own
/// volumes: any path on macOS and Linux, a drive letter on Windows, in
/// either spelling, where a network share (`\\server\share`, `\\wsl$\...`)
/// and a device path are not. It reads nothing, so it can be asked before a
/// path is looked up, and again of what the path resolved to.
pub fn on_a_local_volume(path: &Path) -> bool {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        ),
        _ => true,
    }
}

/// The path the open file or folder is at now, as the system spells it:
/// after a rename it is the new path. On Windows it keeps the `\\?\`
/// prefix whatever its length, so two answers compare by prefix (a file
/// under a folder starts with the folder's answer), which [`canonical`]'s
/// shortened spelling does not promise. A file that was removed while open
/// has none on Linux (`NotFound`).
pub fn path_of(handle: &impl Handle) -> io::Result<PathBuf> {
    sys::path_of(handle)
}

/// Whether `folder` tells `Name` from `name`. It is a property of the folder
/// and not of the system: a Mac or a Windows volume may be either, and
/// Windows can make one folder case sensitive.
///
/// An entry the folder already holds, with its case swapped, answers it
/// without writing anything: the swapped name leads to the same entry on a
/// folder that ignores case and to nothing (or to another entry) on one that
/// does not. A folder with no entry that has a cased letter in its name is
/// answered by making a file in it and looking it up under a swapped name,
/// which needs the folder to be writable.
pub fn case_sensitive(folder: &Path) -> io::Result<bool> {
    match case_sensitive_by_entries(folder)? {
        Some(sensitive) => Ok(sensitive),
        None => probe_case(folder),
    }
}

/// [`case_sensitive`] from what `folder` already holds, writing nothing:
/// `None` when none of its first entries has a cased letter in its name, so
/// only a written probe could tell. For a reader that must not touch the
/// folder, such as an agent hook in the operator's checkout.
pub fn case_sensitive_by_entries(folder: &Path) -> io::Result<Option<bool>> {
    for entry in fs::read_dir(folder)?.take(PROBE_ENTRIES) {
        let name = entry?.file_name();
        let Some(swapped) = name.to_str().and_then(swap_case) else {
            continue;
        };
        return match file_id_nofollow(&folder.join(&swapped)) {
            Ok(found) => Ok(Some(found != file_id_nofollow(&folder.join(&name))?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Some(true)),
            Err(error) => Err(error),
        };
    }
    Ok(None)
}

/// How many entries are looked at for one with a cased name before the folder
/// is probed by writing (rule 15: a folder of ten thousand digits is not
/// walked).
const PROBE_ENTRIES: usize = 64;

/// `name` with every letter's case swapped, or `None` when it has no letter
/// that has a case.
fn swap_case(name: &str) -> Option<String> {
    let swapped: String = name
        .chars()
        .flat_map(|letter| {
            if letter.is_lowercase() {
                letter.to_uppercase().collect::<Vec<_>>()
            } else if letter.is_uppercase() {
                letter.to_lowercase().collect::<Vec<_>>()
            } else {
                vec![letter]
            }
        })
        .collect();
    (swapped != name).then_some(swapped)
}

fn probe_case(folder: &Path) -> io::Result<bool> {
    let name = super::temporary_name(std::ffi::OsStr::new("CaseProbe"), "Hide");
    let probe = folder.join(&name);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    let swapped = swap_case(&name.to_string_lossy()).unwrap_or_default();
    let found = fs::symlink_metadata(folder.join(swapped));
    let _ = fs::remove_file(&probe);
    match found {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

/// `\\?\C:\a` as `C:\a` and `\\?\UNC\host\share\a` as `\\host\share\a`, when
/// the shorter spelling means the same path: it fits the classic length limit
/// and has no component Win32 would rewrite. Anything else is `None` and keeps
/// the prefix. It is text only, so it is checked on every system.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn strip_verbatim(path: &str) -> Option<String> {
    const CLASSIC_LIMIT: usize = 259;
    short_verbatim(path).filter(|short| short.len() <= CLASSIC_LIMIT)
}

/// [`strip_verbatim`] at any length: the spelling Win32 reads as the same
/// path once its own long-path handling adds the prefix back, which is what
/// the wire spelling (`crate::path`) needs; `None` where it would not.
pub(crate) fn short_verbatim(path: &str) -> Option<String> {
    let (stripped, unc) = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        (rest, true)
    } else {
        let rest = path.strip_prefix(r"\\?\")?;
        let mut letters = rest.chars();
        let drive = letters.next()?;
        if !drive.is_ascii_alphabetic()
            || letters.next() != Some(':')
            || letters.next() != Some('\\')
        {
            return None;
        }
        (rest, false)
    };
    let body = if unc { stripped } else { &stripped[3..] };
    if body.split('\\').any(|component| {
        component.is_empty() || component == "." || component == ".." || win32_rewrites(component)
    }) && !body.is_empty()
    {
        return None;
    }
    Some(if unc {
        format!(r"\\{stripped}")
    } else {
        stripped.to_owned()
    })
}

/// Whether Win32 would read a name as something other than itself outside a
/// `\\?\` path: a trailing space or dot it drops, a device name, or a `/` it
/// takes as a separator.
pub(crate) fn win32_rewrites(component: &str) -> bool {
    component.ends_with([' ', '.']) || component.contains('/') || reserved_device(component)
}

/// Whether Win32 would read the component as a device (`CON`, `NUL`, `COM1`,
/// ...), with or without an extension.
fn reserved_device(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or_default();
    let upper = stem.trim_end().to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|digit| {
            matches!(digit, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                || matches!(digit, "¹" | "²" | "³")
        })
    })
}

#[cfg(unix)]
mod sys {
    use std::fs;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    use super::{FileId, Handle, Stamp};

    fn id(metadata: &fs::Metadata) -> FileId {
        FileId::new(metadata.dev(), u128::from(metadata.ino()))
    }

    pub(super) fn id_of_path(path: &Path, follow: bool) -> io::Result<FileId> {
        let metadata = if follow {
            fs::metadata(path)?
        } else {
            fs::symlink_metadata(path)?
        };
        Ok(id(&metadata))
    }

    fn metadata_of(handle: &impl Handle) -> io::Result<fs::Metadata> {
        crate::fs::duplicate(handle)?.metadata()
    }

    pub(super) fn id_of_handle(handle: &impl Handle) -> io::Result<FileId> {
        Ok(id(&metadata_of(handle)?))
    }

    pub(super) fn link_count(handle: &impl Handle) -> io::Result<u64> {
        Ok(metadata_of(handle)?.nlink())
    }

    pub(super) fn stamp_of(handle: &impl Handle) -> io::Result<Stamp> {
        let metadata = metadata_of(handle)?;
        Ok(Stamp {
            len: metadata.len(),
            written: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }

    pub(super) fn entry_id(dir: &impl Handle, name: &std::ffi::OsStr) -> io::Result<FileId> {
        use std::mem::MaybeUninit;
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name holds a NUL"))?;
        let mut found = MaybeUninit::<libc::stat>::zeroed();
        // SAFETY: the descriptor is an open folder borrowed for the call, the
        // name is NUL-terminated, and `found` is a writable `stat`.
        let result = unsafe {
            libc::fstatat(
                dir.as_fd().as_raw_fd(),
                name.as_ptr(),
                found.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and filled the structure.
        let found = unsafe { found.assume_init() };
        // The device is `i32` on macOS and `u64` on Linux; `as` gives the
        // same number the standard library's `dev()` does on each.
        #[allow(clippy::unnecessary_cast)]
        let device = found.st_dev as u64;
        Ok(FileId::new(device, u128::from(found.st_ino)))
    }

    pub(super) fn canonical(path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn path_of(handle: &impl Handle) -> io::Result<PathBuf> {
        use std::ffi::CStr;
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let mut path = [0 as libc::c_char; libc::PATH_MAX as usize];
        // SAFETY: the buffer is writable for PATH_MAX bytes, which F_GETPATH
        // needs, and the descriptor is borrowed for the call.
        if unsafe {
            libc::fcntl(
                handle.as_fd().as_raw_fd(),
                libc::F_GETPATH,
                path.as_mut_ptr(),
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: F_GETPATH wrote a NUL-terminated path.
        let bytes = unsafe { CStr::from_ptr(path.as_ptr()) }.to_bytes();
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn path_of(handle: &impl Handle) -> io::Result<PathBuf> {
        use std::os::fd::AsRawFd;
        let path = fs::read_link(format!("/proc/self/fd/{}", handle.as_fd().as_raw_fd()))?;
        if path.to_string_lossy().ends_with(" (deleted)") {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the open file was removed",
            ));
        }
        Ok(path)
    }
}

#[cfg(windows)]
mod sys {
    use std::fs;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo,
        GetFileInformationByHandle, GetFileInformationByHandleEx, GetFileSizeEx,
    };

    use super::{FileId, Handle, Stamp};

    pub(super) fn id_of_path(path: &Path, follow: bool) -> io::Result<FileId> {
        id_of_handle(&crate::fs::open_for_query(path, follow)?)
    }

    pub(super) fn id_of_handle(handle: &impl Handle) -> io::Result<FileId> {
        let raw = handle.as_handle().as_raw_handle();
        let mut info = MaybeUninit::<FILE_ID_INFO>::zeroed();
        // SAFETY: `raw` is an open handle borrowed for the call, and `info` is
        // a writable FILE_ID_INFO of the size passed.
        let done = unsafe {
            GetFileInformationByHandleEx(
                raw,
                FileIdInfo,
                info.as_mut_ptr().cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        };
        if done == 0 {
            // FAT, exFAT and old network shares keep no 128-bit id; the
            // 64-bit one is the same file's identity on those volumes.
            return id_of_handle_classic(raw);
        }
        // SAFETY: the call succeeded and filled the structure.
        let info = unsafe { info.assume_init() };
        Ok(FileId::new(
            info.VolumeSerialNumber,
            u128::from_le_bytes(info.FileId.Identifier),
        ))
    }

    fn id_of_handle_classic(raw: HANDLE) -> io::Result<FileId> {
        let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: `raw` is an open handle borrowed by the caller and `info` is
        // a writable structure of the size the call expects.
        if unsafe { GetFileInformationByHandle(raw, info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and filled the structure.
        let info = unsafe { info.assume_init() };
        Ok(FileId::new(
            u64::from(info.dwVolumeSerialNumber),
            (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
        ))
    }

    pub(super) fn entry_id(dir: &impl Handle, name: &std::ffi::OsStr) -> io::Result<FileId> {
        id_of_path(&crate::fs::path_of(dir)?.join(name), false)
    }

    pub(super) fn link_count(handle: &impl Handle) -> io::Result<u64> {
        let raw = handle.as_handle().as_raw_handle();
        let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: as in `id_of_handle`.
        if unsafe { GetFileInformationByHandle(raw, info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and filled the structure.
        Ok(u64::from(unsafe { info.assume_init() }.nNumberOfLinks))
    }

    pub(super) fn stamp_of(handle: &impl Handle) -> io::Result<Stamp> {
        let raw = handle.as_handle().as_raw_handle();
        let mut info = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
        // SAFETY: `raw` is an open handle borrowed for the call, and `info` is
        // a writable FILE_BASIC_INFO of the size passed.
        if unsafe {
            GetFileInformationByHandleEx(
                raw,
                FileBasicInfo,
                info.as_mut_ptr().cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded and filled the structure.
        let info = unsafe { info.assume_init() };
        let mut length = 0i64;
        // SAFETY: as above, with a writable i64.
        if unsafe { GetFileSizeEx(raw, &mut length) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Stamp {
            len: u64::try_from(length).unwrap_or(0),
            // Windows keeps both times in 100 ns units in one number.
            written: (info.LastWriteTime, 0),
            changed: (info.ChangeTime, 0),
        })
    }

    pub(super) fn canonical(path: &Path) -> io::Result<PathBuf> {
        Ok(short(fs::canonicalize(path)?))
    }

    pub(super) fn path_of(handle: &impl Handle) -> io::Result<PathBuf> {
        crate::fs::path_of(handle)
    }

    /// The path without the `\\?\` prefix wherever it means the same.
    fn short(real: PathBuf) -> PathBuf {
        real.to_str()
            .and_then(super::strip_verbatim)
            .map_or(real, PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbatim_prefix_is_stripped_only_where_the_short_form_means_the_same() {
        assert_eq!(strip_verbatim(r"\\?\C:\a\b").as_deref(), Some(r"C:\a\b"));
        assert_eq!(
            strip_verbatim(r"\\?\UNC\host\share\a").as_deref(),
            Some(r"\\host\share\a")
        );
        assert_eq!(strip_verbatim(r"C:\a"), None);
        assert_eq!(strip_verbatim(r"\\?\Volume{1234}\a"), None);
        assert_eq!(strip_verbatim(r"\\?\C:\a\NUL"), None);
        assert_eq!(strip_verbatim(r"\\?\C:\a\com1.txt"), None);
        assert_eq!(strip_verbatim(r"\\?\C:\a\trailing."), None);
        assert_eq!(strip_verbatim(r"\\?\C:\a\..\b"), None);
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(strip_verbatim(&long), None);
    }

    #[test]
    fn a_swapped_name_exists_only_for_a_name_with_a_cased_letter() {
        assert_eq!(swap_case("Readme.md").as_deref(), Some("rEADME.MD"));
        assert_eq!(swap_case("1234"), None);
    }
}
