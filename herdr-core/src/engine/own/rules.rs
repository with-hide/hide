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
    ending: bool,
}

type Shared = Arc<(Mutex<Pending>, Condvar)>;

pub(super) struct RulesWorker {
    pending: Shared,
    thread: Option<JoinHandle<()>>,
}

impl RulesWorker {
    pub(super) fn start(holders: Arc<dyn PaneHolders>) -> Result<Self, String> {
        let pending: Shared = Arc::default();
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
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .panes
            .insert(pane.to_owned(), rules);
        wake.notify_one();
    }
}

impl Drop for RulesWorker {
    fn drop(&mut self) {
        let (pending, wake) = &*self.pending;
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ending = true;
        wake.notify_one();
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
            while state.panes.is_empty() && !state.ending {
                state = wake.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            if state.ending {
                return;
            }
            std::mem::take(&mut state.panes)
        };
        for (pane, rules) in panes {
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
