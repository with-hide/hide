//! A WSL distribution and its PC (PRD device-connect-iroh D-27, D-62,
//! D-63): the same pairing, grants and streams as any two machines, on the
//! one pipe `wsl.exe` gives, beside each machine's other transport.
//!
//! The pipe is a local stream pair here, and what stands for `wsl.exe` is
//! the PC's dial handing the distribution's core the far end.

use hide_kit::layout;
use hide_peer::loopback::{Loopback, Net};
use hide_peer::mux::Carrier;
use hide_peer::peers::{Config, Incoming, PeerEvent, Peers};
use hide_peer::pipe::{Beside, PipeTransport};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::{Candidate, CloseCode, PathKind, PeerAddr, StreamKind};
use hide_peer::store::{Host, KnownState, Via};
use hide_peer::{Clock, MachineKey, PeerTransport};
use hide_platform::ipc::LocalStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::within;

type Transport = Beside<Loopback>;
type Conn = <Transport as PeerTransport>::Conn;

struct Machine {
    peers: Peers<Transport>,
    incoming: mpsc::Receiver<Incoming<Conn>>,
    _dir: tempfile::TempDir,
}

fn keys(dir: &tempfile::TempDir, name: &str) -> (MachineKey, MachineKey) {
    let (key, _) = MachineKey::load_or_create(&layout::machine_key(dir.path()), name).unwrap();
    let twin = MachineKey::from_secret(&key.secret());
    (key, twin)
}

fn machine(name: &'static str, dir: tempfile::TempDir, transport: Transport) -> Machine {
    let config = Config {
        state_dir: dir.path(),
        build: "1.3.0-test",
        device: name,
        release: false,
        name: None,
        clock: Clock::system(),
        pacing: Default::default(),
        capabilities: hide_peer::protocol::hello::CAPABILITIES,
    };
    let (peers, incoming) = Peers::start(transport, config).unwrap();
    Machine {
        peers,
        incoming,
        _dir: dir,
    }
}

/// A distribution, and the PC whose every pipe arrives at it. Each is on a
/// wire of its own, so nothing but the pipe joins them.
fn pc_and_distribution() -> (Machine, Machine) {
    let distro_dir = tempfile::tempdir().unwrap();
    let (key, twin) = keys(&distro_dir, "ubuntu");
    let distro = machine(
        "ubuntu",
        distro_dir,
        Beside::new(Net::new().join(key), PipeTransport::accepting(twin)),
    );
    let pc_dir = tempfile::tempdir().unwrap();
    let (key, twin) = keys(&pc_dir, "windows-pc");
    let arrives_at = distro.peers.clone();
    let pipe = PipeTransport::dialing(twin, move |_| {
        let (near, far) = LocalStream::pair().unwrap();
        arrives_at
            .transport()
            .pipe()
            .arrived(far.into(), hide_peer::protocol::PathKind::Local);
        async move { Ok(Carrier::from(near)) }
    });
    let pc = machine(
        "windows-pc",
        pc_dir,
        Beside::new(Net::new().join(key), pipe),
    );
    (pc, distro)
}

fn in_wsl(machine: &Machine) -> PeerAddr {
    PeerAddr::new(
        machine.peers.key(),
        vec![Candidate::Wsl("Ubuntu".to_owned())],
    )
}

/// Makes the PC a guest of the distribution through the stores, as a
/// pairing over `wsl.exe` leaves them: the PC knows the distribution by
/// its name.
fn pair(pc: &Machine, distro: &Machine) {
    let grant = distro
        .peers
        .grant_pending(pc.peers.key(), "windows-pc", "windows-pc", Via::Wsl)
        .unwrap();
    distro.peers.confirm(&pc.peers.key(), &grant.id).unwrap();
    pc.peers
        .known(|known| {
            known.put(Host {
                key: distro.peers.key(),
                device: "ubuntu".to_owned(),
                name: "ubuntu".to_owned(),
                chosen_name: None,
                grant: grant.id,
                state: KnownState::Active,
                candidates: in_wsl(distro).candidates,
                started: None,
                ssh: None,
            })
        })
        .unwrap();
}

async fn connected(machine: &Machine, to: &Machine) {
    let key = to.peers.key();
    let mut events = machine.peers.events();
    within("the connection", async {
        while machine.peers.link(&key).is_none() {
            let _ = events.recv().await;
        }
    })
    .await;
}

/// The PC, paired with its distribution, connects over the pipe, is
/// admitted by its grant, and runs a stream there; the distribution's core
/// reads the stream's kind and bytes as on any connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pc_runs_a_stream_in_its_distribution_over_the_pipe() {
    let (pc, mut distro) = pc_and_distribution();
    pair(&pc, &distro);
    let told = pc.peers.events();
    let link = within("the dial", pc.peers.dial(&in_wsl(&distro)))
        .await
        .unwrap();
    assert_eq!(link.path(), PathKind::Local);
    assert!(link.dialed());
    within("the grant", async {
        let mut told = told;
        while link.granted() != GrantState::Active {
            let _ = told.recv().await;
        }
    })
    .await;

    let mut keys = within("the stream", link.open(StreamKind::Keys))
        .await
        .unwrap();
    keys.send.write_all(b"ls\n").await.unwrap();
    keys.send.flush().await.unwrap();
    let mut theirs = within("the stream there", async {
        loop {
            let incoming = distro.incoming.recv().await.expect("the streams");
            if incoming.stream.kind == StreamKind::Keys {
                assert_eq!(incoming.link.key(), pc.peers.key());
                assert_eq!(incoming.link.path(), PathKind::Local);
                break incoming.stream;
            }
        }
    })
    .await;
    let mut typed = [0_u8; 3];
    within("the keys", theirs.recv.read_exact(&mut typed))
        .await
        .unwrap();
    assert_eq!(&typed, b"ls\n");
}

/// A pipe that ends is a connection lost, and the PC dials its
/// distribution again by the name it saved, as it dials any host it is a
/// guest of.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipe_that_ends_is_dialed_again() {
    let (pc, distro) = pc_and_distribution();
    pair(&pc, &distro);
    let _kept = pc.peers.keep(distro.peers.key());
    connected(&pc, &distro).await;
    let first = pc.peers.link(&distro.peers.key()).unwrap();
    let mut events = pc.peers.events();
    // The distribution's core ends its end, as a distribution that stops
    // does.
    connected(&distro, &pc).await;
    let theirs = distro.peers.link(&pc.peers.key()).unwrap();
    theirs.close(CloseCode::Lost);
    within("the reconnection", async {
        loop {
            match events.recv().await {
                Ok(PeerEvent::Connected { key, .. }) if key == distro.peers.key() => {
                    let now = pc.peers.link(&key).expect("connected");
                    if now.id() != first.id() {
                        break;
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("the PC's events ended: {error}"),
            }
        }
    })
    .await;
    // The distribution never dials: only its PC makes a pipe (D-62).
    assert!(matches!(
        distro.peers.transport().pipe().dial(&in_wsl(&pc)).await,
        Err(hide_peer::TransportError::Unreachable(_))
    ));
}
