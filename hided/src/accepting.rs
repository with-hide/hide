//! The accept thread of a local listener the core serves (the pane bootstrap
//! and the node attach socket), and what it is doing now, for `/health`.
//!
//! An accept that fails is logged and tried again after a wait that doubles
//! from 100 ms to 5 s, as axum's `serve` and hyper's `AddrIncoming` sleep and
//! accept again rather than end: the failures an accept meets are a process
//! out of descriptors (EMFILE), a system out of them (ENFILE), a peer gone
//! before it was taken (ECONNABORTED) or an interrupted call, all of which
//! pass. On 2026-11-20 the bootstrap ended at its first EMFILE instead, and
//! every `hide` command from a pane and every agent hook was refused for
//! seven hours while `hide status` said Hide ran. Only the listener's own
//! close ends the thread.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use serde_json::json;

/// The wait after an accept that failed, doubling to [`RETRY_MAX`].
const RETRY_FIRST: Duration = Duration::from_millis(100);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// What a listener's accept thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenerState {
    /// Waiting for, or taking, connections.
    Accepting,
    /// The last accept failed and none has worked since; it is tried again.
    Failing,
    /// The thread ended, or never started, while the daemon runs: nothing
    /// takes a connection until hided starts again.
    Stopped,
}

impl ListenerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepting => "accepting",
            Self::Failing => "failing",
            Self::Stopped => "stopped",
        }
    }
}

/// A listener's [`ListenerState`], shared between its accept thread and
/// whoever reports it.
#[derive(Debug, Default)]
pub struct ListenerHealth(AtomicU8);

impl ListenerHealth {
    pub fn state(&self) -> ListenerState {
        match self.0.load(Ordering::SeqCst) {
            0 => ListenerState::Accepting,
            1 => ListenerState::Failing,
            _ => ListenerState::Stopped,
        }
    }

    fn set(&self, state: ListenerState) {
        self.0.store(state as u8, Ordering::SeqCst);
    }

    /// The listener stopped for a reason other than the daemon's own stop.
    pub fn stopped(&self) {
        self.set(ListenerState::Stopped);
    }
}

/// The core's local listeners, as `/health` names them.
#[derive(Debug, Default)]
pub struct CoreListeners {
    /// The pane bootstrap: every `hide` command from a pane and every agent
    /// hook gets its credential there.
    pub pane_bootstrap: Arc<ListenerHealth>,
    /// The socket another machine's node attaches to the core through.
    pub node_attach: Arc<ListenerHealth>,
}

impl CoreListeners {
    pub fn report(&self) -> serde_json::Value {
        json!({
            "pane_bootstrap": self.pane_bootstrap.state().as_str(),
            "node_attach": self.node_attach.state().as_str(),
        })
    }
}

/// Closes the listener however its serving future ends, a drop included, so
/// the accept thread never outlives it.
pub struct CloseOnDrop {
    closer: hide_platform::ipc::ListenerCloser,
    /// Set before the close, so the accept thread tells its own stop from an
    /// accept that failed.
    closed: Arc<AtomicBool>,
}

impl CloseOnDrop {
    pub fn new(closer: hide_platform::ipc::ListenerCloser) -> Self {
        Self {
            closer,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn closed(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.closed)
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.closer.close();
    }
}

/// Hands each connection `accept` takes to `accepted` until the listener is
/// closed or nothing receives any more. A failed accept is recorded as
/// `<component>`'s `kind`, with its message and the wait before the next try.
#[allow(clippy::disallowed_methods)] // a production wait between accepts, not test code
pub fn accept_until_closed<S>(
    mut accept: impl FnMut() -> io::Result<S>,
    accepted: &tokio::sync::mpsc::Sender<S>,
    closed: &AtomicBool,
    health: &ListenerHealth,
    (component, kind): (&'static str, &'static str),
) {
    let mut wait = RETRY_FIRST;
    loop {
        match accept() {
            Ok(stream) => {
                wait = RETRY_FIRST;
                health.set(ListenerState::Accepting);
                if accepted.blocking_send(stream).is_err() {
                    return;
                }
            }
            Err(_) if closed.load(Ordering::SeqCst) => return,
            Err(error) => {
                health.set(ListenerState::Failing);
                hide_core::diagnostic!(json!({
                    "component": component,
                    "kind": kind,
                    "message": error.to_string(),
                    "retry_ms": wait.as_millis() as u64,
                }));
                std::thread::sleep(wait);
                wait = (wait * 2).min(RETRY_MAX);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANG_LIMIT: Duration = Duration::from_secs(30);

    /// An accept that runs out of descriptors is tried again, the listener
    /// reads `failing` meanwhile, and the next connection is taken.
    #[tokio::test]
    async fn an_accept_out_of_descriptors_is_tried_again_and_the_next_connection_is_taken() {
        let health = Arc::new(ListenerHealth::default());
        let closed = Arc::new(AtomicBool::new(false));
        let (accepted, mut arrivals) = tokio::sync::mpsc::channel(1);
        // The second accept waits until the test has read `failing`, and the
        // third until the test closes the listener, as a closed listener's
        // waiting accept returns.
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (failed, failure_seen) = std::sync::mpsc::channel::<()>();
        let (close, closing) = std::sync::mpsc::channel::<()>();
        let thread = {
            let health = Arc::clone(&health);
            let closed = Arc::clone(&closed);
            std::thread::spawn(move || {
                let mut tries = 0;
                accept_until_closed(
                    move || {
                        tries += 1;
                        match tries {
                            1 => Err(io::Error::other("Too many open files (os error 24)")),
                            2 => {
                                failed.send(()).unwrap();
                                released.recv_timeout(HANG_LIMIT).unwrap();
                                Ok("the next connection")
                            }
                            _ => {
                                closing.recv_timeout(HANG_LIMIT).unwrap();
                                Err(io::Error::from(io::ErrorKind::ConnectionAborted))
                            }
                        }
                    },
                    &accepted,
                    &closed,
                    &health,
                    ("test", "test.accept_failed"),
                );
            })
        };
        tokio::task::spawn_blocking(move || failure_seen.recv_timeout(HANG_LIMIT))
            .await
            .unwrap()
            .expect("the accept was not tried again");
        assert_eq!(health.state(), ListenerState::Failing);
        release.send(()).unwrap();
        let taken = tokio::time::timeout(HANG_LIMIT, arrivals.recv())
            .await
            .expect("no connection was handed over");
        assert_eq!(taken, Some("the next connection"));
        assert_eq!(health.state(), ListenerState::Accepting);
        // The listener's own close ends the thread.
        closed.store(true, Ordering::SeqCst);
        close.send(()).unwrap();
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .unwrap()
            .unwrap();
    }
}
