//! The holder's part in its pane's record (`hide_holder_protocol::record`):
//! it writes itself in before it starts the pane's program, the program and
//! `running` in one write once the program runs (its socket listens before
//! the program starts), and why the pane ended before it exits. Each change
//! is made only while the record still reserves this holder's start (its
//! incarnation, and the holder itself once it wrote itself in), so a holder
//! whose start the node gave up on, or one of an earlier start of the same
//! pane, never writes over the record.

use std::path::PathBuf;

use hide_holder_protocol::record::{self, End, Identity, PaneRecord, Phase};

/// Where the record is and which start this holder is.
#[derive(Clone)]
pub struct Claim {
    record: PathBuf,
    lock: PathBuf,
    incarnation: u64,
    holder: Identity,
}

impl Claim {
    /// Writes this holder into the record, which must be its start's and
    /// still preparing; anything else fails the start.
    pub fn take(record: PathBuf, lock: PathBuf, incarnation: u64) -> Result<Self, String> {
        let pid = std::process::id();
        let started = hide_platform::process::start_time(pid)
            .map_err(|error| format!("the holder's own start time could not be read: {error}"))?;
        let claim = Self {
            record,
            lock,
            incarnation,
            holder: Identity { pid, started },
        };
        claim.change(|record, holder| {
            if record.phase != Phase::Preparing {
                return Err(format!(
                    "the record is {:?}, not preparing this start",
                    record.phase
                ));
            }
            match record.holder {
                None => record.holder = Some(holder),
                Some(named) if named == holder => {}
                Some(named) => {
                    return Err(format!("the record names holder {} already", named.pid));
                }
            }
            Ok(true)
        })?;
        Ok(claim)
    }

    /// The start this holder is.
    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }

    /// The program runs and the socket listens: records the program, which
    /// leads its terminal session, so what it leaves can be found should
    /// this holder end without ending it, and the phase `running`, in one
    /// write. A program that already exited, and is not yet waited for,
    /// leaves nothing to find by its start time, and is not named.
    pub fn running(&self, pid: u32) -> Result<(), String> {
        let program = Identity::of(pid);
        self.change(|record, _| {
            expect_phase(record, Phase::Preparing)?;
            if program.is_some() {
                record.program = program;
            }
            record.phase = Phase::Running;
            Ok(true)
        })
    }

    /// Writes why the pane ended. A record that names another start, or is
    /// gone, is left as it is: this holder has no say over it.
    pub fn end(&self, end: End) -> Result<(), String> {
        self.change(|record, _| {
            if record.phase == Phase::Ended {
                return Ok(false);
            }
            record.finish(end);
            Ok(true)
        })
        .or_else(|error| {
            if error == NOT_MINE {
                Ok(())
            } else {
                Err(error)
            }
        })
    }

    /// Reads the record under its lock, checks it is this start's, applies
    /// `change`, and writes it when `change` says it changed.
    fn change(
        &self,
        change: impl FnOnce(&mut PaneRecord, Identity) -> Result<bool, String>,
    ) -> Result<(), String> {
        let _lock = record::lock(&self.lock, record::LOCK_WAIT)
            .map_err(|error| format!("the records' lock could not be taken: {error}"))?
            .ok_or_else(|| {
                format!(
                    "the records' lock was held for {} s",
                    record::LOCK_WAIT.as_secs()
                )
            })?;
        let mut current = match record::read(&self.record) {
            Ok(Some(current)) => current,
            Ok(None) => return Err(NOT_MINE.to_owned()),
            Err(error) => return Err(error.to_string()),
        };
        if current.incarnation != self.incarnation {
            return Err(NOT_MINE.to_owned());
        }
        if current.holder.is_some_and(|named| named != self.holder) {
            return Err(NOT_MINE.to_owned());
        }
        if change(&mut current, self.holder)? {
            record::write(&self.record, &current)
                .map_err(|error| format!("the record could not be written: {error}"))?;
        }
        Ok(())
    }
}

/// The holder's log event that says it ended but could not write its record,
/// with the end it meant to write and the `error`. It is for a person reading
/// the log: a node finds such a holder gone and settles its pane as lost.
pub const UNWRITTEN_EVENT: &str = "holder.record_unwritten";

/// What a change answers when the record is not this start's.
const NOT_MINE: &str = "the record no longer reserves this start";

fn expect_phase(record: &PaneRecord, phase: Phase) -> Result<(), String> {
    if record.phase == phase {
        Ok(())
    } else {
        Err(format!("the record is {:?}, not {phase:?}", record.phase))
    }
}
