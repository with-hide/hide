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
use std::time::{Duration, Instant};

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
    /// The accept thread, woken from its wait between tries at the close.
    accepting: Option<std::thread::Thread>,
}

impl CloseOnDrop {
    pub fn new(closer: hide_platform::ipc::ListenerCloser) -> Self {
        Self {
            closer,
            closed: Arc::new(AtomicBool::new(false)),
            accepting: None,
        }
    }

    pub fn closed(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.closed)
    }

    /// The close wakes `thread` from a wait between tries, so it ends at
    /// once rather than after the wait.
    pub fn wakes(&mut self, thread: std::thread::Thread) {
        self.accepting = Some(thread);
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.closer.close();
        if let Some(thread) = &self.accepting {
            thread.unpark();
        }
    }
}

/// Hands each connection `accept` takes to `accepted` until the listener is
/// closed or nothing receives any more. A failed accept is recorded as
/// `<component>`'s `kind`, with its message and the wait before the next try,
/// which `pause` waits out: [`park_unless_closed`] for a listener, the
/// test's own clock in a test.
pub fn accept_until_closed<S>(
    mut accept: impl FnMut() -> io::Result<S>,
    accepted: &tokio::sync::mpsc::Sender<S>,
    closed: &AtomicBool,
    health: &ListenerHealth,
    (component, kind): (&'static str, &'static str),
    mut pause: impl FnMut(Duration),
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
                pause(wait);
                wait = (wait * 2).min(RETRY_MAX);
            }
        }
    }
}

/// Waits `wait` on a park the listener's close ends early
/// ([`CloseOnDrop::wakes`]); a spurious wake only shortens one wait.
pub fn park_unless_closed(wait: Duration, closed: &AtomicBool) {
    wait_unless_closed(wait, closed, std::thread::park_timeout);
}

/// [`park_unless_closed`] with the block it waits in passed in, which is
/// called only after `closed` was last read as open.
fn wait_unless_closed(wait: Duration, closed: &AtomicBool, mut block: impl FnMut(Duration)) {
    let until = Instant::now() + wait;
    while !closed.load(Ordering::SeqCst) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        block(left);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc;

    use super::*;

    const HANG_LIMIT: Duration = Duration::from_secs(30);

    fn out_of_descriptors<S>() -> io::Result<S> {
        Err(io::Error::other("Too many open files (os error 24)"))
    }

    /// Accepts that keep running out of descriptors are tried again after a
    /// wait that doubles from 100 ms to 5 s (docs/ARCHITECTURE.md), the
    /// listener reads `failing` meanwhile, the next connection is taken, and
    /// a failure after it waits 100 ms again. The waits are the test's clock.
    #[tokio::test]
    async fn accepts_out_of_descriptors_back_off_and_the_next_connection_is_taken() {
        let health = Arc::new(ListenerHealth::default());
        let closed = Arc::new(AtomicBool::new(false));
        let waits = Arc::new(Mutex::new(Vec::new()));
        let (accepted, mut arrivals) = tokio::sync::mpsc::channel(1);
        // The ninth accept waits until the test has read `failing`, and the
        // twelfth until the test closes the listener, as a closed
        // listener's waiting accept returns.
        let (release, released) = mpsc::channel::<()>();
        let (failed, failures_seen) = mpsc::channel::<()>();
        let (close, closing) = mpsc::channel::<()>();
        let thread = {
            let (health, closed, waits) =
                (Arc::clone(&health), Arc::clone(&closed), Arc::clone(&waits));
            std::thread::spawn(move || {
                let mut tries = 0;
                accept_until_closed(
                    move || {
                        tries += 1;
                        match tries {
                            1..=8 | 10 => out_of_descriptors(),
                            9 => {
                                failed.send(()).unwrap();
                                released.recv_timeout(HANG_LIMIT).unwrap();
                                Ok("the next connection")
                            }
                            11 => Ok("the one after"),
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
                    |wait| waits.lock().unwrap().push(wait.as_millis()),
                );
            })
        };
        tokio::task::spawn_blocking(move || failures_seen.recv_timeout(HANG_LIMIT))
            .await
            .unwrap()
            .expect("the accept was not tried again");
        assert_eq!(health.state(), ListenerState::Failing);
        assert_eq!(
            *waits.lock().unwrap(),
            [100, 200, 400, 800, 1600, 3200, 5000, 5000]
        );
        release.send(()).unwrap();
        for expected in ["the next connection", "the one after"] {
            let taken = tokio::time::timeout(HANG_LIMIT, arrivals.recv())
                .await
                .expect("no connection was handed over");
            assert_eq!(taken, Some(expected));
        }
        assert_eq!(health.state(), ListenerState::Accepting);
        // The failure between the two connections waited the first wait.
        assert_eq!(waits.lock().unwrap().last(), Some(&100));
        // The listener's own close ends the thread.
        closed.store(true, Ordering::SeqCst);
        close.send(()).unwrap();
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .unwrap()
            .unwrap();
    }

    /// The listener's close wakes an accept thread from its wait between
    /// tries, here an hour long, and the thread ends.
    #[test]
    fn the_close_ends_an_accept_thread_parked_between_tries() {
        let directory = tempfile::tempdir().unwrap();
        let listener =
            hide_platform::ipc::LocalListener::bind(&directory.path().join("listener")).unwrap();
        let mut closing = CloseOnDrop::new(listener.closer());
        let closed = closing.closed();
        let health = ListenerHealth::default();
        let (accepted, _arrivals) = tokio::sync::mpsc::channel::<()>(1);
        let (parking, parked) = mpsc::channel::<()>();
        let (ended, end_seen) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            accept_until_closed(
                out_of_descriptors,
                &accepted,
                &closed,
                &health,
                ("test", "test.accept_failed"),
                // The barrier sits after the wait's last read of `closed`
                // and right before the park, so the close lands while the
                // thread is parked or about to park; only the close's unpark
                // (or its token) then ends the hour.
                |_| {
                    wait_unless_closed(Duration::from_secs(3600), &closed, |left| {
                        let _ = parking.send(());
                        std::thread::park_timeout(left);
                    });
                },
            );
            ended.send(()).unwrap();
        });
        closing.wakes(thread.thread().clone());
        parked
            .recv_timeout(HANG_LIMIT)
            .expect("the accept never failed");
        drop(closing);
        end_seen
            .recv_timeout(HANG_LIMIT)
            .expect("the close did not end the thread's wait");
        thread.join().unwrap();
    }
}
