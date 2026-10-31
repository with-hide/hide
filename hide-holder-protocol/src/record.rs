//! A pane's record: what a state folder keeps of each holder started for
//! it, which outlives both the node and the holder (PRD
//! engine-bundle-2-pane-holder D-14).
//!
//! The record, not the node, is a holder's owner: a node reserves it before
//! the holder starts, the holder writes itself into it before it starts the
//! pane's program and once it runs, and writes why the pane ended before it
//! exits, whether or not a node is connected. A node that starts again reads
//! the records to tell the holders it may adopt from the ones nobody owns.
//! In a later bundle the core takes this contract over unchanged, so the
//! format carries a number of its own, and a reader that meets another one
//! says so rather than reading it as no record.
//!
//! Every change of a record is made under one lock per state folder, held
//! briefly: a read, a decision, an atomic replace. Nothing waits on another
//! process while it holds the lock. The paths are named by
//! `hide_kit::layout`; this module takes them as given.

use std::fmt;
use std::io;
use std::path::Path;
use std::time::Duration;

use hide_platform::fs::Access;
use hide_platform::fs::lock::{Lock, Mode, Waited, lock_file};
use serde::{Deserialize, Serialize};

use crate::CloseReason;

/// The format this build reads and writes.
pub const RECORD_FORMAT: u64 = 1;

/// The most bytes a record file holds; a larger one is unreadable.
pub const MAX_RECORD: usize = 64 * 1024;

/// The longest pane id, which names the record, the socket and the log.
pub const MAX_PANE: usize = 64;

/// How long anyone waits for the records' lock before giving up. Its holder
/// only reads, decides and replaces a file under it.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// The holder's log event that says it ended but could not write its record,
/// with the end it meant to write (`reason`, `exit_code`, `left`) and the
/// `error`; a node that later finds the holder gone reads it from the log
/// instead of taking the pane for one whose holder was killed.
pub const UNWRITTEN_EVENT: &str = "holder.record_unwritten";

/// Where a record is in a pane's life.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Reserved by a node; the holder may not have started, or may be
    /// starting the pane's program.
    Preparing,
    /// The holder's program runs and its socket listens.
    Running,
    /// A node asked the pane to end.
    Closing,
    /// Nothing of the pane runs; [`PaneRecord::end`] says why.
    Ended,
}

/// A process named by its pid and the start time it had then, so a later
/// process that took the pid is never taken for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub started: u64,
}

impl Identity {
    /// `pid` as it runs now; `None` when it has ended.
    pub fn of(pid: u32) -> Option<Self> {
        let started = hide_platform::process::start_time(pid).ok()?;
        hide_platform::process::is_alive(pid).then_some(Self { pid, started })
    }

    /// Whether the process it names still runs.
    pub fn runs(&self) -> bool {
        hide_platform::process::is_alive(self.pid)
            && hide_platform::process::start_time(self.pid).ok() == Some(self.started)
    }
}

/// Why a pane ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// A node asked it to end, and its whole tree ended.
    Terminated,
    /// The pane's program exited, and what it left ended.
    ProgramExited,
    /// The holder could not confirm the tree ended; `left` names what ran.
    TreeEndUnconfirmed,
    /// The holder failed and ended the pane.
    HolderFailed,
    /// The holder ended without saying why (killed outright, or it crashed),
    /// and what its pane left was ended afterwards.
    HolderLost,
    /// The holder ended and logged why, but could not write it here; the
    /// end's detail carries what it logged.
    Unrecorded,
    /// The record is from another boot of the machine, which ended
    /// everything it named.
    MachineRestarted,
    /// The holder never started, or failed before its program ran.
    StartFailed,
    /// The node stopped waiting for the start; a holder that comes late
    /// finds this and ends itself.
    StartUnconfirmed,
    /// The node that reserved the start ended before the holder wrote
    /// itself in, and the start's bound passed.
    StartInterrupted,
}

impl EndReason {
    /// The end a holder reports with `reason`.
    pub fn of_close(reason: CloseReason) -> Self {
        match reason {
            CloseReason::Terminated => Self::Terminated,
            CloseReason::ProgramExited => Self::ProgramExited,
            CloseReason::TreeEndUnconfirmed => Self::TreeEndUnconfirmed,
            _ => Self::HolderFailed,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Terminated => "terminated",
            Self::ProgramExited => "program_exited",
            Self::TreeEndUnconfirmed => "tree_end_unconfirmed",
            Self::HolderFailed => "holder_failed",
            Self::HolderLost => "holder_lost",
            Self::Unrecorded => "unrecorded",
            Self::MachineRestarted => "machine_restarted",
            Self::StartFailed => "start_failed",
            Self::StartUnconfirmed => "start_unconfirmed",
            Self::StartInterrupted => "start_interrupted",
        }
    }
}

