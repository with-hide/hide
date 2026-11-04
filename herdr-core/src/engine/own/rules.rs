//! The thread that gives panes' holders their screen rules (PRD
//! engine-bundle-3-core-owns D-18): the engine decides which agent's rules a
//! pane's holder should have under its topology lock and hands them here,
//! so no node call runs under that lock or on the node's own thread that
//! told the engine what changed. A pane asked for twice before the thread
//! took it is given the latest rules once.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

use hide_node_link::holders::{HolderError, PaneHolders, ScreenRules};

#[derive(Default)]
struct Pending {
    panes: BTreeMap<String, Option<ScreenRules>>,
    phase: Phase,
    #[cfg(test)]
    waiting: Option<std::sync::mpsc::Sender<()>>,
}

#[derive(Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Waiting,
    Ready,
    Ended,
}

const PENDING_CAP: usize = hide_kit::panes::HOLDER_CAP;

type Shared = Arc<(Mutex<Pending>, Condvar)>;

pub(super) struct RulesWorker {
    pending: Shared,
    thread: Option<JoinHandle<()>>,
}

impl RulesWorker {
    pub(super) fn start(holders: Arc<dyn PaneHolders>, ready: bool) -> Result<Self, String> {
        let pending: Shared = Arc::default();
        if ready {
            pending
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .phase = Phase::Ready;
        }
        let waiting = Arc::clone(&pending);
        let thread = std::thread::Builder::new()
            .name("hide-own-engine-rules".into())
            .spawn(move || give_loop(&waiting, holders.as_ref()))
            .map_err(|error| format!("the screen rules thread could not start: {error}"))?;
        Ok(Self {
            pending,
            thread: Some(thread),
        })
    }

    /// Gives `pane`'s holder `rules`, in place of any it was given.
    pub(super) fn give(&self, pane: &str, rules: Option<ScreenRules>) {
        let (pending, wake) = &*self.pending;
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.phase == Phase::Ended {
            return;
        }
        if pending.panes.len() >= PENDING_CAP && !pending.panes.contains_key(pane) {
            drop(pending);
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "pane.rules_not_given",
                "pane_id": pane,
                "message": "the pending screen rules limit was reached",
                "limit": PENDING_CAP,
            }));
            return;
        }
        pending.panes.insert(pane.to_owned(), rules);
        drop(pending);
        wake.notify_one();
    }

    /// The node acknowledged this connection's owner after recovery.
    pub(super) fn ready(&self) {
        let (pending, wake) = &*self.pending;
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.phase == Phase::Waiting {
            pending.phase = Phase::Ready;
        }
        drop(pending);
        wake.notify_one();
    }

    /// Ends admission and discards queued intentions; never reopens them.
    pub(super) fn end(&self) -> usize {
        let (pending, wake) = &*self.pending;
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.phase = Phase::Ended;
        let discarded = pending.panes.len();
        pending.panes.clear();
        drop(pending);
        wake.notify_one();
        discarded
    }

    /// Observes the actual wait with pending work, without holding it up.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> std::sync::mpsc::Receiver<()> {
        let (send, receive) = std::sync::mpsc::channel();
        self.pending.0.lock().unwrap().waiting = Some(send);
        self.pending.1.notify_one();
        receive
    }
}

impl Drop for RulesWorker {
    fn drop(&mut self) {
        self.end();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn give_loop(pending: &Shared, holders: &dyn PaneHolders) {
    let (state, wake) = &**pending;
    loop {
        let panes = {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            while state.phase != Phase::Ended
                && (state.phase == Phase::Waiting || state.panes.is_empty())
            {
                #[cfg(test)]
                if state.phase == Phase::Waiting
                    && !state.panes.is_empty()
                    && let Some(waiting) = state.waiting.take()
                {
                    let _ = waiting.send(());
                }
                state = wake.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            if state.phase == Phase::Ended {
                return;
            }
            std::mem::take(&mut state.panes)
        };
        for (pane, rules) in panes {
            // Only a call admitted before end may finish after it. Do not
            // start the rest of a drained batch while Drop joins us.
            if state.lock().unwrap_or_else(PoisonError::into_inner).phase == Phase::Ended {
                return;
            }
            let agent = rules.as_ref().map(|rules| rules.agent.clone());
            match holders.set_rules(&pane, rules) {
                // A pane that closed meanwhile has no holder to give them to.
                Ok(()) | Err(HolderError::NoPane { .. }) => {}
                Err(error) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "pane.rules_not_given",
                    "pane_id": pane,
                    "agent": agent,
                    "message": error.to_string(),
                })),
            }
        }
    }
}
