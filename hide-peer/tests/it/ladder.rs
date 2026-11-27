//! The SSH path beside a machine's other transport:
//! the first dial's ladder, the climb that moves a live connection, the
//! probe for a direct path, and the beat on an SSH connection.
//!
//! The other transport is the loopback, whose connections start on the path
//! a test chooses; an SSH carrier is a local stream pair whose far end the
//! host's pipe takes, as its daemon takes what `hide device carry --ssh`
//! hands it. Its route is the test's to set. The pipe runs on threads of its
//! own, so these tests read the real clock, each bounded by `within`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use hide_kit::layout;
use hide_peer::loopback::{Loopback, Net};
use hide_peer::mux::Carrier;
use hide_peer::peers::{Config, Incoming, Keeper, Pacing, PeerEvent, Peers};
use hide_peer::pipe::{Beside, PipeTransport, Routes};
use hide_peer::protocol::{CloseCode, PathKind, PeerAddr, PeerKey};
use hide_peer::store::{Host, KnownState, SshRoute, Via};
use hide_peer::{Clock, MachineKey, PeerTransport, Route};
use hide_platform::ipc::LocalStream;
use tokio::sync::mpsc;

use crate::within;

type Transport = Beside<Loopback>;
type Conn = <Transport as PeerTransport>::Conn;

/// The route the guest reads, set by the test, and what the climb told it.
#[derive(Default)]
struct TestRoutes {
    route: Mutex<Option<Route>>,
    fell: AtomicUsize,
    stayed: AtomicUsize,
}

impl TestRoutes {
    fn set(&self, route: Route) {
        *self.route.lock().unwrap() = Some(route);
    }

    fn fell(&self) -> usize {
        self.fell.load(Ordering::Acquire)
    }
}

impl Routes for TestRoutes {
    fn route(&self, _key: &PeerKey, _alias: &str) -> Route {
        self.route.lock().unwrap().unwrap_or(Route::Ready)
    }

    fn fell(&self, _key: &PeerKey) {
        self.fell.fetch_add(1, Ordering::AcqRel);
        let mut route = self.route.lock().unwrap();
        if matches!(*route, None | Some(Route::Ready)) {
            *route = Some(Route::Held);
        }
    }

    fn stayed(&self, _key: &PeerKey) {
        self.stayed.fetch_add(1, Ordering::AcqRel);
    }

    fn network_changed(&self) {
        let mut route = self.route.lock().unwrap();
        if *route == Some(Route::Held) {
            *route = Some(Route::Due);
        }
    }
}

/// Where the guest's SSH carriers arrive, how many sign-ins it made, how
/// late each answers, and whether a carrier stops passing bytes.
#[derive(Default)]
struct TestSsh {
    far: Mutex<Option<Peers<Transport>>>,
    sign_ins: AtomicUsize,
    late: Mutex<Duration>,
    fails: AtomicBool,
    /// Carriers whose bytes stopped, the guest's end closed or the host's.
    ended: Arc<AtomicUsize>,
    /// Held: a carrier made from now passes no byte while it is set.
    frozen: Arc<(Mutex<bool>, Condvar)>,
}

impl TestSsh {
    fn sign_ins(&self) -> usize {
        self.sign_ins.load(Ordering::Acquire)
    }

    fn ended(&self) -> usize {
        self.ended.load(Ordering::Acquire)
    }

    fn freeze(&self, frozen: bool) {
        let (held, changed) = &*self.frozen;
        *held.lock().unwrap() = frozen;
        changed.notify_all();
    }
}

/// Copies `from` to `to` a chunk at a time, holding each chunk while the
/// carrier is frozen, until either end closes; counts its end in `ended`.
fn copy(
    mut from: LocalStream,
    mut to: LocalStream,
    frozen: Arc<(Mutex<bool>, Condvar)>,
    ended: Option<Arc<AtomicUsize>>,
) {
    use std::io::{Read, Write};
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = match from.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let (held, changed) = &*frozen;
            let mut held = held.lock().unwrap();
            while *held {
                held = changed.wait(held).unwrap();
            }
            drop(held);
            if to.write_all(&buffer[..read]).is_err() {
                break;
            }
        }
        to.shutdown_handle().shutdown();
        if let Some(ended) = ended {
            ended.fetch_add(1, Ordering::AcqRel);
        }
    });
}

