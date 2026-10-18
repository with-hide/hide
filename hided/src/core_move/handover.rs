//! `core-handover.json`: what a machine taking or giving the core in a move
//! is doing (PRD core-host-node-move B5, amendments 1 to 3). Written by
//! `hided core-move` and by the core it starts, each change under the
//! record's lock, so the core's activation and the driver's abort can never
//! both win.
//!
//! On the machine taking the core: `Pending` from the moment the copy is
//! placed until the first link that carries the move's intent, then
//! `Active`. On the machine giving it back in a move back: `StoppedFor`
//! once its core stopped for the intent, then `Retired` once its login item
//! and brain state are gone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_platform::fs::atomic::DurableAccess;
use hide_platform::fs::lock::{Lock, Mode, Waited};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
/// The largest record read.
const RECORD_CAP: u64 = 16 * 1024;
/// How long a change waits for the record's lock.
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// How long a started pending core waits at least for the link that
/// carries its move.
pub const PENDING_LEASE: Duration = Duration::from_secs(60);
/// How long a place's lease runs: the start that follows has
/// [`STARTED_WITHIN`](super::starter::STARTED_WITHIN), and the core it
/// started still waits [`PENDING_LEASE`] whatever the start took.
pub const PLACED_LEASE: Duration = super::starter::STARTED_WITHIN.saturating_add(PENDING_LEASE);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Handover {
    pub version: u32,
    pub intent: String,
    /// The node giving the core.
    pub source: String,
    /// The node taking it.
    pub target: String,
    pub state: HandoverState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum HandoverState {
    /// The copy is placed; the core started on it holds every effect until
    /// the link carrying the intent arrives, and gives the copy back when
    /// none has by `lease_until_unix_ms` on this machine's clock, a
    /// deadline no request moves.
    Pending { lease_until_unix_ms: u64 },
    /// The link carrying the intent was taken: the move is committed.
    Active,
    /// The core here stopped for a move back and holds its state for the
    /// driver to pull.
    StoppedFor,
    /// The core left this machine in a move back: its starter and brain
    /// state are gone.
    Retired,
}

impl HandoverState {
    /// A pending handover whose lease runs [`PLACED_LEASE`] from now.
    pub fn pending_from_now() -> Self {
        Self::Pending {
            lease_until_unix_ms: now_unix_ms() + PLACED_LEASE.as_millis() as u64,
        }
    }

    pub fn is_pending(self) -> bool {
        matches!(self, Self::Pending { .. })
    }
}

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

impl Handover {
    pub fn new(intent: &str, source: &str, target: &str, state: HandoverState) -> Self {
        Self {
            version: VERSION,
            intent: intent.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            state,
        }
    }
}

fn path(state_dir: &Path) -> PathBuf {
    hide_kit::layout::core_handover(state_dir)
}

/// The record `state_dir` holds; `None` when no move touches it.
pub fn read(state_dir: &Path) -> Result<Option<Handover>, String> {
    let path = path(state_dir);
    let bytes = match crate::core_move::read_private(&path, RECORD_CAP) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let record: Handover = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{} is not a handover this build reads: {error}",
            path.display()
        )
    })?;
    if record.version != VERSION {
        return Err(format!(
            "{} has version {}, which this build does not read",
            path.display(),
            record.version
        ));
    }
    Ok(Some(record))
}

/// Holds the record's lock for one change.
pub struct Held {
    state_dir: PathBuf,
    _lock: Lock,
}

impl Held {
    pub fn read(&self) -> Result<Option<Handover>, String> {
        read(&self.state_dir)
    }

    pub fn write(&self, record: &Handover) -> Result<(), String> {
        let path = path(&self.state_dir);
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?;
        hide_platform::fs::atomic::write_file_durable(&path, &bytes, DurableAccess::Private)
            .map(|_| ())
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn remove(&self) -> Result<(), String> {
        let path = path(&self.state_dir);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }
}

/// Why the record's lock was not taken.
#[derive(Debug)]
pub enum HoldError {
    /// Another change held it past the wait: a later try may take it.
    Busy(String),
    Failed(String),
}

impl std::fmt::Display for HoldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(reason) | Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

impl From<HoldError> for String {
    fn from(error: HoldError) -> Self {
        error.to_string()
    }
}

/// Takes the record's lock (a file beside it), waiting a few seconds for
/// another change to finish.
pub fn hold(state_dir: &Path) -> Result<Held, HoldError> {
    let lock_path = state_dir.join("core-handover.lock");
    let failed =
        |error: std::io::Error| HoldError::Failed(format!("{}: {error}", lock_path.display()));
    let file = hide_platform::fs::private::open_or_create_file(&lock_path).map_err(failed)?;
    match hide_platform::fs::lock::lock_file(file, Mode::Exclusive, LOCK_WAIT, &|| false)
        .map_err(failed)?
    {
        Waited::Locked(lock) => Ok(Held {
            state_dir: state_dir.to_path_buf(),
            _lock: lock,
        }),
        Waited::TimedOut | Waited::Cancelled => Err(HoldError::Busy(format!(
            "{} is held by another change",
            lock_path.display()
        ))),
    }
}

/// What a link that carries `intent` finds: a pending core is activated by
/// it, an active one is already committed to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activation {
    /// This link made the move commit.
    Activated,
    /// The move had committed on an earlier link.
    AlreadyActive,
    /// The record names another intent, or no move: the link is refused.
    NotThisMove,
}

/// Activates the pending core of `state_dir` for `intent`, linked by
/// `node`, under the record's lock: only the node the move came from
/// commits it.
pub fn activate(state_dir: &Path, intent: &str, node: &str) -> Result<Activation, String> {
    let held = hold(state_dir)?;
    let Some(mut record) = held.read()? else {
        return Ok(Activation::NotThisMove);
    };
    if record.intent != intent || record.source != node {
        return Ok(Activation::NotThisMove);
    }
    match record.state {
        HandoverState::Pending { .. } => {
            record.state = HandoverState::Active;
            held.write(&record)?;
            Ok(Activation::Activated)
        }
        HandoverState::Active => Ok(Activation::AlreadyActive),
        HandoverState::StoppedFor | HandoverState::Retired => Ok(Activation::NotThisMove),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_move_s_own_link_activates_a_pending_core_and_only_once() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            activate(dir.path(), "i1", "m").unwrap(),
            Activation::NotThisMove
        );
        hold(dir.path())
            .unwrap()
            .write(&Handover::new(
                "i1",
                "m",
                "c",
                HandoverState::pending_from_now(),
            ))
            .unwrap();
        assert_eq!(
            activate(dir.path(), "i2", "m").unwrap(),
            Activation::NotThisMove
        );
        // Another node naming the move's intent does not commit it.
        assert_eq!(
            activate(dir.path(), "i1", "other").unwrap(),
            Activation::NotThisMove
        );
        assert_eq!(
            activate(dir.path(), "i1", "m").unwrap(),
            Activation::Activated
        );
        assert_eq!(
            activate(dir.path(), "i1", "m").unwrap(),
            Activation::AlreadyActive
        );
        assert_eq!(
            read(dir.path()).unwrap().unwrap().state,
            HandoverState::Active
        );
    }
}
