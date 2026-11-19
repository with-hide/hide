//! Two machines on the loopback transport (PRD device-connect-iroh D-63):
//! pairing, grants, the dial rule and delivery, with no network.
//!
//! Each test builds its machines from real state folders and the real
//! stores; only the wire is in-process. A test waits for the event that
//! says a state was reached, never for time; [`within`] is the bound after
//! which a wait is a failure.

mod delivery;
mod dial;
mod grants;
mod pairing;
mod pairings;
mod raw;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use hide_kit::layout;
use hide_peer::loopback::{LoopConn, Loopback, Net};
use hide_peer::peers::{Config, Incoming, PeerEvent, Peers};
use hide_peer::protocol::{PeerAddr, PeerKey, StreamKind};
use hide_peer::store::{Host, KnownState, Via};
use hide_peer::{Clock, MachineKey, PeerStream};
use tokio::sync::{broadcast, mpsc};

/// How long a state may take to be reached before the test fails. It is a
/// bound on a wait for an event, never a pause.
const WITHIN: Duration = Duration::from_secs(20);

pub async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, wait)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

pub struct Machine {
    pub name: &'static str,
    pub peers: Peers<Loopback>,
    pub incoming: mpsc::Receiver<Incoming<LoopConn>>,
    pub events: broadcast::Receiver<PeerEvent>,
    pub dir: Arc<tempfile::TempDir>,
}

impl Machine {
    /// A machine with its own state folder and key on `net`.
    pub fn new(net: &Arc<Net>, name: &'static str) -> Self {
        Self::in_dir(net, name, Arc::new(tempfile::tempdir().unwrap()))
    }

    /// A machine started on a state folder that may already hold a key and
    /// records: the same machine after a restart.
    pub fn in_dir(net: &Arc<Net>, name: &'static str, dir: Arc<tempfile::TempDir>) -> Self {
        Self::with_clock(net, name, dir, Clock::system())
    }

    /// A machine whose records are stamped by `clock`.
    pub fn with_clock(
        net: &Arc<Net>,
        name: &'static str,
        dir: Arc<tempfile::TempDir>,
        clock: Clock,
    ) -> Self {
        let (key, _) = MachineKey::load_or_create(&layout::machine_key(dir.path()), name).unwrap();
        let config = Config {
            state_dir: dir.path(),
            build: "1.3.0-test",
            device: name,
            release: false,
            clock,
        };
        let (peers, incoming) = Peers::start(net.join(key), config).unwrap();
        let events = peers.events();
        Self {
            name,
            peers,
            incoming,
            events,
            dir,
        }
    }

    pub fn key(&self) -> PeerKey {
        self.peers.key()
    }

    pub fn addr(&self) -> PeerAddr {
        PeerAddr::new(self.key(), Vec::new())
    }

    /// The next event `pick` takes.
    pub async fn event<T>(
        &mut self,
        what: &str,
        mut pick: impl FnMut(&PeerEvent) -> Option<T>,
    ) -> T {
        within(what, async {
            loop {
                let event = self.events.recv().await.expect("the machine's events");
                if let Some(picked) = pick(&event) {
                    return picked;
                }
            }
        })
        .await
    }

    /// The next stream of `kind` a peer opened here.
    pub async fn accepted(&mut self, kind: StreamKind) -> PeerStream<LoopConn> {
        within("a stream the peer opened", async {
            loop {
                let incoming = self.incoming.recv().await.expect("the machine's streams");
                if incoming.stream.kind == kind {
                    return incoming.stream;
                }
            }
        })
        .await
    }
}

/// A clock that follows the test runtime's time: under a paused runtime,
/// ten minutes of a pending grant pass when the test advances them.
pub fn runtime_clock() -> Clock {
    let started = tokio::time::Instant::now();
    Clock::new(move || 1_000_000 + started.elapsed().as_millis() as u64)
}

/// Makes `guest` a guest of `host` through the stores, as a finished
/// pairing leaves them.
pub fn pair(host: &Machine, guest: &Machine) {
    let grant = host
        .peers
        .grant_pending(guest.key(), guest.name, guest.name, Via::Link)
        .unwrap();
    host.peers.confirm(&guest.key(), &grant.id).unwrap();
    guest
        .peers
        .known(|known| {
            known.put(Host {
                key: host.key(),
                device: host.name.to_owned(),
                name: host.name.to_owned(),
                grant: grant.id,
                state: KnownState::Active,
                candidates: Vec::new(),
            })
        })
        .unwrap();
}

/// Waits until the grant `of` gives this machine is `state`.
pub async fn granted(
    machine: &mut Machine,
    of: PeerKey,
    state: hide_peer::protocol::hello::GrantState,
) {
    if machine
        .peers
        .link(&of)
        .is_some_and(|link| link.granted() == state)
    {
        return;
    }
    machine
        .event("the peer's grant", |event| {
            matches!(event, PeerEvent::Granted { key, state: now } if *key == of && *now == state)
                .then_some(())
        })
        .await;
}