struct Machine {
    peers: Peers<Transport>,
    _incoming: mpsc::Receiver<Incoming<Conn>>,
    _dir: tempfile::TempDir,
}

fn keys(dir: &tempfile::TempDir, name: &str) -> (MachineKey, MachineKey) {
    let (key, _) = MachineKey::load_or_create(&layout::machine_key(dir.path()), name).unwrap();
    let twin = MachineKey::from_secret(&key.secret());
    (key, twin)
}

fn started(
    name: &'static str,
    dir: tempfile::TempDir,
    transport: Transport,
    pacing: Pacing,
) -> Machine {
    let config = Config {
        state_dir: dir.path(),
        build: "1.3.0-test",
        device: name,
        release: false,
        name: None,
        clock: Clock::system(),
        pacing,
    };
    let (peers, incoming) = Peers::start(transport, config).unwrap();
    Machine {
        peers,
        _incoming: incoming,
        _dir: dir,
    }
}

/// A guest that dials SSH carriers to its host, and the host whose pipe
/// takes them, on one loopback wire; `pacing` is both machines'.
struct Pair {
    net: Arc<Net>,
    guest: Machine,
    host: Machine,
    routes: Arc<TestRoutes>,
    ssh: Arc<TestSsh>,
}

fn pair(pacing: Pacing) -> Pair {
    pair_by(pacing, Some("studio"))
}

/// A pair whose guest keeps the host by `alias`, or by none.
fn pair_by(pacing: Pacing, alias: Option<&str>) -> Pair {
    let net = Net::new();
    let host_dir = tempfile::tempdir().unwrap();
    let (key, twin) = keys(&host_dir, "studio");
    let host = started(
        "studio",
        host_dir,
        Beside::new(net.join(key), PipeTransport::accepting(twin)),
        pacing,
    );
    let routes = Arc::new(TestRoutes::default());
    let ssh = Arc::new(TestSsh::default());
    *ssh.far.lock().unwrap() = Some(host.peers.clone());
    let guest_dir = tempfile::tempdir().unwrap();
    let (key, twin) = keys(&guest_dir, "macbook");
    let dialing = Arc::clone(&ssh);
    let pipe = PipeTransport::dialing(twin, move |_| {
        let ssh = Arc::clone(&dialing);
        async move {
            ssh.sign_ins.fetch_add(1, Ordering::AcqRel);
            let late = *ssh.late.lock().unwrap();
            // A sign-in that answers late is what these tests are about.
            tokio::time::sleep(late).await; // wait-ok: the stand-in's sign-in takes this long
            if ssh.fails.load(Ordering::Acquire) {
                return Err(std::io::Error::other("ssh did not sign in"));
            }
            let far = ssh.far.lock().unwrap().clone().expect("the host");
            let (near, near_far) = LocalStream::pair()?;
            let (far_near, far_end) = LocalStream::pair()?;
            copy(
                near_far.duplicate(),
                far_near.duplicate(),
                Arc::clone(&ssh.frozen),
                None,
            );
            copy(
                far_near,
                near_far,
                Arc::clone(&ssh.frozen),
                Some(Arc::clone(&ssh.ended)),
            );
            far.transport()
                .pipe()
                .arrived(far_end.into(), PathKind::Ssh);
            Ok(Carrier::from(near))
        }
    })
    .with_routes(Arc::clone(&routes) as Arc<dyn Routes>);
    let guest = started(
        "macbook",
        guest_dir,
        Beside::new(net.join(key), pipe),
        pacing,
    );
    // The host grants the guest, and the guest keeps the host with the
    // alias it signs in to it by.
    let grant = host
        .peers
        .grant_pending(guest.peers.key(), "macbook", "macbook", Via::Ssh)
        .unwrap();
    host.peers.confirm(&guest.peers.key(), &grant.id).unwrap();
    guest
        .peers
        .known(|known| {
            known.put(Host {
                key: host.peers.key(),
                device: "studio".to_owned(),
                name: "studio".to_owned(),
                chosen_name: None,
                grant: grant.id,
                state: KnownState::Active,
                candidates: Vec::new(),
                started: None,
                ssh: alias.map(|alias| SshRoute::Alias(alias.to_owned())),
            })
        })
        .unwrap();
    Pair {
        net,
        guest,
        host,
        routes,
        ssh,
    }
}

