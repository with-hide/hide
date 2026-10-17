//! `core-move-abandoned.json`: the moves whose wait on their other machine
//! the operator gave up (PRD core-host-node-move amendment 4), with the step
//! that machine still owes: a forward move given up before its peer said
//! whether it committed is ended there, and a move back taken here before
//! the core's machine retired is retired there. Each is kept until that
//! machine answers that the step is done, and this machine's supervisor
//! asks it whatever role runs, so a later move neither carries nor migrates
//! the record (`node_migration::UNBOUND`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hide_platform::fs::atomic::DurableAccess;
use serde::{Deserialize, Serialize};

use super::journal::Peer;

const VERSION: u32 = 1;
/// The most moves given up and not yet ended on their machine; another
/// abandon past it is refused.
pub const LIMIT: usize = 8;
const RECORD_CAP: u64 = 64 * 1024;

/// A move whose wait was given up here, the machine its core may still
/// run on or be held on, and what that machine still owes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Abandoned {
    pub intent: String,
    pub peer: Peer,
    pub step: PeerStep,
}

/// The step the other machine of a given-up move still owes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerStep {
    /// A forward move given up: a core of the move there is stopped and set
    /// aside, and its starter, handover and copy go (`hided core-move
    /// abandon`).
    Abandon,
    /// A move back taken here: the core held there for it is retired
    /// (`hided core-move retire`).
    Retire,
}

#[derive(Debug, Deserialize, Serialize)]
struct Record {
    version: u32,
    moves: Vec<Abandoned>,
}

/// One change at a time in this process, each reading the record as the
/// last one left it; only this process's supervisor writes it.
static CHANGES: Mutex<()> = Mutex::new(());

fn path(state_dir: &Path) -> PathBuf {
    hide_kit::layout::core_move_abandoned(state_dir)
}

/// The moves given up and not yet ended on their machine.
pub fn read(state_dir: &Path) -> Result<Vec<Abandoned>, String> {
    let path = path(state_dir);
    let bytes = match super::read_private(&path, RECORD_CAP) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let record: Record = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{} is not a record this build reads: {error}",
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
    Ok(record.moves)
}

fn write(state_dir: &Path, moves: Vec<Abandoned>) -> Result<(), String> {
    let path = path(state_dir);
    if moves.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        };
    }
    let bytes = serde_json::to_vec_pretty(&Record {
        version: VERSION,
        moves,
    })
    .map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file_durable(&path, &bytes, DurableAccess::Private)
        .map(|_| ())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn change(
    state_dir: &Path,
    change: impl FnOnce(&mut Vec<Abandoned>) -> Result<(), String>,
) -> Result<(), String> {
    let _changing = CHANGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut moves = read(state_dir)?;
    let before = moves.clone();
    change(&mut moves)?;
    if moves != before {
        write(state_dir, moves)?;
    }
    Ok(())
}

/// Whether another move can be given up.
pub fn has_room(state_dir: &Path) -> Result<bool, String> {
    Ok(read(state_dir)?.len() < LIMIT)
}

/// Records `abandoned`, flushed before this returns; recording it again
/// changes nothing, and one past [`LIMIT`] is refused.
pub fn add(state_dir: &Path, abandoned: Abandoned) -> Result<(), String> {
    change(state_dir, |moves| {
        if moves.iter().any(|kept| kept.intent == abandoned.intent) {
            return Ok(());
        }
        if moves.len() >= LIMIT {
            return Err(format!(
                "{LIMIT} moves given up have not been ended on their machines"
            ));
        }
        moves.push(abandoned);
        Ok(())
    })
}

/// Drops `intent` once its machine has no core of it.
pub fn remove(state_dir: &Path, intent: &str) -> Result<(), String> {
    change(state_dir, |moves| {
        moves.retain(|kept| kept.intent != intent);
        Ok(())
    })
}

/// Records the `hided` that runs `intent`'s steps there, after this
/// machine installed its own build on it.
pub fn set_program(state_dir: &Path, intent: &str, program: &str) -> Result<(), String> {
    change(state_dir, |moves| {
        for kept in moves.iter_mut().filter(|kept| kept.intent == intent) {
            kept.peer.program = program.to_owned();
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abandoned(intent: &str) -> Abandoned {
        Abandoned {
            intent: intent.to_owned(),
            peer: Peer {
                device: "mini".to_owned(),
                alias: "mini".to_owned(),
                node: "node-c".to_owned(),
                program: "/c/hided".to_owned(),
                state_dir: "/c/state".to_owned(),
            },
            step: PeerStep::Abandon,
        }
    }

    /// A move is recorded once whatever the retries, the record holds at
    /// most `LIMIT`, and it is gone with its last move.
    #[test]
    fn the_record_holds_each_move_once_and_at_most_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..LIMIT {
            add(dir.path(), abandoned(&format!("move-{index}"))).unwrap();
        }
        add(dir.path(), abandoned("move-0")).unwrap();
        assert_eq!(read(dir.path()).unwrap().len(), LIMIT);
        assert!(!has_room(dir.path()).unwrap());
        assert!(add(dir.path(), abandoned("move-over")).is_err());
        set_program(dir.path(), "move-1", "/c/v2/hided").unwrap();
        assert_eq!(read(dir.path()).unwrap()[1].peer.program, "/c/v2/hided");
        for index in 0..LIMIT {
            remove(dir.path(), &format!("move-{index}")).unwrap();
        }
        assert!(!path(dir.path()).exists());
        assert!(read(dir.path()).unwrap().is_empty());
    }
}
