//! Two endpoints on this machine's loopback, over real iroh and QUIC
//! (PRD device-connect-iroh D-64). No relay and no lookup service: each
//! side is dialed at the address its endpoint bound.
//!
//! What only a network shows (a relay path, a NAT, a sleep) is the network
//! gate's; these prove the seam carries `hide-peer`'s rules over iroh as
//! the loopback transport does. The one relay here is a listener that
//! answers as the device relay's host does while it is stopped.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use hide_peer::peers::{Config, Incoming, PeerEvent, Peers};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey, StreamKind};
use hide_peer::store::{Host, KnownState, Via};
use hide_peer::{Clock, MachineKey, PeerTransport, RelayState};
use hide_peer_iroh::{IrohConn, IrohTransport, Options};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};

/// The bound after which a wait for an event is a failure. It is never a
/// pause.
const WITHIN: Duration = Duration::from_secs(30);

async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, wait)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

fn local() -> Options {
    Options {
        relay: None,
        mdns: false,
        bind: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
        port: None,
        relay_only: false,
    }
}

struct Machine {
    name: &'static str,
    peers: Peers<IrohTransport>,
    incoming: mpsc::Receiver<Incoming<IrohConn>>,
    events: broadcast::Receiver<PeerEvent>,
    _dir: tempfile::TempDir,
}

impl Machine {
    async fn new(name: &'static str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let key = MachineKey::generate().unwrap();
        let transport = IrohTransport::open(&key.secret(), local()).await.unwrap();
        assert_eq!(
            transport.key(),
            key.public(),
            "iroh connects as the machine key"
        );
        let config = Config {
            state_dir: dir.path(),
            build: "1.3.0-test",
            device: name,
            release: false,
            name: None,
            clock: Clock::system(),
            pacing: Default::default(),
        };
        let (peers, incoming) = Peers::start(transport, config).unwrap();
        let events = peers.events();
        Self {
            name,
            peers,
            incoming,
            events,
            _dir: dir,
        }
    }

    fn key(&self) -> PeerKey {
        self.peers.key()
    }

    /// Where this machine's endpoint listens, as it would tell a peer.
    fn addr(&self) -> PeerAddr {
        let candidates = self.peers.transport().candidates();
        assert!(
            candidates
                .iter()
                .any(|candidate| matches!(candidate, Candidate::Ip(_))),
            "a bound endpoint has an address"
        );
        PeerAddr::new(self.key(), candidates)
    }