impl Pair {
    /// The address the guest's keeper dials: the alias last.
    fn to_host(&self) -> PeerAddr {
        self.guest
            .peers
            .host(&self.host.peers.key())
            .unwrap()
            .dial_addr()
    }

    fn main(&self) -> &Loopback {
        self.guest.peers.transport().main()
    }

    fn keep(&self) -> Keeper {
        self.guest.peers.keep(self.host.peers.key())
    }

    /// Waits until both machines hold one live connection on `path`, the
    /// same one, and answers its name.
    async fn held_on(&self, path: PathKind) -> u64 {
        let (guest, host) = (self.guest.peers.clone(), self.host.peers.clone());
        let (to_host, to_guest) = (host.key(), guest.key());
        let mut events = guest.events();
        within(&format!("both machines on {path:?}"), async {
            loop {
                let both = (guest.link(&to_host), host.link(&to_guest));
                if let (Some(ours), Some(theirs)) = both
                    && ours.path() == path
                    && theirs.path() == path
                    && ours.dial_name() == theirs.dial_name()
                {
                    return ours.dial_name();
                }
                let _ = tokio::time::timeout(Duration::from_millis(50), events.recv()).await; // wait-ok: the other end's install is not an event of this one
            }
        })
        .await
    }
}

fn quick() -> Pacing {
    Pacing {
        relay_grace: Duration::from_millis(200),
        climb_first: Duration::from_millis(300),
        climb_most: Duration::from_secs(2),
        ..Pacing::default()
    }
}

/// A path the other transport proves direct before the SSH head start ends
/// is taken at once, and no SSH sign-in is made.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_direct_path_inside_the_window_makes_no_ssh_dial() {
    let pair = pair(Pacing::default());
    let link = within("the dial", pair.guest.peers.dial(&pair.to_host()))
        .await
        .unwrap();
    assert_eq!(link.path(), PathKind::Local);
    assert_eq!(pair.ssh.sign_ins(), 0);
}

/// With only the relay beside it, a proved SSH connection is taken at the
/// end of the window, and the relay connection is closed before its first
/// exchange: the host never admits it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ssh_carrier_beats_a_relay_connection_and_the_relay_one_is_never_admitted() {
    let pair = pair(Pacing::default());
    pair.main().dial_on(PathKind::Relay);
    let mut told = pair.host.peers.events();
    let link = within("the dial", pair.guest.peers.dial(&pair.to_host()))
        .await
        .unwrap();
    assert_eq!(link.path(), PathKind::Ssh);
    assert_eq!(pair.ssh.sign_ins(), 1);
    pair.held_on(PathKind::Ssh).await;
    while let Ok(event) = told.try_recv() {
        assert!(
            !matches!(
                event,
                PeerEvent::Connected {
                    path: PathKind::Relay,
                    ..
                }
            ),
            "the host admitted the relay connection: {event:?}"
        );
    }
}

/// With nothing else reaching the host, SSH carries the pair: a ready
/// route through the ladder, a due one after the failed dial.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ssh_carries_the_pair_when_nothing_else_connects() {
    for route in [Route::Ready, Route::Due] {
        let pair = pair(Pacing::default());
        pair.main().set_unreachable(true);
        pair.routes.set(route);
        let link = within("the dial", pair.guest.peers.dial(&pair.to_host()))
            .await
            .unwrap_or_else(|error| panic!("{route:?}: {error}"));
        assert_eq!(link.path(), PathKind::Ssh, "{route:?}");
    }
}

/// A route that waits or is parked adds nothing to a dial: the other
/// transport's connection is taken as it comes, with no sign-in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_route_that_waits_adds_no_wait_to_a_dial() {
    let pair = pair(Pacing::default());
    pair.main().dial_on(PathKind::Relay);
    pair.routes.set(Route::Held);
    let started = tokio::time::Instant::now();
    let link = within("the dial", pair.guest.peers.dial(&pair.to_host()))
        .await
        .unwrap();
    assert_eq!(link.path(), PathKind::Relay);
    assert!(started.elapsed() < hide_peer::limits::SSH_AFTER);
    assert_eq!(pair.ssh.sign_ins(), 0);
}