/// How a pane ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct End {
    pub reason: EndReason,
    /// The program's exit code, when it exited with one.
    pub exit_code: Option<i32>,
    /// Processes of the pane still running when the end gave up.
    pub left: Vec<u32>,
    pub detail: String,
    pub at_unix_ms: u64,
}

impl End {
    pub fn new(reason: EndReason, detail: impl Into<String>) -> Self {
        Self {
            reason,
            exit_code: None,
            left: Vec::new(),
            detail: clip(detail.into()),
            at_unix_ms: now_unix_ms(),
        }
    }
}

/// One pane's record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneRecord {
    /// [`RECORD_FORMAT`].
    pub format: u64,
    /// The pane's id, which is also the key its start is asked for by:
    /// starting it again with the same command finds this record.
    pub pane: String,
    /// The state folder that owns the pane, by the identity of the folder
    /// itself, so a copy of a state folder is not taken for its original.
    pub owner: String,
    /// This start among every start of this state folder, never reused, so a
    /// holder from an earlier start of the same pane is never taken for this
    /// one's.
    pub incarnation: u64,
    pub phase: Phase,
    /// The protocol version of the holder started.
    pub protocol: u16,
    /// The version folder its holder runs from (`v<version>-<hash>`).
    pub version_folder: String,
    /// A digest of the command, working folder and environment the start
    /// asked for; a start of the same pane with another one is a conflict.
    pub spec: String,
    pub reserved_at_unix_ms: u64,
    /// The boot the record was made in, where the system names one.
    pub boot: Option<String>,
    /// The holder, once it wrote itself in.
    pub holder: Option<Identity>,
    /// The pane's program, which leads the pane's terminal session, once
    /// the holder started it.
    pub program: Option<Identity>,
    pub end: Option<End>,
}

/// Why a record could not be read. None of these is "no record".
#[derive(Debug)]
pub enum Unreadable {
    Io(io::Error),
    TooLarge(u64),
    /// A format this build does not read.
    UnknownFormat(u64),
    Corrupt(String),
}

impl Unreadable {
    /// One word for a log field.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Io(_) => "unreadable",
            Self::TooLarge(_) => "too_large",
            Self::UnknownFormat(_) => "unknown_format",
            Self::Corrupt(_) => "corrupt",
        }
    }
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the record could not be read: {error}"),
            Self::TooLarge(length) => {
                write!(f, "the record is {length} bytes, more than {MAX_RECORD}")
            }
            Self::UnknownFormat(format) => write!(
                f,
                "the record is in format {format}, which this build does not read (it reads {RECORD_FORMAT})"
            ),
            Self::Corrupt(why) => write!(f, "the record is corrupt: {why}"),
        }
    }
}

/// Whether `pane` may name a pane: 1 to [`MAX_PANE`] ASCII letters, digits,
/// `-` and `_`, so it is a file name on every system.
pub fn check_pane(pane: &str) -> Result<(), String> {
    if pane.is_empty() || pane.len() > MAX_PANE {
        return Err(format!(
            "a pane id is 1 to {MAX_PANE} characters, not {}",
            pane.len()
        ));
    }
    if let Some(bad) = pane
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(format!("a pane id holds no {bad:?}"));
    }
    Ok(())
}

/// The record at `path`; `None` when there is no file.
pub fn read(path: &Path) -> Result<Option<PaneRecord>, Unreadable> {
    let bytes = match read_bounded(path) {
        Ok(bytes) => bytes,
        Err(Unreadable::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    parse(&bytes).map(Some)
}

/// A record from its bytes.
pub fn parse(bytes: &[u8]) -> Result<PaneRecord, Unreadable> {
    #[derive(Deserialize)]
    struct Head {
        format: u64,
    }
    let head: Head =
        serde_json::from_slice(bytes).map_err(|error| Unreadable::Corrupt(error.to_string()))?;
    if head.format != RECORD_FORMAT {
        return Err(Unreadable::UnknownFormat(head.format));
    }
    serde_json::from_slice(bytes).map_err(|error| Unreadable::Corrupt(error.to_string()))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, Unreadable> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(Unreadable::Io)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(Unreadable::Io)?;
    if bytes.len() > MAX_RECORD {
        let length = std::fs::metadata(path).map_or(bytes.len() as u64, |meta| meta.len());
        return Err(Unreadable::TooLarge(length));
    }
    Ok(bytes)
}

/// Replaces the record at `path` whole, private to this account. The caller
/// holds the records' lock.
pub fn write(path: &Path, record: &PaneRecord) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
    if bytes.len() > MAX_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "a record of {} bytes is more than {MAX_RECORD}",
                bytes.len()
            ),
        ));
    }
    hide_platform::fs::atomic::write_file(path, &bytes, Access::Private).map(|_| ())
}