    async fn event<T>(&mut self, what: &str, mut pick: impl FnMut(&PeerEvent) -> Option<T>) -> T {
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

    async fn granted(&mut self, of: PeerKey, state: GrantState) {
        if self
            .peers
            .link(&of)
            .is_some_and(|link| link.granted() == state)
        {
            return;
        }
        self.event("the peer's grant", |event| {
            matches!(event, PeerEvent::Granted { key, state: now } if *key == of && *now == state)
                .then_some(())
        })
        .await;
    }
}

/// Makes `guest` a guest of `host`, as a finished pairing leaves them.
fn pair(host: &Machine, guest: &Machine) {
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
                chosen_name: None,
                grant: grant.id,
                state: KnownState::Active,
                candidates: host.addr().candidates,
                started: None,
                ssh: None,
            })
        })
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_connects_opens_streams_and_is_closed_out_when_its_grant_is_removed() {
    let mut host = Machine::new("windows-pc").await;
    let mut guest = Machine::new("macbook").await;
    pair(&host, &guest);

    let link = within("the dial", guest.peers.dial(&host.addr()))
        .await
        .unwrap();
    assert_eq!(link.path(), PathKind::Direct);
    guest.granted(host.key(), GrantState::Active).await;

    // Two kinds of stream on the one connection, each with its bytes.
    let mut keys = link.open(StreamKind::Keys).await.unwrap();
    let mut terminal = link.open(StreamKind::Terminal).await.unwrap();
    keys.send.write_all(b"k").await.unwrap();
    terminal.send.write_all(b"out").await.unwrap();
    let mut got = Vec::new();
    for _ in 0..2 {
        let mut stream = within("a stream the guest opened", host.incoming.recv())
            .await
            .expect("the host's streams")
            .stream;
        let mut bytes = vec![
            0u8;
            if stream.kind == StreamKind::Keys {
                1
            } else {
                3
            }
        ];
        within("the stream's bytes", stream.recv.read_exact(&mut bytes))
            .await
            .unwrap();
        got.push((stream.kind, bytes));
    }
    got.sort_by_key(|(kind, _)| kind.tag());
    assert_eq!(
        got,
        [
            (StreamKind::Keys, b"k".to_vec()),
            (StreamKind::Terminal, b"out".to_vec())
        ]
    );
    // A second dial takes the live connection: no second one is made
    // (iroh#4639).
    assert_eq!(
        guest.peers.dial(&host.addr()).await.unwrap().id(),
        link.id()
    );

    // The host removes the guest: the close code crosses QUIC as written.
    host.peers.revoke(&guest.key(), None).unwrap();
    assert_eq!(
        within("the connection's end", link.closed()).await,
        CloseCode::NoGrant
    );
    let host_key = host.key();
    guest
        .event("the guest's record of the end", |event| {
            matches!(event, PeerEvent::Disconnected { key, code: CloseCode::NoGrant } if *key == host_key)
                .then_some(())
        })
        .await;
    guest.peers.close().await;
    host.peers.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_connection_is_dialed_again_after_it_closed() {
    let host = Machine::new("windows-pc").await;
    let mut guest = Machine::new("macbook").await;
    pair(&host, &guest);
    let host_key = host.key();
    let connected = |event: &PeerEvent| {
        matches!(event, PeerEvent::Connected { key, .. } if *key == host_key).then_some(())
    };

    let _kept = guest.peers.keep(host_key);
    guest.event("the first connection", connected).await;
    let first = guest.peers.link(&host_key).unwrap();

    // The host's end goes away. The guest's connection closes to its end,
    // and only then is the host dialed again, one redial interval later.
    host.peers
        .link(&guest.key())
        .unwrap()
        .close(CloseCode::Lost);
    assert_eq!(
        within("the first connection's end", first.closed()).await,
        CloseCode::Lost
    );
    guest
        .event("the connection after the drop", connected)
        .await;
    let second = guest.peers.link(&host_key).unwrap();
    assert_ne!(second.id(), first.id());
    guest.granted(host_key, GrantState::Active).await;
    second
        .open(StreamKind::Management)
        .await
        .expect("the new connection carries streams");
    guest.peers.close().await;
    host.peers.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rebuilt_endpoint_keeps_its_key_and_accepts_again() {
    let key = MachineKey::generate().unwrap();
    let host = IrohTransport::open(&key.secret(), local()).await.unwrap();
    let guest = IrohTransport::open(&MachineKey::generate().unwrap().secret(), local())
        .await
        .unwrap();
    let before = host.candidates();

    host.rebuild().await.unwrap();
    assert_eq!(host.key(), key.public());
    let after = host.candidates();
    assert_ne!(after, before, "a new endpoint bound a new port");

    let to = PeerAddr::new(host.key(), after);
    let (dialed, accepted) = within("a connection to the rebuilt endpoint", async {
        tokio::join!(guest.dial(&to), host.accept())
    })
    .await;
    use hide_peer::PeerConn;
    let (dialed, accepted) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(accepted.peer(), guest.key());
    assert!(dialed.dialed() && !accepted.dialed());
    guest.close().await;
    host.close().await;
    assert!(matches!(
        host.accept().await,
        Err(hide_peer::TransportError::Closed)
    ));
}

/// A machine starts a handshake with the host and never finishes it: what
/// it sends arrives, and nothing the host answers reaches it. Anyone who
/// can reach the host's UDP port can do this, and a slow network does it
/// by accident. Another machine connects meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handshake_that_never_finishes_holds_up_no_other_connection() {
    use hide_peer::PeerConn;
    use std::sync::Arc;

    let open = || async {
        IrohTransport::open(&MachineKey::generate().unwrap().secret(), local())
            .await
            .unwrap()
    };
    let (host, guest, stuck) = (open().await, open().await, Arc::new(open().await));
    let host_at = host
        .candidates()
        .into_iter()
        .find_map(|candidate| match candidate {
            Candidate::Ip(address) => Some(address),
            Candidate::Relay(_) | Candidate::Wsl(_) | Candidate::Ssh(_) => None,
        })
        .expect("a bound endpoint has an address");

    // A one-way wire to the host: it passes on what is sent to it and
    // drops what the host answers, telling the test of each answer.
    let wire = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let wire_at = wire.local_addr().unwrap();
    let (answered, mut answers) = mpsc::channel(64);
    let wiring = tokio::spawn(async move {
        let mut datagram = vec![0u8; 4096];
        while let Ok((length, from)) = wire.recv_from(&mut datagram).await {
            if from == host_at {
                let _ = answered.try_send(());
            } else {
                let _ = wire.send_to(&datagram[..length], host_at).await;
            }
        }
    });
    let through_the_wire = PeerAddr::new(host.key(), vec![Candidate::Ip(wire_at)]);
    let stalled = tokio::spawn({
        let stuck = Arc::clone(&stuck);
        async move { stuck.dial(&through_the_wire).await.is_ok() }
    });

    // The host began that handshake: its answer went out, and was dropped.
    let accepted = host.accept();
    tokio::pin!(accepted);
    within("the host's answer to the unfinished handshake", async {
        tokio::select! {
            _ = answers.recv() => {}
            _ = &mut accepted => panic!("nothing can have connected yet"),
        }
    })
    .await;

    // Well inside the time the host gives the unfinished handshake: a host
    // that waited for it first would take that whole time to get here.
    let to = PeerAddr::new(host.key(), host.candidates());
    let (dialed, accepted) = tokio::time::timeout(hide_peer::limits::HELLO_WITHIN / 2, async {
        tokio::join!(guest.dial(&to), &mut accepted)
    })
    .await
    .expect("the other machine connected while the host still waited for that handshake");
    assert_eq!(accepted.unwrap().peer(), guest.key());
    dialed.unwrap();
    assert!(!stalled.is_finished(), "that handshake never finished");

    stalled.abort();
    wiring.abort();
    stuck.close().await;
    guest.close().await;
    host.close().await;
}

/// A relay host on this machine's loopback that answers every request with
/// `status` and `body` and relays nothing, and its URL.
async fn relay_host_answering(status: &'static str, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                // The request's head; nothing in it changes the answer.
                let mut head = [0u8; 4096];
                let _ = stream.read(&mut head).await;
                let answer = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    url
}

async fn relay_state_with(relay: Option<String>) -> RelayState {
    let key = MachineKey::generate().unwrap();
    let options = Options { relay, ..local() };
    let transport = IrohTransport::open(&key.secret(), options).await.unwrap();
    let state = within("the relay's state", transport.relay()).await;
    transport.close().await;
    state
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relay_stopped_for_the_month_is_told_apart_from_one_that_is_down() {
    assert_eq!(relay_state_with(None).await, RelayState::None);

    // What the device relay's host answers while it is stopped (B34).
    let stopped = relay_host_answering(
        "503 Service Unavailable",
        "device relay stopped: monthly limit reached",
    )
    .await;
    assert_eq!(
        relay_state_with(Some(stopped)).await,
        RelayState::MonthlyLimit
    );

    // A 503 for another reason, and a host that answers something else, are
    // a relay that cannot be reached.
    let overloaded = relay_host_answering("503 Service Unavailable", "no upstream").await;
    assert_eq!(
        relay_state_with(Some(overloaded)).await,
        RelayState::Unreachable
    );
    let missing = relay_host_answering("404 Not Found", "monthly limit reached").await;
    assert_eq!(
        relay_state_with(Some(missing)).await,
        RelayState::Unreachable
    );
}

/// The product endpoint binds the state folder's port on every interface,
/// the one port the Windows firewall rule lets in (PRD D-38): while it is
/// open, nothing else can have that port.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_endpoint_given_a_port_holds_it_on_every_interface() {
    let free = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let key = MachineKey::generate().unwrap();
    let options = Options {
        relay: None,
        mdns: false,
        bind: None,
        port: Some(free),
        relay_only: false,
    };
    let transport = within("the endpoint", IrohTransport::open(&key.secret(), options))
        .await
        .unwrap();
    let taken = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, free)).unwrap_err();
    assert_eq!(taken.kind(), std::io::ErrorKind::AddrInUse);
    drop(transport);
}
