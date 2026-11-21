//! Whether this core may act outside its own machine yet (PRD
//! core-host-node-move amendment 3).
//!
//! A core started on a move's copy is pending until the link that carries
//! the move's intent commits it. Until then it may be rolled back, and the
//! core it replaces starts again on the folder the copy came from, so
//! anything this one did outside would be done twice or lost. While held,
//! the core asks no provider, rings no doorbell, raises no watch warning or
//! human notice, reads nothing from GitHub, runs no Factory, dials no
//! device and puts no agent to sleep; session sync, the catalog, the attach
//! role and reads that change nothing run, so the link can be proven.
//!
//! The hold is released once and never taken again. A worker with a thread
//! of its own waits on it ([`EffectHold::wait_released`]); a decision made
//! under the runtime mutex reads it ([`EffectHold::held`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// How often a waiting worker looks at its own stop flag.
const STOP_CHECK: Duration = Duration::from_millis(250);

pub struct EffectHold {
    held: Mutex<bool>,
    released: Condvar,
}

impl EffectHold {
    pub fn new(held: bool) -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(held),
            released: Condvar::new(),
        })
    }

    pub fn held(&self) -> bool {
        *self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lets every held effect run. Returns whether this call released it.
    pub(crate) fn release(&self) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let released = std::mem::replace(&mut *held, false);
        drop(held);
        self.released.notify_all();
        released
    }

    /// Blocks until the hold is released or `stop` is set. Returns whether
    /// the effects may run.
    pub(crate) fn wait_released(&self, stop: &AtomicBool) -> bool {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        while *held {
            if stop.load(Ordering::Acquire) {
                return false;
            }
            held = self
                .released
                .wait_timeout(held, STOP_CHECK)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        !stop.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_worker_runs_once_the_hold_is_released_and_stops_with_its_owner() {
        let hold = EffectHold::new(true);
        let stop = Arc::new(AtomicBool::new(false));
        let waiter = {
            let (hold, stop) = (Arc::clone(&hold), Arc::clone(&stop));
            std::thread::spawn(move || hold.wait_released(&stop))
        };
        assert!(hold.held());
        assert!(hold.release());
        assert!(!hold.release(), "the hold is released once");
        assert!(waiter.join().unwrap());
        assert!(!hold.held());

        let held = EffectHold::new(true);
        let stopped = {
            let (held, stop) = (Arc::clone(&held), Arc::clone(&stop));
            std::thread::spawn(move || held.wait_released(&stop))
        };
        stop.store(true, Ordering::Release);
        assert!(
            !stopped.join().unwrap(),
            "a stopped owner never runs held work"
        );
        assert!(held.held());
    }
}
