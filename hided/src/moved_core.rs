//! What a core moved to another machine before 1.2.0 left behind (PRD
//! device-connect-iroh D-35, D-46). No build moves a core now: the machine
//! that took one keeps it as its own, and the machine that gave it runs the
//! node role from its placement record until the two are split.
//!
//! A move wrote two records while it ran: the driver's journal
//! (`core-move.json`), which stays after the move with the phase it ended
//! in, and the handover (`core-handover.json`) on the other machine, which
//! the move's last step removed. A folder whose journal names a phase that
//! is not an end, or that still holds a handover, was left in the middle of
//! a move, and only a build that still moves cores can finish or undo it.

use std::path::Path;

use serde::Deserialize;

/// The largest record read: a journal carries the id table of the copy.
const RECORD_CAP: u64 = 4 * 1024 * 1024;
/// The last release whose start finishes or undoes a move.
const LAST_MOVING_RELEASE: &str = "1.2.10";

/// The part of the journal that says whether its move ended.
#[derive(Deserialize)]
struct Journal {
    intent: String,
    phase: Phase,
}

#[derive(Deserialize)]
struct Phase {
    direction: String,
    phase: String,
}

impl Phase {
    /// The phases a move ends in: done, undone, or given up.
    fn ended(&self) -> bool {
        matches!(
            (self.direction.as_str(), self.phase.as_str()),
            ("forward", "done" | "rolled_back" | "abandoned") | ("back", "done" | "rolled_back")
        )
    }
}

/// The part of the handover that names its move.
#[derive(Deserialize)]
struct Handover {
    intent: String,
}

/// Why `state_dir` starts no role: a move from before 1.2.0 was left
/// unfinished in it. `None` for every other folder. A record that cannot be
/// read is an error, since nothing says how its move ended.
pub fn unfinished(state_dir: &Path) -> Result<Option<String>, String> {
    let journal: Option<Journal> = read(&hide_kit::layout::core_move_journal(state_dir))?;
    let handover: Option<Handover> = read(&hide_kit::layout::core_handover(state_dir))?;
    let intent = match (journal, handover) {
        (Some(journal), _) if !journal.phase.ended() => journal.intent,
        (_, Some(handover)) => handover.intent,
        _ => return Ok(None),
    };
    Ok(Some(format!(
        "a core move from before Hide 1.2.0 ({intent}) was left unfinished in {}, and this build moves no core: open Hide {LAST_MOVING_RELEASE} on both machines once, which finishes or undoes it, then update again",
        state_dir.display()
    )))
}

/// The account's own private record at `path`, or `None` when there is
/// none.
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    use std::io::Read;
    let refused = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(refused(&error)),
    };
    let mut bytes = Vec::new();
    file.take(RECORD_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| refused(&error))?;
    if bytes.len() as u64 > RECORD_CAP {
        return Err(refused(&"the record is too large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| refused(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private(path: &Path, text: &str) {
        hide_platform::fs::atomic::write_file(
            path,
            text.as_bytes(),
            hide_platform::fs::Access::Private,
        )
        .unwrap();
    }

    fn journal(dir: &Path, phase: &str) {
        private(
            &hide_kit::layout::core_move_journal(dir),
            &format!(
                r#"{{"version":1,"intent":"move-1","peer":{{"device":"mini","alias":"mini","node":"c","program":"/x/hided","state_dir":"/s"}},"change":{{}},"ids":{{}},"phase":{phase}}}"#
            ),
        );
    }

    /// A folder with no record of a move, and one whose move ended (done,
    /// undone or given up, either way), starts as any folder does.
    #[test]
    fn a_folder_whose_move_ended_starts() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unfinished(dir.path()), Ok(None));
        for phase in [
            r#"{"direction":"forward","phase":"done"}"#,
            r#"{"direction":"forward","phase":"abandoned"}"#,
            r#"{"direction":"forward","phase":"rolled_back","failed":"copy","cause":{"kind":"local","reason":"x"}}"#,
            r#"{"direction":"back","phase":"done"}"#,
            r#"{"direction":"back","phase":"rolled_back","failed":"copy","cause":{"kind":"local","reason":"x"}}"#,
        ] {
            journal(dir.path(), phase);
            assert_eq!(unfinished(dir.path()), Ok(None), "{phase}");
        }
    }

    /// A journal stopped between two steps, and a handover the move's last
    /// step never removed, each refuse the start and name the move and the
    /// release that can finish it.
    #[test]
    fn a_folder_left_in_the_middle_of_a_move_starts_nothing_and_says_what_finishes_it() {
        for phase in [
            r#"{"direction":"forward","phase":"attach_sent"}"#,
            r#"{"direction":"forward","phase":"committed"}"#,
            r#"{"direction":"back","phase":"retiring"}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            journal(dir.path(), phase);
            let reason = unfinished(dir.path()).unwrap().expect(phase);
            assert!(reason.contains("move-1") && reason.contains("1.2.10"), "{reason}");
        }
        let dir = tempfile::tempdir().unwrap();
        journal(dir.path(), r#"{"direction":"forward","phase":"done"}"#);
        private(
            &hide_kit::layout::core_handover(dir.path()),
            r#"{"version":1,"intent":"move-2","source":"m","target":"c","state":{"state":"active"}}"#,
        );
        let reason = unfinished(dir.path()).unwrap().unwrap();
        assert!(reason.contains("move-2"), "{reason}");
    }

    /// A record that is not one this build can read says nothing of how its
    /// move ended, so the start stops on it.
    #[test]
    fn a_record_that_cannot_be_read_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        private(&hide_kit::layout::core_move_journal(dir.path()), "{");
        assert!(unfinished(dir.path()).is_err());
    }
}
