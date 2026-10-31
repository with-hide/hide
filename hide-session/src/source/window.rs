//! Reading a bounded window of a session file: its tail, a page of whole
//! records before an offset, the whole file under a cap, and the bytes
//! appended since a reader's last position.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::file::FileIdentity;
use crate::{RescanReason, Result, SessionError};

/// The whole records among a file's last `maximum_bytes`, in one read: the
/// record the window's start cuts and a last record without its newline yet
/// are left out. A record longer than the window leaves the tail empty.
pub fn read_tail(path: &Path, maximum_bytes: u64) -> Result<PageBefore> {
    let mut file =
        super::file::open(path).map_err(|error| SessionError::io("open", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| SessionError::io("stat", path, error))?
        .len();
    let start = length.saturating_sub(maximum_bytes);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| SessionError::io("seek", path, error))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(length - start)
        .read_to_end(&mut bytes)
        .map_err(|error| SessionError::io("read", path, error))?;
    let first = if start == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |newline| newline + 1)
    };
    let last = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(first, |newline| (newline + 1).max(first));
    Ok(PageBefore {
        contents: String::from_utf8_lossy(&bytes[first..last]).into_owned(),
        start_offset: start + first as u64,
        end_offset: start + last as u64,
    })
}

/// The file's first record, when it is whole within its first
/// `maximum_bytes`, without its newline.
pub fn read_first_record(path: &Path, maximum_bytes: u64) -> Result<Option<String>> {
    let file = super::file::open(path).map_err(|error| SessionError::io("open", path, error))?;
    let mut head = super::frame::Head::new(file.take(maximum_bytes), maximum_bytes as usize);
    match head
        .next_record()
        .map_err(|error| SessionError::io("read", path, error))?
    {
        Some(super::frame::HeadRecord::Complete(record)) => {
            Ok(Some(String::from_utf8_lossy(&record).into_owned()))
        }
        _ => Ok(None),
    }
}

/// One page of complete lines read backwards from a line boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBefore {
    pub contents: String,
    /// Where `contents` begins, and where the next older page ends; 0 is the
    /// start of the file, with nothing older.
    pub start_offset: u64,
    /// Where `contents` ends: the `end` asked for, or after the file's last
    /// newline, so a line still being written is left to the cursor.
    pub end_offset: u64,
}

/// The block a backwards scan for a line start reads at a time.
const SCAN_BLOCK_BYTES: u64 = 64 * 1024;

/// Read the complete lines that end at or before `end`, at most
/// `maximum_bytes` of them, dropping the line the window cuts.
///
/// `end` must be a line boundary: 0, a previous page's `start_offset`, or
/// `None` for the file's last newline. A line longer than the window is
/// passed over by scanning back to its start, without keeping its bytes.
pub fn read_page_before(path: &Path, end: Option<u64>, maximum_bytes: u64) -> Result<PageBefore> {
    let file = super::file::open(path).map_err(|error| SessionError::io("open", path, error))?;
    page_before(&file, path, end, maximum_bytes)
}

/// [`read_page_before`] through a file the caller opened.
pub(crate) fn page_before(
    mut file: &File,
    path: &Path,
    end: Option<u64>,
    maximum_bytes: u64,
) -> Result<PageBefore> {
    let length = file
        .metadata()
        .map_err(|error| SessionError::io("stat", path, error))?
        .len();
    let end = match end {
        Some(end) => end.min(length),
        None => line_start_before(file, path, length)?,
    };
    let start = end.saturating_sub(maximum_bytes);
    // One byte before the window, so a line that starts exactly at `start` is
    // known to be whole.
    let read_start = start.saturating_sub(1);
    let mut bytes = vec![0; (end - read_start) as usize];
    file.seek(SeekFrom::Start(read_start))
        .map_err(|error| SessionError::io("seek", path, error))?;
    file.read_exact(&mut bytes)
        .map_err(|error| SessionError::io("read", path, error))?;
    let (start_offset, bytes) = if start == 0 {
        (0, bytes.as_slice())
    } else {
        // The window's last byte ends the newest line; a line starts after an
        // earlier newline or not at all.
        let body = &bytes[..bytes.len().saturating_sub(1)];
        match body.iter().position(|byte| *byte == b'\n') {
            Some(newline) => (read_start + newline as u64 + 1, &bytes[newline + 1..]),
            None => (line_start_before(file, path, start)?, &[][..]),
        }
    };
    Ok(PageBefore {
        contents: String::from_utf8_lossy(bytes).into_owned(),
        start_offset,
        end_offset: end,
    })
}

