//! Opening a session file and naming the file that was opened.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Opens a session file for reading without waiting on it: a pipe or a
/// device named where a session file should be is refused at once instead of
/// blocking the reader, and so is a folder.
pub(crate) fn open(path: &Path) -> io::Result<File> {
    open_with_links(path, true)
}

/// [`open`] that also refuses a link at `path` itself, for a reader whose
/// path was proven inside a session folder and must open that very file.
pub(crate) fn open_nofollow(path: &Path) -> io::Result<File> {
    open_with_links(path, false)
}

fn open_with_links(path: &Path, follow: bool) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
    }
    let file = options.open(path)?;
    if !follow
        && hide_platform::fs::identity::file_id_nofollow(path)?
            != hide_platform::fs::identity::file_id_of(&file)?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session source is linked.",
        ));
    }
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Session source is not a regular file.",
        ));
    }
    Ok(file)
}

/// Which file a descriptor reads, so a reader that resumes by offset can
/// tell an atomic replacement at the same path from an append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct FileIdentity {
    pub(crate) first: u64,
    pub(crate) second: u64,
    /// Windows keeps a 128-bit file index. Old Unix checkpoints omit this.
    #[serde(default)]
    pub(crate) high: u64,
}

impl FileIdentity {
    pub(crate) fn of(file: &File) -> io::Result<Self> {
        let id = hide_platform::fs::identity::file_id_of(file)?;
        Ok(Self {
            first: id.volume(),
            second: id.index() as u64,
            high: (id.index() >> 64) as u64,
        })
    }

    /// The identity as `<volume>:<index>`, the one spelling every reader
    /// compares and keeps (ownership proofs, thread cursors, usage and
    /// Cursor checkpoints).
    pub(crate) fn incarnation(self) -> String {
        let index = (u128::from(self.high) << 64) | u128::from(self.second);
        format!("{}:{index}", self.first)
    }
}

/// [`FileIdentity::incarnation`] of an open file.
pub(crate) fn incarnation(file: &File) -> io::Result<String> {
    Ok(FileIdentity::of(file)?.incarnation())
}