/// A sign-in slower than the window: the relay connection is taken at its
/// end, and the climb takes that same sign-in as its move, with no second
/// one, on both machines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_ssh_sign_in_is_taken_by_the_climb_not_made_again() {
    let pair = pair(quick());
    pair.main().dial_on(PathKind::Relay);
    *pair.ssh.late.lock().unwrap() = hide_peer::limits::DIRECT_WITHIN + Duration::from_secs(1);
    let _keeper = pair.keep();
    pair.held_on(PathKind::Relay).await;
    pair.held_on(PathKind::Ssh).await;
    assert_eq!(pair.ssh.sign_ins(), 1);
}

/// A relay connection climbs to SSH once its route is ready and the relay
/// was read for the grace; both machines hold the one SSH connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relay_connection_moves_to_ssh_on_both_machines() {
    let pair = pair(quick());
    pair.main().dial_on(PathKind::Relay);
    pair.routes.set(Route::Held);
    let _keeper = pair.keep();
    let relay = pair.held_on(PathKind::Relay).await;
    assert_eq!(pair.ssh.sign_ins(), 0);
    pair.routes.set(Route::Ready);
    let ssh = pair.held_on(PathKind::Ssh).await;
    assert_ne!(relay, ssh);
    assert_eq!(pair.ssh.sign_ins(), 1);
}

