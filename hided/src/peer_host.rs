//! This machine's connections to other Hides (PRD device-connect-iroh D-63,
//! D-83): the one endpoint, opened only when it is needed, and where each
//! stream a peer opens goes.
//!
//! A machine that is paired with nothing, and is neither sharing itself nor
//! adding a machine, opens no endpoint: no socket, no relay, and on macOS no
//! question about the local network. [`PeerHost::start`] opens it when a
//! grant or a known host is saved, and [`PeerHost::open`] at the moment the
//! operator shares this machine or adds one. Once open it stays open until
//! the daemon ends, which closes it.
//!
//! Each [`StreamKind`] has one owner, which hands in its handler
//! ([`PeerHost::handle`]) when the daemon starts. A stream of a kind no one
//! handles is dropped, which closes it for the peer.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use hide_kit::layout;
use hide_peer::peers::{Config, Peers};
use hide_peer::protocol::{Candidate, StreamKind};
use hide_peer::store::{Grants, Known};
use hide_peer::{Clock, MachineKey, PeerTransport};
use hide_peer_iroh::{DEVICE_RELAY_URL, IrohConn, IrohTransport, Options};
use serde_json::json;
use tokio::sync::{OnceCell, mpsc};

use crate::ending::Ending;

/// The transport's connection, and `hide_peer`'s types over it: what every
/// handler and every dialer in this daemon names.
pub type Conn = IrohConn;
pub type HostPeers = Peers<IrohTransport>;
pub type Link = hide_peer::Link<Conn>;
pub type Stream = hide_peer::PeerStream<Conn>;
pub type Incoming = hide_peer::peers::Incoming<Conn>;

type Handling = Pin<Box<dyn Future<Output = ()> + Send>>;
type Handler = Arc<dyn Fn(Incoming) -> Handling + Send + Sync>;

/// What this machine says of itself to a peer, and how its endpoint is set
/// up.
pub struct Settings {
    pub state_dir: PathBuf,
    /// This build's version, as an update notice names it.
    pub build: String,
    /// This machine's product device id: its node id.
    pub device: String,
    /// Whether this build is an official release.
    pub release: bool,
    /// `HIDE_PEER_DIRECT`: bind this one address and use no relay and no
    /// local network search. `None` is the product's endpoint.
    pub direct: Option<SocketAddr>,
}

impl Settings {
    fn endpoint(&self) -> Options {
        match self.direct {
            Some(address) => Options {
                relay: None,
                mdns: false,
                bind: Some(address),
            },
            None => Options {
                relay: Some(DEVICE_RELAY_URL.to_owned()),
                mdns: true,
                bind: None,
            },
        }
    }
}

pub struct PeerHost {
    settings: Settings,
    handlers: Mutex<HashMap<StreamKind, Handler>>,
    opened: OnceCell<HostPeers>,
    shutdown: Ending,
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // A map of handlers is whole whatever panicked beside it.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// Whether `state_dir` has a reason to open the endpoint at the daemon's
/// start: a grant this machine gives that has not run out, a pending one
/// included, since its guest confirms over the endpoint, or a host it is a
/// guest of. It is not "a pairing is confirmed": what follows only a
/// confirmed pairing asks that separately.
pub fn endpoint_wanted(state_dir: &Path) -> io::Result<bool> {
    let mut grants = Grants::open(layout::peer_grants(state_dir))?;
    grants.expire(now_unix_ms()).map_err(io::Error::other)?;
    let known = Known::open(layout::peer_known(state_dir))?;
    Ok(grants.list(now_unix_ms()).next().is_some() || !known.list().is_empty())
}

impl PeerHost {
    /// Nothing is opened yet. `shutdown` is the daemon's end, which closes
    /// the endpoint and every connection.
    pub fn new(settings: Settings, shutdown: Ending) -> Arc<Self> {
        Arc::new(Self {
            settings,
            handlers: Mutex::new(HashMap::new()),
            opened: OnceCell::new(),
            shutdown,
        })
    }

