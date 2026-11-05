//! The observe passes running now (`Runtime::observe_delivery` to the end of
//! its sync pass), which a pull at an agent's tool event or turn end waits
//! on. Such a pull is answered by the turn the core laid on the pane
//! (`Observation::projected_turn`), and a pass that observed the pane lays
//! that turn only at its own ingest, after work outside the runtime's lock;
//! a pull that arrived in between found no turn and handed nothing over,
//! although the turn the pass was about to lay was still the agent's. So a
//! pull that finds the pane's turn not yet laid by a pass still running
//! waits for that pass to end, outside the runtime's lock and off the
//! session-sync thread, and reads the pane again; one whose pass is not
//! running is answered at once.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Instant;

/// The most pulls waiting on passes at once; one more is answered from what
/// the core holds now, as when no pass runs.
pub(crate) const WAITING_LIMIT: usize = 16;

#[derive(Default)]
pub(crate) struct Passes {
    state: Mutex<State>,
    ended: Condvar,
}

#[derive(Default)]
struct State {
    running: BTreeSet<u64>,
    waiting: usize,
}

/// How a wait on a pass ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waited {
    /// The pass ended, or was not running.
    Ended,
    /// The request's deadline came first.
    Deadline,
    /// [`WAITING_LIMIT`] pulls were waiting already.
    Capacity,
}

impl Waited {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Ended => "ended",
            Self::Deadline => "deadline",
            Self::Capacity => "capacity",
        }
    }
}

impl Passes {
    /// Marks pass `generation` running until the returned guard drops, which
    /// its sync pass holds to its end on every path.
    pub(crate) fn begin(self: &Arc<Self>, generation: u64) -> Running {
        self.lock().running.insert(generation);
        Running {
            passes: Arc::clone(self),
            generation,
        }
    }

    pub(crate) fn running(&self, generation: u64) -> bool {
        self.lock().running.contains(&generation)
    }

    /// Waits until pass `generation` ends or `deadline` comes.
    pub(crate) fn wait(&self, generation: u64, deadline: Instant) -> Waited {
        let mut state = self.lock();
        if !state.running.contains(&generation) {
            return Waited::Ended;
        }
        if state.waiting >= WAITING_LIMIT {
            return Waited::Capacity;
        }
        state.waiting += 1;
        let waited = loop {
            if !state.running.contains(&generation) {
                break Waited::Ended;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break Waited::Deadline;
            };
            state = self
                .ended
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        state.waiting -= 1;
        waited
    }

    /// Takes `count` places among the waiting pulls, as pulls still waiting.
    #[cfg(test)]
    pub(crate) fn hold_waiting(&self, count: usize) {
        self.lock().waiting += count;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A pass running; dropping it ends the pass and wakes the pulls waiting on
/// it.
#[must_use = "the pass ends when this drops"]
pub(crate) struct Running {
    passes: Arc<Passes>,
    generation: u64,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.passes.lock().running.remove(&self.generation);
        self.passes.ended.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_pass_not_running_is_no_wait_and_one_running_is_waited_to_its_end() {
        let passes = Arc::new(Passes::default());
        assert_eq!(
            passes.wait(1, Instant::now() + Duration::from_secs(60)),
            Waited::Ended
        );
        let running = passes.begin(2);
        let waiter = {
            let passes = Arc::clone(&passes);
            std::thread::spawn(move || passes.wait(2, Instant::now() + Duration::from_secs(60)))
        };
        drop(running);
        assert_eq!(waiter.join().unwrap(), Waited::Ended);
        assert!(!passes.running(2));
    }

    #[test]
    fn a_wait_ends_at_its_deadline_and_past_the_limit_at_once() {
        let passes = Arc::new(Passes::default());
        let _running = passes.begin(3);
        assert_eq!(passes.wait(3, Instant::now()), Waited::Deadline);
        passes.lock().waiting = WAITING_LIMIT;
        assert_eq!(
            passes.wait(3, Instant::now() + Duration::from_secs(60)),
            Waited::Capacity
        );
        // A wait that ended gives its place back.
        passes.lock().waiting = WAITING_LIMIT - 1;
        assert_eq!(passes.wait(3, Instant::now()), Waited::Deadline);
        assert_eq!(passes.lock().waiting, WAITING_LIMIT - 1);
    }
}
