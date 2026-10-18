//! Who may put bytes into a pane's terminal while its agent is being put to
//! sleep, asleep, or waking (PRD agent-sleep).
//!
//! The shell under a sleeping agent is not what the operator was talking to,
//! so a reply, a key or a pasted path that lands there runs as a command. The
//! core owns the sleep records, so the core owns the answer, and every writer
//! that does not go through a node's terminal (a phone's reply, the agent's
//! find keys) asks here at the write, never from a copy of the list it read
//! earlier: an admission is taken immediately before the write and held until
//! it returns, and the sleep worker waits for the admissions already held
//! before it ends the agent. A write admitted before the pane closed therefore
//! lands in the agent, and one that comes after is refused; none lands between.
//!
//! The runtime only flips a flag here (never under an admission's wait), and
//! the worker that ends the agent waits off the runtime's lock.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// The pane takes no input: its agent is ending, asleep, waking or failed to wake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Closed;

#[derive(Debug, Default)]
struct State {
    closed: HashSet<String>,
    writing: HashMap<String, usize>,
}

/// The panes closed to input and the writes in flight to the others.
#[derive(Debug, Default)]
pub struct PaneInputGate {
    state: Mutex<State>,
    drained: Condvar,
}

/// One write admitted to a pane; the gate counts it until it drops.
#[derive(Debug)]
pub struct Admission {
    gate: Arc<PaneInputGate>,
    pane: String,
}

impl Drop for Admission {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        if let Some(count) = state.writing.get_mut(&self.pane) {
            *count -= 1;
            if *count == 0 {
                state.writing.remove(&self.pane);
            }
        }
        drop(state);
        self.gate.drained.notify_all();
    }
}

impl PaneInputGate {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Admits one write to `pane`, or refuses it while the pane is closed.
    /// Hold the admission across the write and drop it after.
    pub fn admit(self: &Arc<Self>, pane: &str) -> Result<Admission, Closed> {
        let mut state = self.lock();
        if state.closed.contains(pane) {
            return Err(Closed);
        }
        *state.writing.entry(pane.to_owned()).or_default() += 1;
        Ok(Admission {
            gate: Arc::clone(self),
            pane: pane.to_owned(),
        })
    }

    /// Closes one pane at once; writes already admitted finish.
    pub fn close(&self, pane: &str) {
        self.lock().closed.insert(pane.to_owned());
    }

    /// Makes `panes` exactly the closed ones, the runtime's answer from its
    /// sleep records. A pane that opens again takes input from here on.
    pub fn set_closed(&self, panes: &HashSet<String>) {
        let mut state = self.lock();
        if state.closed != *panes {
            state.closed.clone_from(panes);
        }
    }

    /// Waits until no admitted write to `pane` is still running, or `limit`
    /// passes (a write has its own timeout, so a longer wait would be a hang).
    /// Close the pane first, or a new write may be admitted behind the wait.
    pub fn wait_drained(&self, pane: &str, limit: Duration) {
        let state = self.lock();
        let _ = self
            .drained
            .wait_timeout_while(state, limit, |state| state.writing.contains_key(pane));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    use super::*;

    #[test]
    fn a_closed_pane_refuses_and_an_open_one_admits() {
        let gate = Arc::new(PaneInputGate::default());
        assert!(gate.admit("w1:p1").is_ok());
        gate.close("w1:p1");
        assert_eq!(gate.admit("w1:p1").err(), Some(Closed));
        assert!(gate.admit("w1:p2").is_ok(), "another pane is not closed");
        gate.set_closed(&HashSet::new());
        assert!(
            gate.admit("w1:p1").is_ok(),
            "an agent that is back takes input"
        );
    }

    /// The race the gate exists for: a write admitted just before the pane
    /// closed is still on its way when the worker would end the agent. The
    /// worker waits for it, so the bytes land in the agent and not behind it,
    /// and a write that arrives after the close never starts.
    #[test]
    fn the_end_waits_for_a_write_admitted_before_the_close() {
        let gate = Arc::new(PaneInputGate::default());
        let admission = gate.admit("w1:p1").expect("awake");
        gate.close("w1:p1");
        let ended = Arc::new(AtomicBool::new(false));
        let worker = {
            let (gate, ended) = (Arc::clone(&gate), Arc::clone(&ended));
            thread::spawn(move || {
                gate.wait_drained("w1:p1", Duration::from_secs(30));
                ended.store(true, Ordering::SeqCst);
            })
        };
        // A late write is refused while the first still runs.
        assert_eq!(gate.admit("w1:p1").err(), Some(Closed));
        assert!(
            !ended.load(Ordering::SeqCst),
            "the worker waits for the write in flight"
        );
        drop(admission);
        worker.join().expect("worker");
        assert!(ended.load(Ordering::SeqCst));
    }

    #[test]
    fn an_idle_pane_is_drained_at_once() {
        let gate = Arc::new(PaneInputGate::default());
        gate.close("w1:p1");
        gate.wait_drained("w1:p1", Duration::from_secs(30));
    }
}