    /// Gives every stream of `kind` a peer opens to `handler`, each in a
    /// task of its own; the stream ends when that task drops it. A kind has
    /// one owner: a second handler for it is a mistake in the daemon's
    /// start and panics there.
    pub fn handle<F, Fut>(&self, kind: StreamKind, handler: F)
    where
        F: Fn(Incoming) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |incoming| Box::pin(handler(incoming)));
        let earlier = lock(&self.handlers).insert(kind, handler);
        assert!(
            earlier.is_none(),
            "two handlers for the {} stream",
            kind.name()
        );
    }

    /// Opens the endpoint at the daemon's start when a saved grant or a
    /// known host wants it ([`endpoint_wanted`]). `None` when nothing does.
    pub async fn start(self: &Arc<Self>) -> io::Result<Option<HostPeers>> {
        let state_dir = self.settings.state_dir.clone();
        let wanted = tokio::task::spawn_blocking(move || endpoint_wanted(&state_dir))
            .await
            .map_err(io::Error::other)??;
        match wanted {
            true => self.open().await.map(Some),
            false => Ok(None),
        }
    }

    /// Opens the endpoint, or returns the one already open: for the moment
    /// the operator shares this machine or adds one, and for whoever needs
    /// a connection a pairing already allows. Two callers open one.
    pub async fn open(self: &Arc<Self>) -> io::Result<HostPeers> {
        self.opened
            .get_or_try_init(|| self.open_endpoint())
            .await
            .cloned()
    }

    /// The open endpoint's connections; `None` while it is not open.
    pub fn peers(&self) -> Option<HostPeers> {
        self.opened.get().cloned()
    }

    /// Where a peer reaches this machine now, in the order to try; empty
    /// while the endpoint is not open.
    pub fn candidates(&self) -> Vec<Candidate> {
        self.peers()
            .map(|peers| peers.transport().candidates())
            .unwrap_or_default()
    }

    async fn open_endpoint(self: &Arc<Self>) -> io::Result<HostPeers> {
        if self.shutdown.has_ended() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the daemon is ending",
            ));
        }
        let key_file = layout::machine_key(&self.settings.state_dir);
        // The machine id is asked of the system, on macOS through a
        // program, and the key is a file: neither on a runtime thread.
        let (key, loaded) = tokio::task::spawn_blocking(move || {
            let machine = hide_platform::host::machine_id()?;
            MachineKey::load_or_create(&key_file, &machine)
        })
        .await
        .map_err(io::Error::other)??;
        let transport = IrohTransport::open(&key.secret(), self.settings.endpoint()).await?;
        let (peers, incoming) = Peers::start(
            transport,
            Config {
                state_dir: &self.settings.state_dir,
                build: &self.settings.build,
                device: &self.settings.device,
                release: self.settings.release,
                clock: Clock::system(),
            },
        )?;
        hide_core::diagnostic!(json!({
            "component": "peer_host",
            "kind": "peer_host.opened",
            "key": peers.key().short(),
            "machine_key": format!("{loaded:?}"),
            "direct": self.settings.direct.is_some(),
        }));
        tokio::spawn(Arc::clone(self).serve(peers.clone(), incoming));
        Ok(peers)
    }

    /// Hands each stream to its kind's handler until the daemon ends, then
    /// closes the endpoint. The streams a peer may have open are capped per
    /// connection (`hide_peer::limits::MAX_STREAMS`), and so are the tasks
    /// started here.
    async fn serve(self: Arc<Self>, peers: HostPeers, mut incoming: mpsc::Receiver<Incoming>) {
        loop {
            let next = tokio::select! {
                () = self.shutdown.ended() => break,
                next = incoming.recv() => next,
            };
            let Some(next) = next else { break };
            let kind = next.stream.kind;
            let handler = lock(&self.handlers).get(&kind).cloned();
            match handler {
                Some(handler) => {
                    tokio::spawn(handler(next));
                }
                None => hide_core::diagnostic!(json!({
                    "component": "peer_host",
                    "kind": "peer_host.stream_unhandled",
                    "stream": kind.name(),
                    "peer": next.link.key().short(),
                })),
            }
        }
        peers.close().await;
    }
}

/// Two machines for a test: each a [`PeerHost`] on a loopback address with
/// no relay, in a state folder of its own.
#[cfg(test)]
pub(crate) mod fixture {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use hide_peer::protocol::hello::GrantState;
    use hide_peer::protocol::{PeerAddr, PeerKey};
    use hide_peer::store::{Host, KnownState, Via};

    use super::*;

    /// A hang guard, never a pause.
    pub(crate) const WITHIN: Duration = Duration::from_secs(30);

    pub(crate) async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
        tokio::time::timeout(WITHIN, wait)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    pub(crate) struct Machine {
        pub(crate) host: Arc<PeerHost>,
        pub(crate) shutdown: Ending,
        pub(crate) dir: tempfile::TempDir,
    }

    impl Machine {
        pub(crate) fn new(name: &str) -> Self {
            Self::at(name, tempfile::tempdir().unwrap())
        }

        /// A machine whose state folder is `dir`.
        pub(crate) fn at(name: &str, dir: tempfile::TempDir) -> Self {
            let shutdown = Ending::new();
            let host = PeerHost::new(
                Settings {
                    state_dir: dir.path().to_path_buf(),
                    build: "1.3.0-test".to_owned(),
                    device: name.to_owned(),
                    release: false,
                    direct: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
                },
                shutdown.clone(),
            );
            Self {
                host,
                shutdown,
                dir,
            }
        }

        pub(crate) fn addr(&self) -> PeerAddr {
            let peers = self.host.peers().expect("an open endpoint");
            PeerAddr::new(peers.key(), self.host.candidates())
        }
    }