/// Takes the records' lock at `path`, waiting up to `wait`; `None` when
/// someone held it all that time.
pub fn lock(path: &Path, wait: Duration) -> io::Result<Option<Lock>> {
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)?;
    }
    let file = hide_platform::fs::private::open_own_file(path, true)?;
    match lock_file(file, Mode::Exclusive, wait, &|| false)? {
        Waited::Locked(lock) => Ok(Some(lock)),
        Waited::TimedOut | Waited::Cancelled => Ok(None),
    }
}

/// Milliseconds since the Unix epoch, as a record keeps times.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// `detail` cut to [`crate::MAX_DETAIL`] bytes on a character boundary.
fn clip(mut detail: String) -> String {
    if detail.len() > crate::MAX_DETAIL {
        let mut end = crate::MAX_DETAIL;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> PaneRecord {
        PaneRecord {
            format: RECORD_FORMAT,
            pane: "p1".to_owned(),
            owner: "1:2".to_owned(),
            incarnation: 7,
            phase: Phase::Running,
            protocol: 1,
            version_folder: "v1-0123456789abcdef".to_owned(),
            spec: "s".to_owned(),
            reserved_at_unix_ms: 1,
            boot: Some("b".to_owned()),
            holder: Some(Identity {
                pid: 10,
                started: 11,
            }),
            program: Some(Identity {
                pid: 12,
                started: 13,
            }),
            end: Some(End {
                reason: EndReason::ProgramExited,
                exit_code: Some(3),
                left: vec![],
                detail: "the program exited".to_owned(),
                at_unix_ms: 2,
            }),
        }
    }

    #[test]
    fn a_record_reads_back_as_it_was_written() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("p1.json");
        assert!(read(&path).unwrap().is_none(), "no file is no record");
        write(&path, &record()).unwrap();
        assert_eq!(read(&path).unwrap(), Some(record()));
    }

    /// What a node must not take for "no record": each is its own reason.
    #[test]
    fn a_record_this_build_cannot_read_says_why() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("p1.json");
        std::fs::write(&path, br#"{"format":2,"anything":"else"}"#).unwrap();
        assert!(matches!(read(&path), Err(Unreadable::UnknownFormat(2))));
        std::fs::write(&path, b"{\"format\":1,\"pane\":").unwrap();
        assert!(matches!(read(&path), Err(Unreadable::Corrupt(_))));
        std::fs::write(&path, br#"{"format":1,"pane":"p1"}"#).unwrap();
        assert!(matches!(read(&path), Err(Unreadable::Corrupt(_))));
        std::fs::write(&path, vec![b' '; MAX_RECORD + 1]).unwrap();
        assert!(matches!(read(&path), Err(Unreadable::TooLarge(_))));
    }

    #[test]
    fn a_pane_id_is_a_file_name_on_every_system() {
        assert!(check_pane("pane-1_A").is_ok());
        for bad in [
            "",
            "a/b",
            "a\\b",
            "..",
            "a.json",
            "a b",
            &"x".repeat(MAX_PANE + 1),
        ] {
            assert!(check_pane(bad).is_err(), "{bad:?} was taken");
        }
    }

    /// The names a node's log and a holder's agree on.
    #[test]
    fn end_reasons_are_named_as_they_are_stored() {
        for reason in [
            EndReason::Terminated,
            EndReason::ProgramExited,
            EndReason::TreeEndUnconfirmed,
            EndReason::HolderFailed,
            EndReason::HolderLost,
            EndReason::Unrecorded,
            EndReason::MachineRestarted,
            EndReason::StartFailed,
            EndReason::StartUnconfirmed,
            EndReason::StartInterrupted,
        ] {
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::Value::from(reason.name())
            );
        }
    }
}