/// A hole punch that lands while the SSH sign-in runs keeps its direct
/// path: the SSH connection does not beat it, so it is closed unadmitted,
/// and the route waits once, as after a fall, so the next relay connection
/// does not sign in at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_never_replaces_a_connection_that_became_direct_meanwhile() {
    let pair = pair(quick());
    pair.main().dial_on(PathKind::Relay);
    pair.routes.set(Route::Held);
    *pair.ssh.late.lock().unwrap() = Duration::from_secs(1);
    let _keeper = pair.keep();
    let relay = pair.held_on(PathKind::Relay).await;
    pair.routes.set(Route::Ready);
    within("the sign-in", async {
        while pair.ssh.sign_ins() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    pair.main().move_path(PathKind::Direct);
    pair.host
        .peers
        .transport()
        .main()
        .move_path(PathKind::Direct);
    let mut host_told = pair.host.peers.events();
    within("the move given up, counted once", async {
        while pair.routes.fell() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    within("its carrier ended", async {
        while pair.ssh.ended() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(pair.held_on(PathKind::Direct).await, relay);
    assert_eq!(
        pair.routes.fell(),
        1,
        "a move that was not taken waits once"
    );
    while let Ok(event) = host_told.try_recv() {
        assert!(
            !matches!(
                event,
                PeerEvent::Connected {
                    path: PathKind::Ssh,
                    ..
                }
            ),
            "the host admitted the SSH connection: {event:?}"
        );
    }
}

/// An alias the operator records for a host already connected over the
/// relay is climbed to while that connection lives: both machines move to
/// SSH, and the relay connection is not dropped first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alias_recorded_on_a_relay_connection_is_climbed_to() {
    let pair = pair_by(quick(), None);
    pair.main().dial_on(PathKind::Relay);
    let _keeper = pair.keep();
    let relay = pair.held_on(PathKind::Relay).await;
    assert_eq!(pair.ssh.sign_ins(), 0);
    let mut guest_told = pair.guest.peers.events();
    let host = pair.host.peers.key();
    pair.guest
        .peers
        .known(|known| known.record_ssh(&host, "studio"))
        .unwrap();
    let ssh = pair.held_on(PathKind::Ssh).await;
    assert_ne!(relay, ssh);
    assert_eq!(pair.ssh.sign_ins(), 1);
    // The relay connection ended only as the one the move replaced.
    while let Ok(event) = guest_told.try_recv() {
        assert!(
            !matches!(
                event,
                PeerEvent::Disconnected { code, .. } if code != CloseCode::Replaced
            ),
            "the pair was disconnected on the way: {event:?}"
        );
    }
}

/// A due route is signed in to when the other transport's dial neither
/// connects nor fails: it is given the window a first dial gives it, not
/// the whole dial's bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_due_route_is_tried_when_the_other_dial_never_answers() {
    for route in [Route::Ready, Route::Due] {
        let pair = pair(Pacing::default());
        pair.main().set_silent(true);
        pair.routes.set(route);
        let started = tokio::time::Instant::now();
        let link = within("the dial", pair.guest.peers.dial(&pair.to_host()))
            .await
            .unwrap_or_else(|error| panic!("{route:?}: {error}"));
        assert_eq!(link.path(), PathKind::Ssh, "{route:?}");
        assert!(
            started.elapsed() < hide_peer::limits::DIAL_WITHIN,
            "{route:?} waited for the whole dial"
        );
    }
}

/// A climb whose sign-in fails tells the route nothing more: the attempt
/// counted its own failure, so the route's wait doubles once per failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_sign_in_on_the_climb_is_counted_once() {
    let pair = pair(quick());
    pair.main().dial_on(PathKind::Relay);
    pair.routes.set(Route::Held);
    pair.ssh.fails.store(true, Ordering::Release);
    let _keeper = pair.keep();
    pair.held_on(PathKind::Relay).await;
    pair.routes.set(Route::Ready);
    // The stand-in route stays ready, so a second sign-in means the first
    // climb ended.
    within("two climbs", async {
        while pair.ssh.sign_ins() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(pair.routes.fell(), 0);
}

/// The side that dialed SSH looks for a direct path beside it: a probe
/// that reads the relay is closed and waits longer, and one that reads a
/// direct path moves the pair there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ssh_connection_moves_to_a_direct_path_that_appears() {
    let pair = pair(quick());
    pair.main().set_unreachable(true);
    let _keeper = pair.keep();
    pair.held_on(PathKind::Ssh).await;
    // A probe now reaches the host over the relay only: two of them are
    // closed before their first exchange, and the pair stays on SSH.
    pair.main().dial_on(PathKind::Relay);
    pair.main().set_unreachable(false);
    let before = pair.net.dials();
    within("two probes", async {
        while pair.net.dials() < before + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        pair.guest
            .peers
            .link(&pair.host.peers.key())
            .unwrap()
            .path()
            == PathKind::Ssh
    );
    pair.main().dial_on(PathKind::Direct);
    pair.held_on(PathKind::Direct).await;
}

/// A network change probes an SSH connection at once, without waiting for
/// the climb's first wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_network_change_probes_an_ssh_connection_at_once() {
    let pair = pair(Pacing::default());
    pair.main().set_unreachable(true);
    let _keeper = pair.keep();
    pair.held_on(PathKind::Ssh).await;
    pair.main().set_unreachable(false);
    pair.guest.peers.network_changed();
    pair.held_on(PathKind::Local).await;
}

/// A carrier that stops passing bytes is closed on both machines once the
/// beat went unheard, and the guest dials again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_carrier_that_goes_silent_is_closed_after_beat_dead() {
    let pacing = Pacing {
        beat_every: Duration::from_millis(100),
        beat_dead: Duration::from_millis(500),
        ..Pacing::default()
    };
    let pair = pair(pacing);
    pair.main().set_unreachable(true);
    let mut guest_told = pair.guest.peers.events();
    let mut host_told = pair.host.peers.events();
    let _keeper = pair.keep();
    pair.held_on(PathKind::Ssh).await;
    // Beats keep it while the carrier passes them.
    let lost = |told: &mut tokio::sync::broadcast::Receiver<PeerEvent>| {
        let mut told = told.resubscribe();
        async move {
            loop {
                if let Ok(PeerEvent::Disconnected { code, .. }) = told.recv().await {
                    return code;
                }
            }
        }
    };
    let kept = tokio::time::timeout(pacing.beat_dead * 3, lost(&mut guest_told)).await; // wait-ok: three beat windows with the carrier passing bytes
    assert!(kept.is_err(), "a beating connection ended: {kept:?}");
    pair.ssh.freeze(true);
    let (guest, host) = tokio::join!(
        within("the guest's end", lost(&mut guest_told)),
        within("the host's end", lost(&mut host_told)),
    );
    assert_eq!((guest, host), (CloseCode::Lost, CloseCode::Lost));
    pair.ssh.freeze(false);
}