/// The offset just after the last newline before `end`, or 0.
fn line_start_before(mut file: &File, path: &Path, end: u64) -> Result<u64> {
    let mut block_end = end;
    let mut block = Vec::new();
    while block_end > 0 {
        let block_start = block_end.saturating_sub(SCAN_BLOCK_BYTES);
        block.resize((block_end - block_start) as usize, 0);
        file.seek(SeekFrom::Start(block_start))
            .map_err(|error| SessionError::io("seek", path, error))?;
        file.read_exact(&mut block)
            .map_err(|error| SessionError::io("read", path, error))?;
        if let Some(newline) = block.iter().rposition(|byte| *byte == b'\n') {
            return Ok(block_start + newline as u64 + 1);
        }
        block_end = block_start;
    }
    Ok(0)
}

/// Read a whole file while enforcing a hard byte cap before and during I/O.
pub fn read_bounded(path: &Path, maximum_bytes: u64) -> Result<String> {
    let file = super::file::open(path).map_err(|error| SessionError::io("open", path, error))?;
    let bytes = read_whole(&file, maximum_bytes)
        .map_err(|error| SessionError::io("read", path, error))?
        .ok_or(SessionError::Capacity {
            resource: "file_bytes",
            limit: maximum_bytes,
        })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// All of an open file, `None` when it holds more than `maximum_bytes`:
/// its size is checked before reading and the read stops one byte past the
/// cap, so a file that grows meanwhile is refused too.
pub(crate) fn read_whole(mut file: &File, maximum_bytes: u64) -> std::io::Result<Option<Vec<u8>>> {
    let length = file.metadata()?.len();
    if length > maximum_bytes {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(length as usize);
    (&mut file)
        .take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= maximum_bytes).then_some(bytes))
}

impl PageBefore {
    /// The page's records, each with the offset it starts at.
    pub fn records(&self) -> impl Iterator<Item = (u64, &str)> {
        let mut offset = self.start_offset;
        self.contents.split_inclusive('\n').map(move |record| {
            let at = offset;
            offset += record.len() as u64;
            (at, record)
        })
    }
}

/// The bytes appended to a session file since a reader's last position.
pub(crate) struct Appended {
    pub(crate) contents: Vec<u8>,
    /// Where `contents` begins: the reader's position, or 0 when the file is
    /// new to it, replaced or truncated.
    pub(crate) start_offset: u64,
    pub(crate) identity: FileIdentity,
    /// Why the reader starts over from the file's start, if it does.
    pub(crate) rescan_reason: Option<RescanReason>,
    /// Bytes remain past `contents`.
    pub(crate) has_more: bool,
}

/// Reads at most `budget` bytes of `file` from `offset`, the position a
/// reader of the file `known` reached; a file that is not `known`
/// (replaced) or shorter than `offset` (truncated) is read from its start
/// and says why. Reads through an already-open descriptor, so a path
/// replaced meanwhile cannot race the read.
pub(crate) fn read_appended(
    path: &Path,
    file: &File,
    known: Option<FileIdentity>,
    offset: u64,
    budget: u64,
    read_bytes: &mut u64,
) -> Result<Appended> {
    let mut file = file
        .try_clone()
        .map_err(|e| SessionError::io("clone", path, e))?;
    let length = file
        .metadata()
        .map_err(|e| SessionError::io("stat", path, e))?
        .len();
    let identity =
        FileIdentity::of(&file).map_err(|error| SessionError::io("identity", path, error))?;
    let rescan_reason = match known {
        Some(old) if old != identity => Some(RescanReason::Replaced),
        Some(_) if length < offset => Some(RescanReason::Truncated),
        _ => None,
    };
    let start = if known.is_none() || rescan_reason.is_some() {
        0
    } else {
        offset
    };
    file.seek(SeekFrom::Start(start))
        .map_err(|e| SessionError::io("seek", path, e))?;
    let mut contents = Vec::new();
    let read = file.take(budget).read_to_end(&mut contents);
    // Failed parsing or a partial I/O failure still spent this read's bytes.
    *read_bytes = contents.len() as u64;
    read.map_err(|e| SessionError::io("read", path, e))?;
    Ok(Appended {
        has_more: start + (contents.len() as u64) < length,
        contents,
        start_offset: start,
        identity,
        rescan_reason,
    })
}
