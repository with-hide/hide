//! Engine input has a reply, unlike fire-and-forget screen keys. The same
//! state lock decides whether the writer or cancellation owns its bytes.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use hide_node_link::holders::HolderError;

use super::{input::HELD_INPUT_MAX_AGE, permit::LIMIT};

enum State {
    Queued,
    Writing,
    Finished(Result<(), HolderError>),
}

struct Pending {
    state: Mutex<State>,
    changed: Condvar,
    deadline: Instant,
    _work: Work,
}

/// Shares the node's existing outstanding-input cap, including zero-byte
/// commands and a letter's gap after its body has finished.
pub(super) struct Work(Arc<AtomicUsize>);
impl Work {
    pub(super) fn take(count: &Arc<AtomicUsize>) -> Result<Self, HolderError> {
        count
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < LIMIT).then_some(n + 1)
            })
            .map_err(|_| {
                refused(
                    "pane_input_limit",
                    "Too many pane inputs are awaiting delivery",
                )
            })?;
        Ok(Self(Arc::clone(count)))
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub(super) struct Receipt(Arc<Pending>);

impl PartialEq for Receipt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Receipt {}

impl Receipt {
    pub(super) fn take(count: &Arc<AtomicUsize>) -> Result<(Self, Command), HolderError> {
        let work = Work::take(count)?;
        let receipt = Self(Arc::new(Pending {
            state: Mutex::new(State::Queued),
            changed: Condvar::new(),
            deadline: Instant::now() + HELD_INPUT_MAX_AGE,
            _work: work,
        }));
        Ok((receipt.clone(), Command(Arc::new(Delivery(receipt)))))
    }

    pub(super) fn cancel(&self, reason: &str) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        match *state {
            State::Queued => *state = State::Finished(Err(refused("pane_input_cancelled", reason))),
            State::Writing => {
                *state = State::Finished(Err(HolderError::Unreachable {
                    message: format!(
                        "{reason}; the write started and its result is unknown; do not resend automatically"
                    ),
                }))
            }
            State::Finished(_) => return,
        }
        self.0.changed.notify_all();
    }

    pub(super) fn finished(&self) -> bool {
        matches!(
            *self.0.state.lock().unwrap_or_else(|e| e.into_inner()),
            State::Finished(_)
        )
    }

    pub(super) fn wait(&self) -> Result<(), HolderError> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let State::Finished(result) = &*state {
                return result.clone();
            }
            let left = self.0.deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                drop(state);
                self.cancel("Pane input delivery exceeded its deadline");
                state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            } else {
                state = self
                    .0
                    .changed
                    .wait_timeout(state, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        }
    }
}

struct Delivery(Receipt);
impl Drop for Delivery {
    fn drop(&mut self) {
        self.0
            .cancel("Pane input was discarded before delivery completed");
    }
}

#[derive(Clone)]
pub(super) struct Command(Arc<Delivery>);
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EngineInput")
    }
}
impl PartialEq for Command {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Command {}

impl Command {
    /// Cancellation and write-start compete on this lock. Once writing,
    /// timeout is unknown, never a claim that nothing was sent.
    pub(super) fn begin(&self) -> bool {
        let pending = &(self.0.0).0;
        let mut state = pending.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, State::Queued) {
            return false;
        }
        if Instant::now() >= pending.deadline {
            *state = State::Finished(Err(refused(
                "pane_input_expired",
                "Pane input expired before writing",
            )));
            pending.changed.notify_all();
            return false;
        }
        *state = State::Writing;
        true
    }

    pub(super) fn finish(&self, result: std::io::Result<()>) {
        let pending = &(self.0.0).0;
        let mut state = pending.state.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*state, State::Writing) {
            return;
        }
        *state = State::Finished(result.map_err(|error| HolderError::Unreachable {
            message: format!(
                "Pane input write failed and may be partial: {error}; do not resend automatically"
            ),
        }));
        pending.changed.notify_all();
    }

    pub(super) fn refused(&self, reason: &str) {
        self.0.0.cancel(reason);
    }

    pub(super) fn finished(&self) -> bool {
        self.0.0.finished()
    }
}

pub(super) fn refused(code: &str, message: &str) -> HolderError {
    HolderError::Refused {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_queued_command_can_never_start_writing() {
        let (receipt, command) = Receipt::take(&Arc::default()).unwrap();
        receipt.cancel("pane closed");
        assert!(matches!(receipt.wait(), Err(HolderError::Refused { .. })));
        assert!(!command.begin());
    }

    #[test]
    fn a_cancelled_live_write_is_unknown_and_retains_its_work_slot() {
        let count = Arc::default();
        let (receipt, command) = Receipt::take(&count).unwrap();
        assert!(command.begin());
        receipt.cancel("pane closed");
        assert!(matches!(
            receipt.wait(),
            Err(HolderError::Unreachable { .. })
        ));
        drop(receipt);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        // A late completion cannot replace the outcome already answered.
        command.finish(Ok(()));
        assert!(matches!(
            command.0.0.wait(),
            Err(HolderError::Unreachable { .. })
        ));
        drop(command);
        assert_eq!(count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn outstanding_commands_have_a_count_cap_even_without_any_bytes() {
        let count = Arc::default();
        let pending: Vec<_> = (0..64).map(|_| Receipt::take(&count).unwrap()).collect();
        assert!(matches!(
            Receipt::take(&count),
            Err(HolderError::Refused { .. })
        ));
        drop(pending);
        assert_eq!(count.load(Ordering::Relaxed), 0);
        assert!(Receipt::take(&count).is_ok());
    }
}
