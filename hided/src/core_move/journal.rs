//! `core-move.json`: the driver's record of a move, written before every
//! step that cannot be undone (PRD core-host-node-move B5, B6, amendment 1).
//!
//! A start that finds one past `Stopping` and not finished never starts a
//! core on its own: before `AttachSent` the move is rolled back (the other
//! machine's pending core is stopped first), from `AttachSent` the other
//! machine's handover decides, and from `Committed` the move only goes
//! forward.

use std::path::{Path, PathBuf};

use herdr_core::node_migration::{IdTable, OwnerChange};
use hide_platform::fs::atomic::DurableAccess;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const RECORD_CAP: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Journal {
    pub version: u32,
    pub intent: String,
    /// The other machine, as this one reaches it.
    pub peer: Peer,
    /// The copy's owner change and the ids it maps.
    pub change: OwnerChange,
    pub ids: IdTable,
    pub phase: Phase,
    /// By when, on this machine's clock, the other machine's pending core
    /// has surely given its copy back if no link took the move: the lease
    /// that machine answered from its start, or, with its answer lost, the
    /// longest that start could have been sent before plus a full lease. A
    /// driver that cannot reach it waits this out; none while no core was
    /// started there.
    #[serde(default)]
    pub target_lease_until_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// This machine's core goes to the peer.
    Forward,
    /// The peer's core comes back to this machine.
    Back,
}

/// The other machine of a move.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Peer {
    /// Its registration id in the core before the move.
    pub device: String,
    /// The SSH alias this machine reaches it by.
    pub alias: String,
    /// Its node id.
    pub node: String,
    /// The `hided` there that runs the move's steps and, after a forward
    /// move, the attach role.
    pub program: String,
    /// Its state folder, as it reported it.
    pub state_dir: String,
}

/// Where a move is, in the direction it goes; each direction has only its
/// own steps.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "direction")]
pub enum Phase {
    /// This machine's core goes to the peer.
    Forward(ForwardPhase),
    /// The peer's core comes back to this machine.
    Back(BackPhase),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum ForwardPhase {
    /// The core is stopping or stopped; nothing has left this machine.
    Stopping,
    /// The copy is on the peer and verified.
    Sent,
    /// The copy is placed on the peer under a pending handover.
    Placed,
    /// The peer's pending core started.
    TargetStarted,
    /// The link carrying the intent may have reached the peer: from here
    /// only the peer's handover says whether the move committed.
    AttachSent,
    /// The peer's core took the link: the move only goes forward.
    Committed,
    /// This machine's brain state is set aside and the peer's records are
    /// cleared.
    Done,
    /// The move was undone; this machine's core runs on its folder again.
    RolledBack {
        failed: MoveStep,
        cause: MoveFailure,
    },
    /// The operator gave the move up while the peer had not said whether
    /// it committed: this machine's core runs on its folder again, and the
    /// peer's core of the move is ended once this machine reaches it
    /// (`abandoned`).
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "phase")]
pub enum BackPhase {
    /// The peer's core may have been asked to stop for the move.
    Releasing,
    /// The peer's core stopped for the move and its copy is staged there;
    /// the peer starts no core of its own until it is resumed.
    Released,
    /// The copy is placed in this machine's folder.
    PlacedHere,
    /// The peer may be retiring its core, which is the commit; from here
    /// only the peer's handover says whether it is.
    Retiring,
    /// The peer's core retired and this machine's core runs.
    Done,
    /// The move was undone; the peer's core runs again and this machine is
    /// its node.
    RolledBack {
        failed: MoveStep,
        cause: MoveFailure,
    },
}

impl Phase {
    pub fn direction(&self) -> Direction {
        match self {
            Self::Forward(_) => Direction::Forward,
            Self::Back(_) => Direction::Back,
        }
    }

    pub fn is_rolled_back(&self) -> bool {
        matches!(
            self,
            Self::Forward(ForwardPhase::RolledBack { .. })
                | Self::Back(BackPhase::RolledBack { .. })
        )
    }

    /// Whether a start finding this phase must not start a core of its own
    /// before the move is resolved.
    pub fn holds_the_core(&self) -> bool {
        !self.is_rolled_back()
            && !matches!(
                self,
                Self::Forward(ForwardPhase::Done | ForwardPhase::Abandoned)
                    | Self::Back(BackPhase::Done)
            )
    }

