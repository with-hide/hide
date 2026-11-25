//! How each end of a node link finds the other gone.
//!
//! A connection can die without a close reaching one side (a Wi-Fi roam
//! leaves a server's TCP open for about fifteen minutes), and a node holds
//! its state folder's pane ownership until its link ends, so both ends of
//! every link run the same [`Liveness`]: the node in `hide_host::serve`,
//! whatever carries its link, the core in `hide_node::ssh::host` on every
//! link it holds, and the attach role's pipe between them. Each end writes
//! [`PING_LINE`] every [`HEARTBEAT`] from a pinger thread, notes every read
//! that brings bytes, and a watch thread that never touches the link ends
//! it once the other end has sent nothing for [`HEARTBEAT_LIMIT`]. A line
//! costs nothing more than the read that brought it storing one time: no
//! lock, no hand-off, no wakeup.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How often each end of every node link writes [`PING_LINE`]: the node,
/// whatever its link, and the core on every link it holds (SSH, WSL, a node
/// that dialed it, a paired machine's guest node).
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// How long each end of a node link waits for a byte from the other before
/// it ends the link as gone: three heartbeats, so no link may go without
/// this limit.
pub const HEARTBEAT_LIMIT: Duration = Duration::from_secs(30);
/// The line each end of a node link writes every [`HEARTBEAT`] and the
/// other drops as it reads it: the node's
/// [`NodeEvent::Ping`](crate::panes::NodeEvent::Ping), which the core writes
/// back as it is.
pub const PING_LINE: &[u8] = b"{\"event\":\"ping\"}\n";

/// `heard` before the other end's first byte.
const UNHEARD: u64 = u64::MAX;

/// Why a link's watch found it gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gone {
    /// The other end sent nothing for [`HEARTBEAT_LIMIT`].
    Silent,
    /// A write to the other end failed ([`Liveness::fail`]).
    Failed(io::ErrorKind, String),
}

impl Gone {
    /// The link's end as the error its owner answers with; `other` names
    /// the other end, such as "the core".
    pub fn error(&self, other: &str) -> io::Error {
        match self {
            Self::Silent => io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{other} sent nothing for {} s", HEARTBEAT_LIMIT.as_secs()),
            ),
            Self::Failed(kind, reason) => io::Error::new(*kind, reason.clone()),
        }
    }
}

/// One end of a node link: when it last heard the other, whether the link
/// ended, and what its watch found.
#[derive(Debug)]
pub struct Liveness {
    started: Instant,
    /// Milliseconds after `started` of the last read that brought bytes,
    /// on the monotonic clock, so a step of the wall clock neither ends a
    /// healthy link nor hides a silent one; [`UNHEARD`] before the first.
    heard: AtomicU64,
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct State {
    /// The link ended: the watch and the pinger return.
    ended: bool,
    /// A write failed, for the watch to end the link by.
    failed: Option<(io::ErrorKind, String)>,
    /// What the watch found, once it did.
    gone: Option<Gone>,
}

impl Liveness {
    /// A link whose other end counts as heard as it starts.
    pub fn new() -> Self {
        Self::heard_at(0)
    }

    /// A link whose silence counts from the other end's first byte: a
    /// program that may still be starting (a new helper's first start on
    /// Windows is scanned for tens of seconds), whose first call's own
    /// deadline bounds that wait.
    pub fn unheard() -> Self {
        Self::heard_at(UNHEARD)
    }

    fn heard_at(heard: u64) -> Self {
        Self {
            started: Instant::now(),
            heard: AtomicU64::new(heard),
            state: Mutex::default(),
            changed: Condvar::new(),
        }
    }

    /// A read brought bytes.
    pub fn hear(&self) {
        let at = self.started.elapsed().as_millis() as u64;
        self.heard.store(at, Ordering::Relaxed);
    }

    /// How long until the other end has sent nothing for
    /// [`HEARTBEAT_LIMIT`]; `None` once it has. A link not heard yet waits
    /// a whole limit at a time.
    fn left(&self) -> Option<Duration> {
        let heard = self.heard.load(Ordering::Relaxed);
        if heard == UNHEARD {
            return Some(HEARTBEAT_LIMIT);
        }
        (Duration::from_millis(heard) + HEARTBEAT_LIMIT)
            .checked_sub(self.started.elapsed())
            .filter(|left| !left.is_zero())
    }

