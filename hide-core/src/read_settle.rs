//! The operator's look compared once more after every announced change.
//!
//! A change reaches the screen only by an announcement
//! ([`crate::handle::ChangeNotifier::notify`]), whoever made it: an event, a
//! device's session, or any worker that applies its answer under the runtime
//! lock and announces after. The look can move in any of them (a moved pane
//! focused, a device brought forward by a project open, a device window's
//! tab), and a moved look reads every row again (`Runtime::follow_operator_look`,
//! `docs/status-model.md`, Hide owns the read axis). Every announcement
//! raises this settle's one pending mark, and its thread takes the lock once
//! and compares, whether or not a window reads snapshots: no list of entries
//! to keep, and a read never waits for a window (with-hide/hide-private#685).
//!
//! Some announcers hold the lock while they announce, so the comparison
//! cannot run in `notify` itself; it runs on this thread as soon as the lock
//! is free. An event and a session already compare inside their own change;
//! this is the close for every other entry.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread;

use crate::handle::ChangeNotifier;
use crate::runtime::Runtime;

#[derive(Default)]
struct State {
    /// An announcement arrived since the last comparison began: one mark,
    /// however many announcements, so the work is one pass per burst.
    pending: bool,
    stopped: bool,
    /// A comparison is running, for a test that waits for them.
    #[cfg(test)]
    running: bool,
    /// Comparisons finished, for a test that counts them.
    #[cfg(test)]
    passes: u64,
}

/// What an announcement raises and the settle thread waits on.
#[derive(Default)]
pub(crate) struct SettleSignal {
    state: Mutex<State>,
    wake: Condvar,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SettleSignal {
    /// Called by every announcement; cheap, and never takes the runtime lock.
    pub(crate) fn raise(&self) {
        let mut state = lock(&self.state);
        if !state.pending {
            state.pending = true;
            self.wake.notify_all();
        }
    }

    /// Waits for a mark and takes it; `false` once the settle stops.
    fn next(&self) -> bool {
        let mut state = lock(&self.state);
        loop {
            if state.stopped {
                return false;
            }
            if state.pending {
                state.pending = false;
                #[cfg(test)]
                {
                    state.running = true;
                }
                return true;
            }
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn finished(&self) {
        #[cfg(test)]
        {
            let mut state = lock(&self.state);
            state.running = false;
            state.passes += 1;
            self.wake.notify_all();
        }
    }

    fn stop(&self) {
        lock(&self.state).stopped = true;
        self.wake.notify_all();
    }

    /// Waits until no mark is pending and no comparison runs, and answers
    /// how many comparisons have finished; `None` past `timeout`.
    #[cfg(test)]
    pub(crate) fn wait_settled(&self, timeout: std::time::Duration) -> Option<u64> {
        let until = std::time::Instant::now() + timeout;
        let mut state = lock(&self.state);
        while state.pending || state.running {
            let left = until.checked_duration_since(std::time::Instant::now())?;
            state = self
                .wake
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        Some(state.passes)
    }
}

/// The core's one settle thread. It ends with the core: dropping it stops
/// the thread and joins it, outside the runtime lock.
pub(crate) struct ReadSettle {
    signal: Arc<SettleSignal>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ReadSettle {
    pub(crate) fn spawn(
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) -> std::io::Result<Self> {
        let signal = notifier.settle_signal();
        let thread_signal = Arc::clone(&signal);
        let worker = thread::Builder::new()
            .name("read-settle".into())
            .spawn(move || {
                while thread_signal.next() {
                    let changed = runtime.upgrade().is_some_and(|runtime| {
                        runtime
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .follow_operator_look()
                    });
                    // Announced before this pass is counted finished, so a
                    // read that changed rows is followed by one more pass,
                    // which finds the look where the rows were read and
                    // announces nothing: two passes, never a loop.
                    if changed {
                        notifier.notify();
                    }
                    thread_signal.finished();
                }
            })?;
        Ok(Self {
            signal,
            worker: Some(worker),
        })
    }
}

impl Drop for ReadSettle {
    fn drop(&mut self) {
        self.signal.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            crate::diagnostic!(
                serde_json::json!({"component": "read_settle", "kind": "worker.join_failed"})
            );
        }
    }
}