    impl Drop for Machine {
        fn drop(&mut self) {
            self.shutdown.end();
        }
    }

    /// Saves in `host`'s state folder the grant a finished pairing leaves,
    /// before its endpoint is open, and returns the grant's id.
    pub(crate) fn grant(host: &Machine, guest: PeerKey) -> String {
        let mut grants = Grants::open(layout::peer_grants(host.dir.path())).unwrap();
        let grant = grants
            .grant_pending(guest, "macbook", "macbook", Via::Link, now_unix_ms())
            .unwrap();
        grants.confirm(&guest, &grant.id, now_unix_ms()).unwrap();
        grant.id
    }

    /// A folder short enough that a guest's endpoints fit a Unix socket
    /// address below it (docs/TESTING.md, Writing a Rust test, rule 7).
    pub(crate) fn short_root() -> tempfile::TempDir {
        #[cfg(unix)]
        return tempfile::Builder::new()
            .prefix("hg")
            .tempdir_in("/tmp")
            .unwrap();
        #[cfg(not(unix))]
        tempfile::tempdir().unwrap()
    }

    /// Pairs `guest` to `host` as a finished pairing leaves them, opens
    /// both endpoints and answers the guest's connection once the host's
    /// grant is active on it. The host's handlers are handed in first.
    pub(crate) async fn paired(host: &Machine, guest: &Machine) -> Link {
        let guest_peers = guest.host.open().await.unwrap();
        let grant = grant(host, guest_peers.key());
        let host_peers = host
            .host
            .start()
            .await
            .unwrap()
            .expect("a machine with a grant opens its endpoint at start");
        guest_peers
            .known(|known| {
                known.put(Host {
                    key: host_peers.key(),
                    device: "windows-pc".to_owned(),
                    name: "windows-pc".to_owned(),
                    grant,
                    state: KnownState::Active,
                    candidates: host.addr().candidates,
                })
            })
            .unwrap();
        let link = within("the dial", guest_peers.dial(&host.addr()))
            .await
            .unwrap();
        let mut events = guest_peers.events();
        if link.granted() != GrantState::Active {
            within("the host's grant", async {
                loop {
                    if let hide_peer::PeerEvent::Granted {
                        state: GrantState::Active,
                        ..
                    } = events.recv().await.unwrap()
                    {
                        break;
                    }
                }
            })
            .await;
        }
        link
    }
}

#[cfg(test)]
mod tests {
    use hide_peer::protocol::CloseCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::fixture::*;
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_machine_paired_with_nothing_opens_no_endpoint_and_makes_no_key() {
        let machine = Machine::new("windows-pc");
        assert!(machine.host.start().await.unwrap().is_none());
        assert!(machine.host.peers().is_none());
        assert!(machine.host.candidates().is_empty());
        assert!(!layout::peers(machine.dir.path()).exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_paired_machine_opens_at_start_and_each_stream_reaches_its_kinds_handler() {
        let host = Machine::new("windows-pc");
        let guest = Machine::new("macbook");
        let (heard, mut hears) = mpsc::channel(4);
        host.host
            .handle(StreamKind::Management, move |mut incoming: Incoming| {
                let heard = heard.clone();
                async move {
                    let mut bytes = [0u8; 5];
                    incoming.stream.recv.read_exact(&mut bytes).await.unwrap();
                    heard
                        .send((incoming.link.key(), incoming.stream.kind, bytes))
                        .await
                        .unwrap();
                }
            });
        // The guest adds a machine, which opens its endpoint and makes its
        // key; the host's start then finds the grant.
        let link = paired(&host, &guest).await;
        let guest_peers = guest.host.peers().unwrap();
        let mut management = link.open(StreamKind::Management).await.unwrap();
        management.send.write_all(b"hello").await.unwrap();
        management.send.flush().await.unwrap();
        assert_eq!(
            within("the handler's stream", hears.recv()).await,
            Some((guest_peers.key(), StreamKind::Management, *b"hello"))
        );

        // A kind no one handles is closed for the peer, not left open.
        let mut file = link.open(StreamKind::File).await.unwrap();
        let mut byte = [0u8; 1];
        let read = within("the unhandled stream's end", file.recv.read(&mut byte)).await;
        assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");

        // The daemon's end closes the endpoint and tells the peer why.
        host.shutdown.end();
        assert_eq!(
            within("the connection's end", link.closed()).await,
            CloseCode::Shutdown
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_callers_open_one_endpoint() {
        let machine = Machine::new("windows-pc");
        let (first, second) = tokio::join!(machine.host.open(), machine.host.open());
        assert_eq!(first.unwrap().key(), second.unwrap().key());
        assert_eq!(
            machine.host.peers().unwrap().key(),
            MachineKey::load_or_create(
                &layout::machine_key(machine.dir.path()),
                &hide_platform::host::machine_id().unwrap()
            )
            .unwrap()
            .0
            .public()
        );
    }
}