    /// The step a move interrupted in this phase was at.
    pub fn step(&self) -> MoveStep {
        match self {
            Self::Forward(phase) => match phase {
                ForwardPhase::Stopping => MoveStep::StopCore,
                ForwardPhase::Sent | ForwardPhase::Placed => MoveStep::Copy,
                ForwardPhase::TargetStarted => MoveStep::StartTarget,
                ForwardPhase::AttachSent
                | ForwardPhase::Committed
                | ForwardPhase::Done
                | ForwardPhase::Abandoned => MoveStep::Reattach,
                ForwardPhase::RolledBack { failed, .. } => *failed,
            },
            Self::Back(phase) => match phase {
                BackPhase::Releasing => MoveStep::StopCore,
                BackPhase::Released | BackPhase::PlacedHere => MoveStep::Copy,
                BackPhase::Retiring => MoveStep::StartTarget,
                BackPhase::Done => MoveStep::Reattach,
                BackPhase::RolledBack { failed, .. } => *failed,
            },
        }
    }
}

/// The five steps the move's window names.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveStep {
    Check,
    StopCore,
    Copy,
    StartTarget,
    Reattach,
}

/// Why a move failed, typed so the window and the log say which step and
/// what to do without reading a message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MoveFailure {
    /// The peer could not be reached over SSH.
    Unreachable { reason: String },
    /// The core did not stop.
    StopUnconfirmed { reason: String },
    /// The copy could not be made or its owner changed: the named file.
    Staging { file: String, reason: String },
    /// The copy did not reach the peer whole.
    Copy { reason: String },
    /// The peer's copy still differs after it was sent again.
    Digest { files: Vec<String> },
    /// The peer's build cannot load the copy: the named file.
    Load { file: String, reason: String },
    /// The peer refused a step.
    Refused { step: String, reason: String },
    /// The peer cannot act on a step yet: a lock another change holds, or
    /// a core there still starting or not yet confirmed stopped.
    Busy { step: String, reason: String },
    /// The peer's `hided` is another build than this machine's, so its
    /// answers are not read.
    OtherBuild { build: String },
    /// The peer's core exited at its start or did not take links in time.
    NotStarted { reason: String },
    /// The peer's core refused this machine's first link.
    LinkRefused { reason: String },
    /// Something on this machine failed.
    Local { reason: String },
}

impl MoveFailure {
    /// Whether asking the peer again may answer: it could not be reached,
    /// or it said it cannot act yet. Every other failure answers the same
    /// when asked again.
    pub fn transient(&self) -> bool {
        matches!(self, Self::Unreachable { .. } | Self::Busy { .. })
    }
}

pub fn path(state_dir: &Path) -> PathBuf {
    hide_kit::layout::core_move_journal(state_dir)
}

/// The journal `state_dir` holds; `None` when no move was ever driven here.
pub fn read(state_dir: &Path) -> Result<Option<Journal>, String> {
    let path = path(state_dir);
    let bytes = match super::read_private(&path, RECORD_CAP) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let journal: Journal = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a move this build reads: {error}", path.display()))?;
    if journal.version != VERSION {
        return Err(format!(
            "{} has version {}, which this build does not read",
            path.display(),
            journal.version
        ));
    }
    Ok(Some(journal))
}

/// Records `journal`, flushed before this returns.
pub fn write(state_dir: &Path, journal: &Journal) -> Result<(), String> {
    let path = path(state_dir);
    let bytes = serde_json::to_vec_pretty(journal).map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file_durable(&path, &bytes, DurableAccess::Private)
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", path.display()))
}

impl Journal {
    /// A move starting at `phase`, whose direction it carries.
    pub fn new(
        intent: String,
        phase: Phase,
        peer: Peer,
        change: OwnerChange,
        ids: IdTable,
    ) -> Self {
        Self {
            version: VERSION,
            intent,
            peer,
            change,
            ids,
            phase,
            target_lease_until_unix_ms: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phase is written with its direction beside it, and a phase of one
    /// direction is not read as the other's.
    #[test]
    fn a_phase_reads_back_only_in_its_own_direction() {
        let phase = Phase::Back(BackPhase::RolledBack {
            failed: MoveStep::Copy,
            cause: MoveFailure::Local {
                reason: "no".to_owned(),
            },
        });
        let written = serde_json::to_value(&phase).unwrap();
        assert_eq!(written["direction"], "back");
        assert_eq!(written["phase"], "rolled_back");
        assert_eq!(serde_json::from_value::<Phase>(written).unwrap(), phase);
        assert!(
            serde_json::from_value::<Phase>(
                serde_json::json!({"direction": "forward", "phase": "retiring"})
            )
            .is_err()
        );
    }
}