    /// The link ended, by whatever ended it: its watch and its pinger
    /// return.
    pub fn end(&self) {
        self.lock().ended = true;
        self.changed.notify_all();
    }

    /// A write to the other end failed: the watch ends the link with it,
    /// even while the other end still writes. The first failure stands.
    pub fn fail(&self, error: &io::Error) {
        let mut state = self.lock();
        if state.failed.is_none() {
            state.failed = Some((error.kind(), error.to_string()));
        }
        drop(state);
        self.changed.notify_all();
    }

    /// What the watch found, once it did.
    pub fn gone(&self) -> Option<Gone> {
        self.lock().gone.clone()
    }

    /// Watches the link until it ends: once its other end has sent nothing
    /// for [`HEARTBEAT_LIMIT`], or a write to it failed, calls `gone`,
    /// which ends the link its owner's way, and returns. It never reads,
    /// writes or waits on the link itself, so a connection stuck in a read
    /// or a write never holds it.
    pub fn watch(&self, gone: impl FnOnce(&Gone)) {
        let mut state = self.lock();
        let found = loop {
            if state.ended {
                return;
            }
            if let Some((kind, reason)) = state.failed.clone() {
                break Gone::Failed(kind, reason);
            }
            let Some(left) = self.left() else {
                break Gone::Silent;
            };
            let asked = Instant::now();
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            // A wait that ended a heartbeat or more past what it asked for
            // was the machine asleep, which Windows's clock counts and
            // Linux's and macOS's do not: the other end gets a whole limit
            // from the wake, rather than every link ending as it wakes.
            if asked.elapsed() > left + HEARTBEAT && self.heard.load(Ordering::Relaxed) != UNHEARD {
                self.hear();
            }
        };
        state.gone = Some(found.clone());
        drop(state);
        gone(&found);
    }

    /// Calls `ping`, which writes [`PING_LINE`] to the other end, every
    /// [`HEARTBEAT`] until the link ends or `ping` answers false. `ping`
    /// may wait on a stuck connection; the watch does not wait for it.
    pub fn pinger(&self, mut ping: impl FnMut() -> bool) {
        loop {
            let state = self
                .changed
                .wait_timeout_while(self.lock(), HEARTBEAT, |state| !state.ended)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            if state.ended {
                return;
            }
            drop(state);
            if !ping() {
                return;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for Liveness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A write that failed ends the link at once, whatever the other end
    /// still sends, and the watch says why.
    #[test]
    fn a_failed_write_ends_the_link_while_the_other_end_still_speaks() {
        let liveness = Arc::new(Liveness::new());
        let (found, seen) = std::sync::mpsc::channel();
        let watching = {
            let liveness = Arc::clone(&liveness);
            std::thread::spawn(move || liveness.watch(|gone| found.send(gone.clone()).unwrap()))
        };
        liveness.hear();
        liveness.fail(&io::Error::new(io::ErrorKind::BrokenPipe, "the pipe broke"));
        let gone = seen
            .recv_timeout(Duration::from_secs(5))
            .expect("the watch ended a link whose write failed");
        assert_eq!(
            gone,
            Gone::Failed(io::ErrorKind::BrokenPipe, "the pipe broke".to_owned())
        );
        assert_eq!(liveness.gone(), Some(gone));
        watching.join().unwrap();
    }

    /// A link that ended some other way stops its watch and its pinger at
    /// once, without a gone.
    #[test]
    fn an_ended_link_stops_its_watch_and_its_pinger() {
        let liveness = Arc::new(Liveness::unheard());
        let threads = [
            {
                let liveness = Arc::clone(&liveness);
                std::thread::spawn(move || liveness.watch(|_| panic!("an ended link went")))
            },
            {
                let liveness = Arc::clone(&liveness);
                std::thread::spawn(move || liveness.pinger(|| panic!("an ended link pinged")))
            },
        ];
        let started = Instant::now();
        liveness.end();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(started.elapsed() < HEARTBEAT);
        assert_eq!(liveness.gone(), None);
    }
}
