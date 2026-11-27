//! The SSH routes against a stand-in signer: each carrier is one end of a
//! local stream whose other end a far pipe transport takes, as the far
//! daemon takes what `hide device carry --ssh` hands it, so the key proof
//! that settles a route is the real one.

use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;

use hide_node_link::device::SshFailure;
use hide_peer::MachineKey;
use hide_peer::pipe::PipeTransport;
use hide_peer::protocol::{Candidate, PathKind, PeerAddr};
use hide_peer::{PeerConn, PeerTransport};
use hide_platform::ipc::LocalStream;

use super::*;

/// A hang guard, never a pause.
const WITHIN: Duration = Duration::from_secs(20);

async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, wait)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

/// What the stand-in does at its next sign-in.
#[derive(Clone)]
enum Next {
    /// Signs in and hands the carrier to the far machine's transport.
    Carry,
    /// `ssh` stops on this problem.
    Fails(SshProblem),
    /// The far command writes this line and ends before any proof.
    Says(&'static str),
    /// Never answers until its attempt is stopped.
    Stalls,
}

struct Stand {
    far: Mutex<Option<Arc<PipeTransport>>>,
    next: Mutex<Next>,
    /// Sign-ins started, and the ones running now.
    started: AtomicUsize,
    running: AtomicUsize,
    /// Told once a stalled sign-in was stopped.
    stopped: Mutex<Option<mpsc::Sender<()>>>,
    /// How many of the attempts it carried were stopped.
    stops: Arc<AtomicUsize>,
}

impl Stand {
    fn new(far: &Arc<PipeTransport>) -> Arc<Self> {
        Arc::new(Self {
            far: Mutex::new(Some(Arc::clone(far))),
            next: Mutex::new(Next::Carry),
            started: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            stopped: Mutex::new(None),
            stops: Arc::default(),
        })
    }

    fn next(&self, next: Next) {
        *lock(&self.next) = next;
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::Acquire)
    }
}

struct Running<'a>(&'a AtomicUsize);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Signer for Stand {
    fn carry(&self, _alias: &str, stop: &Stop) -> Result<AttachChannel, SessionError> {
        self.started.fetch_add(1, Ordering::AcqRel);
        self.running.fetch_add(1, Ordering::AcqRel);
        let _running = Running(&self.running);
        let (ended, end) = mpsc::channel::<()>();
        let stops = Arc::clone(&self.stops);
        stop.set(move || {
            stops.fetch_add(1, Ordering::AcqRel);
            let _ = ended.send(());
        });
        let said = Arc::new(Mutex::new(Vec::new()));
        match lock(&self.next).clone() {
            Next::Carry => {
                let (near, far) =
                    LocalStream::pair().map_err(|e| SessionError::Local(e.to_string()))?;
                let transport = lock(&self.far).clone().expect("a far machine");
                transport.arrived(far.into(), PathKind::Ssh);
                Ok(AttachChannel {
                    stream: near,
                    stderr: said,
                })
            }
            Next::Fails(problem) => Err(SessionError::Ssh(SshFailure {
                problem,
                detail: String::new(),
            })),
            Next::Says(line) => {
                let (near, far) =
                    LocalStream::pair().map_err(|e| SessionError::Local(e.to_string()))?;
                lock(&said).extend_from_slice(format!("{line}\n").as_bytes());
                drop(far);
                Ok(AttachChannel {
                    stream: near,
                    stderr: said,
                })
            }
            Next::Stalls => {
                let _ = end.recv();
                if let Some(told) = lock(&self.stopped).take() {
                    let _ = told.send(());
                }
                Err(SessionError::Lost("stopped".to_owned()))
            }
        }
    }
}

fn key() -> MachineKey {
    MachineKey::generate().unwrap()
}

/// This machine with SSH carriers over `stand`, and a pipe transport that
/// dials through them; the far machine's key is `far`.
struct Near {
    carriers: Arc<SshCarriers>,
    pipe: PipeTransport,
    /// What the records were asked to turn off, and whether saving fails.
    turned_off: Arc<Mutex<Vec<(PeerKey, String)>>>,
    save_fails: Arc<AtomicBool>,
}

fn this_machine(stand: &Arc<Stand>) -> Near {
    let carriers = SshCarriers::with_signer(Arc::clone(stand) as Arc<dyn Signer>);
    let turned_off = Arc::new(Mutex::new(Vec::new()));
    let save_fails = Arc::new(AtomicBool::new(false));
    {
        let (turned_off, save_fails) = (Arc::clone(&turned_off), Arc::clone(&save_fails));
        carriers.store_in(move |key, alias| {
            if save_fails.load(Ordering::Acquire) {
                return Err(StoreError::Io(io::Error::other("the disk is full")));
            }
            lock(&turned_off).push((*key, alias.to_owned()));
            Ok(true)
        });
    }
    let dialing = Arc::clone(&carriers);
    let pipe = PipeTransport::dialing(key(), move |to| {
        let alias = hide_peer::protocol::addr::ssh_alias(to).unwrap().to_owned();
        dialing.carrier(to.key, alias)
    })
    .with_routes(Arc::clone(&carriers) as Arc<dyn Routes>);
    Near {
        carriers,
        pipe,
        turned_off,
        save_fails,
    }
}

fn over(key: PeerKey, alias: &str) -> PeerAddr {
    PeerAddr::new(key, vec![Candidate::Ssh(alias.to_owned())])
}

fn route(near: &Near, key: &PeerKey, alias: &str) -> Route {
    Routes::route(near.carriers.as_ref(), key, alias)
}

/// A route is ready until an attempt says otherwise; a proved carrier
/// connects, says its path is SSH, and keeps the route ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_proved_carrier_keeps_its_route_ready() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
    let (dialed, accepted) = within("the connection", async {
        tokio::join!(near.pipe.dial(&to), far.accept())
    })
    .await;
    let (ours, theirs) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!((ours.path(), theirs.path()), (PathKind::Ssh, PathKind::Ssh));
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
    assert_eq!(near.carriers.blocked(&to.key), None);
}

/// Each way a sign-in fails moves the route as the device row's rule says:
/// what Hide retries waits, the rest parks with what the operator does, and
/// a parked or waiting route refuses at once without starting `ssh`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_failure_moves_the_route_by_the_rule_a_device_row_reads() {
    let far = Arc::new(PipeTransport::accepting(key()));
    for (problem, parked) in [
        (SshProblem::Unreachable, None),
        (SshProblem::NameNotFound, None),
        (SshProblem::SignInNeeded, Some(PeerSshBlocked::SignIn)),
        (
            SshProblem::HostKeyChanged,
            Some(PeerSshBlocked::CheckHostKey),
        ),
        (SshProblem::HideUpdated, Some(PeerSshBlocked::ReopenHide)),
        (SshProblem::PolicyRefused, Some(PeerSshBlocked::Retry)),
    ] {
        let stand = Stand::new(&far);
        let near = this_machine(&stand);
        let to = over(far.key(), "studio");
        stand.next(Next::Fails(problem));
        assert!(within("the failure", near.pipe.dial(&to)).await.is_err());
        assert_eq!(near.carriers.blocked(&to.key), parked, "{problem:?}");
        assert_eq!(route(&near, &to.key, "studio"), Route::Held, "{problem:?}");
        // Held: refused at once, with no second sign-in.
        assert!(near.pipe.dial(&to).await.is_err());
        assert_eq!(stand.started(), 1, "{problem:?}");
        // The operator's retry tries it again.
        near.carriers.retry(&to.key);
        assert_eq!(route(&near, &to.key, "studio"), Route::Ready, "{problem:?}");
    }
}

/// A route that waits is due once its wait ran out or the network changed,
/// and its wait doubles with each failure; a parked one stays parked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_route_is_due_after_its_wait_or_a_network_change() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    stand.next(Next::Fails(SshProblem::Unreachable));
    assert!(within("the failure", near.pipe.dial(&to)).await.is_err());
    assert_eq!(route(&near, &to.key, "studio"), Route::Held);
    Routes::network_changed(near.carriers.as_ref());
    assert_eq!(route(&near, &to.key, "studio"), Route::Due);
    // Tried again while due, it fails again and waits again.
    assert!(within("the failure", near.pipe.dial(&to)).await.is_err());
    assert_eq!(stand.started(), 2);
    assert_eq!(route(&near, &to.key, "studio"), Route::Held);
    {
        let routes = lock(&near.carriers.routes);
        let entry = routes.get(&to.key).unwrap();
        assert_eq!(entry.wait, limits::CLIMB_FIRST * 2);
    }
    // A parked route is no due one, whatever the network does.
    stand.next(Next::Fails(SshProblem::SignInNeeded));
    near.carriers.retry(&to.key);
    assert!(within("the failure", near.pipe.dial(&to)).await.is_err());
    Routes::network_changed(near.carriers.as_ref());
    assert_eq!(route(&near, &to.key, "studio"), Route::Held);
    assert_eq!(near.carriers.blocked(&to.key), Some(PeerSshBlocked::SignIn));
    // The app opening is when a sign-in may ask.
    near.carriers.app_opened();
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
}

/// The route's state once an attempt settled it.
async fn settled(near: &Near, key: &PeerKey) -> State {
    let mut changed = near.carriers.changed();
    within("the far line", async {
        loop {
            let state = lock(&near.carriers.routes)
                .get(key)
                .map(|entry| entry.state);
            if !matches!(state, Some(State::Ready) | None) {
                return state.unwrap();
            }
            changed.changed().await.unwrap();
        }
    })
    .await
}

/// What the far command said as it refused moves the route: a Hide that
/// does not run waits the longest, one too old to carry parks for its
/// update until that machine names another build, and a usage line from a
/// `hide` before `--ssh` reads as too old.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_the_far_command_said_moves_the_route() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let to = over(far.key(), "studio");
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    stand.next(Next::Says(r#"{"ok":false,"reason":"hide_not_running"}"#));
    assert!(within("the refusal", near.pipe.dial(&to)).await.is_err());
    assert!(matches!(
        settled(&near, &to.key).await,
        State::Waiting { .. }
    ));
    assert_eq!(
        lock(&near.carriers.routes).get(&to.key).unwrap().wait,
        limits::CLIMB_MOST
    );

    for said in [
        r#"{"ok":false,"reason":"ssh_update_there"}"#,
        "hide device switch <device> [--install] [--json]",
    ] {
        let stand = Stand::new(&far);
        let near = this_machine(&stand);
        near.carriers.heard_build(&to.key, "1.2.0");
        stand.next(Next::Says(said));
        assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
        near.carriers.heard_build(&to.key, "1.2.0");
        assert!(within("the refusal", near.pipe.dial(&to)).await.is_err());
        assert_eq!(
            settled(&near, &to.key).await,
            State::Parked(PeerSshBlocked::UpdateThere),
            "{said}"
        );
        near.carriers.heard_build(&to.key, "1.2.0");
        assert_eq!(route(&near, &to.key, "studio"), Route::Held);
        near.carriers.heard_build(&to.key, "1.3.0");
        assert_eq!(route(&near, &to.key, "studio"), Route::Ready, "{said}");
    }
}

/// An alias that reaches a Hide proving another key turns the route off in
/// the records, for the alias that attempt dialed; the route goes, and the
/// host names no alias to dial any more (R3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_key_turns_the_route_off_only_for_the_attempts_epoch() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let paired = key().public();
    let to = over(paired, "studio");
    assert!(within("the refusal", near.pipe.dial(&to)).await.is_err());
    assert_eq!(*lock(&near.turned_off), vec![(paired, "studio".to_owned())]);
    assert!(lock(&near.carriers.routes).get(&paired).is_none());

    // An attempt that outlived its route (the alias was recorded again
    // meanwhile) writes nothing.
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    stand.next(Next::Stalls);
    let dialing = {
        let to = to.clone();
        let carriers = Arc::clone(&near.carriers);
        tokio::spawn(async move { carriers.carrier(to.key, "studio".to_owned()).await })
    };
    within("the sign-in", async {
        while stand.running.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let earlier = lock(&near.carriers.routes).get(&paired).unwrap().epoch;
    // A newer alias stops the attempt in flight.
    assert_eq!(route(&near, &paired, "studio-2"), Route::Ready);
    assert!(
        within("the stopped attempt", dialing)
            .await
            .unwrap()
            .is_err()
    );
    near.carriers
        .settled(&paired, "studio", earlier, Outcome::OtherKey);
    assert!(lock(&near.turned_off).is_empty());
    assert_eq!(route(&near, &paired, "studio-2"), Route::Ready);
}

/// A save of the route turned off that fails parks the route: the row says
/// it, nothing is dialed, and the next retry proves the other key again and
/// saves it then.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsaved_off_is_parked_shown_and_saved_after_a_retry() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let paired = key().public();
    let to = over(paired, "studio");
    near.save_fails.store(true, Ordering::Release);
    assert!(within("the refusal", near.pipe.dial(&to)).await.is_err());
    assert_eq!(
        near.carriers.blocked(&paired),
        Some(PeerSshBlocked::NotSaved)
    );
    assert!(near.pipe.dial(&to).await.is_err());
    assert_eq!(stand.started(), 1, "a parked route starts no ssh");
    near.save_fails.store(false, Ordering::Release);
    near.carriers.retry(&paired);
    assert!(within("the refusal", near.pipe.dial(&to)).await.is_err());
    assert_eq!(*lock(&near.turned_off), vec![(paired, "studio".to_owned())]);
}

/// A far end that names the dialed key and signs wrongly proves nothing:
/// the route parks for a retry and is never turned off.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unproved_key_parks_and_never_turns_off() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    near.carriers
        .settled(&to.key, "studio", 0, Outcome::NotProved);
    assert_eq!(
        route(&near, &to.key, "studio"),
        Route::Ready,
        "another epoch"
    );
    let epoch = lock(&near.carriers.routes).get(&to.key).unwrap().epoch;
    near.carriers
        .settled(&to.key, "studio", epoch, Outcome::NotProved);
    assert_eq!(near.carriers.blocked(&to.key), Some(PeerSshBlocked::Retry));
    assert!(lock(&near.turned_off).is_empty());
}

/// Recording an alias, the same one included, starts its route over under a
/// new epoch: a parked route is ready again, an attempt in flight is
/// stopped, and an outcome of an older attempt, another key included,
/// changes nothing, even one reached while the record was being saved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recording_an_alias_again_starts_its_route_over_and_outdates_every_attempt() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    stand.next(Next::Fails(SshProblem::SignInNeeded));
    assert!(within("the failure", near.pipe.dial(&to)).await.is_err());
    assert_eq!(near.carriers.blocked(&to.key), Some(PeerSshBlocked::SignIn));
    let parked = lock(&near.carriers.routes).get(&to.key).unwrap().epoch;
    near.carriers
        .record(&to.key, "studio", || Ok::<(), ()>(()))
        .unwrap();
    assert_eq!(near.carriers.blocked(&to.key), None);

    // A connection the route carries now stays through a record of its
    // alias, and is still the route's to end.
    stand.next(Next::Carry);
    let (dialed, accepted) = within("the connection", async {
        tokio::join!(near.pipe.dial(&to), far.accept())
    })
    .await;
    let (_ours, _theirs) = (dialed.unwrap(), accepted.unwrap());
    let stopped = stand.stops.load(Ordering::Acquire);
    near.carriers
        .record(&to.key, "studio", || Ok::<(), ()>(()))
        .unwrap();
    assert_eq!(
        stand.stops.load(Ordering::Acquire),
        stopped,
        "the live carrier stays"
    );
    near.carriers.hosts(&[]);
    assert_eq!(
        stand.stops.load(Ordering::Acquire),
        stopped + 1,
        "forgetting ends it"
    );
    near.carriers
        .record(&to.key, "studio", || Ok::<(), ()>(()))
        .unwrap();
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
    near.carriers
        .settled(&to.key, "studio", parked, Outcome::OtherKey);
    assert!(lock(&near.turned_off).is_empty());

    // An attempt in flight is stopped by the record.
    stand.next(Next::Stalls);
    let (told, stopped) = mpsc::channel();
    *lock(&stand.stopped) = Some(told);
    let dialing = {
        let carriers = Arc::clone(&near.carriers);
        let key = to.key;
        tokio::spawn(async move { carriers.carrier(key, "studio".to_owned()).await })
    };
    within("the sign-in", async {
        while stand.running.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    near.carriers
        .record(&to.key, "studio", || Ok::<(), ()>(()))
        .unwrap();
    within("the stopped attempt", async {
        while stopped.try_recv().is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(within("its end", dialing).await.unwrap().is_err());

    // Another key, of the attempt current when the save began, waits for
    // the save and then finds its epoch gone.
    let current = lock(&near.carriers.routes).get(&to.key).unwrap().epoch;
    let carriers = Arc::clone(&near.carriers);
    let key = to.key;
    near.carriers
        .record(&to.key, "studio", move || {
            let (done, finished) = mpsc::channel();
            std::thread::spawn(move || {
                carriers.settled(&key, "studio", current, Outcome::OtherKey);
                let _ = done.send(());
            });
            // wait-ok: proves an absence, that the outcome cannot finish
            // while the record is saved.
            let early = finished.recv_timeout(Duration::from_millis(200));
            assert!(early.is_err(), "another key wrote while the record saved");
            Ok::<(), ()>(())
        })
        .unwrap();
    assert!(lock(&near.turned_off).is_empty());
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
}

/// A path that proves and falls again waits longer each time: a proof
/// keeps the wait the route has, and only a connection that stayed starts
/// it over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_that_proves_and_falls_waits_longer_each_time() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    let wait = |near: &Near| lock(&near.carriers.routes).get(&to.key).unwrap().wait;
    for doubled in [2, 4, 8] {
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(near.pipe.dial(&to), far.accept())
        })
        .await;
        let (_ours, _theirs) = (dialed.unwrap(), accepted.unwrap());
        Routes::fell(near.carriers.as_ref(), &to.key);
        assert_eq!(wait(&near), limits::CLIMB_FIRST * doubled);
        // Its wait ran out.
        lock(&near.carriers.routes).get_mut(&to.key).unwrap().state = State::Due;
    }
    Routes::stayed(near.carriers.as_ref(), &to.key);
    assert_eq!(wait(&near), limits::CLIMB_FIRST);
}

/// A carrier the far end closes before the proof, saying no refusal, makes
/// the route wait, so the keeper's redials do not sign in at their pace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_carrier_closed_before_its_proof_makes_the_route_wait() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    stand.next(Next::Says("Connection to studio closed."));
    assert!(within("the end", near.pipe.dial(&to)).await.is_err());
    assert!(matches!(
        settled(&near, &to.key).await,
        State::Waiting { .. }
    ));
    assert_eq!(near.carriers.blocked(&to.key), None);
}

/// An SSH connection that ends at once puts its route into its wait, so a
/// path that keeps falling is signed in to at most once per wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_that_ends_at_once_puts_the_route_into_its_wait() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
    Routes::fell(near.carriers.as_ref(), &to.key);
    assert_eq!(route(&near, &to.key, "studio"), Route::Held);
}

/// A sign-in that never answers holds one of the carriers' own places for
/// its bound, never the operator's, and its stop ends it; one dropped
/// before its bound changes nothing, one its bound ended makes the route
/// wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_sign_in_holds_a_carrier_place_only_for_its_bound_and_never_an_operator_place() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    stand.next(Next::Stalls);
    let (told, stopped) = mpsc::channel();
    *lock(&stand.stopped) = Some(told);
    // Dropped early: the attempt ends and the route is as it was.
    let early = tokio::time::timeout(
        Duration::from_millis(200),
        near.carriers.carrier(to.key, "studio".to_owned()),
    )
    .await;
    assert!(early.is_err());
    stopped
        .recv_timeout(WITHIN)
        .expect("the stalled sign-in was stopped");
    within("the place back", async {
        while near.carriers.places.available_permits() < limits::MAX_CARRIER_SIGN_INS {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
    // Two run at once in the carriers' places; a third waits for one.
    let (told, _stopped) = mpsc::channel();
    *lock(&stand.stopped) = Some(told);
    let first = tokio::spawn(near.carriers.carrier(to.key, "studio".to_owned()));
    let second = tokio::spawn(near.carriers.carrier(to.key, "studio".to_owned()));
    within("two sign-ins", async {
        while stand.running.load(Ordering::Acquire) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(near.carriers.places.available_permits(), 0);
    first.abort();
    second.abort();
}

/// Its own bound reached with nothing said, an attempt makes its route
/// wait, so the climb does not sign in again at the next grace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attempt_its_deadline_ends_puts_the_route_into_its_wait() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    route(&near, &to.key, "studio");
    let epoch = lock(&near.carriers.routes).get(&to.key).unwrap().epoch;
    drop(Attempt {
        carriers: Arc::downgrade(&near.carriers),
        key: to.key,
        alias: "studio".to_owned(),
        epoch,
        started: Instant::now() - limits::CARRIER_WITHIN,
        settled: AtomicBool::new(false),
        said: AtomicBool::new(false),
        unproved: AtomicBool::new(false),
        read: AtomicBool::new(false),
        stop: Arc::default(),
    });
    assert_eq!(route(&near, &to.key, "studio"), Route::Held);
}

/// An attempt dropped before its bound (another path was chosen) leaves
/// its route as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attempt_dropped_early_leaves_the_route_as_it_was() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    route(&near, &to.key, "studio");
    let epoch = lock(&near.carriers.routes).get(&to.key).unwrap().epoch;
    drop(Attempt {
        carriers: Arc::downgrade(&near.carriers),
        key: to.key,
        alias: "studio".to_owned(),
        epoch,
        started: Instant::now(),
        settled: AtomicBool::new(false),
        said: AtomicBool::new(false),
        unproved: AtomicBool::new(false),
        read: AtomicBool::new(false),
        stop: Arc::default(),
    });
    assert_eq!(route(&near, &to.key, "studio"), Route::Ready);
}

/// A host forgotten, or one whose alias changed, takes its route with it,
/// and an attempt still running for it ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forgetting_a_host_stops_its_attempt_in_flight() {
    let far = Arc::new(PipeTransport::accepting(key()));
    let stand = Stand::new(&far);
    let near = this_machine(&stand);
    let to = over(far.key(), "studio");
    stand.next(Next::Stalls);
    let (told, stopped) = mpsc::channel();
    *lock(&stand.stopped) = Some(told);
    let dialing = tokio::spawn(near.carriers.carrier(to.key, "studio".to_owned()));
    within("the sign-in", async {
        while stand.running.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    near.carriers.hosts(&[]);
    stopped
        .recv_timeout(WITHIN)
        .expect("the attempt was stopped");
    assert!(within("its end", dialing).await.unwrap().is_err());
    assert!(lock(&near.carriers.routes).is_empty());
}

/// R7, R1: the carrier's `ssh` forwards nothing of the account's, and the
/// one the daemon makes asks nobody.
#[test]
fn the_carrier_forwards_nothing_and_asks_nothing() {
    let home = tempfile::tempdir().unwrap();
    let reach = Reach {
        home: home.path().to_path_buf(),
        command_folder: None,
        hide: None,
    };
    assert!(carrier_target(&reach, "studio").forwards_nothing());
    assert!(!carrier_upstream(&reach, "studio").asks());
    // The words that run the far command, absolute from the account's home.
    assert_eq!(
        pairing_ssh::carry_argv(&reach, "/home/me", "linux"),
        ["/home/me/.local/bin/hide", "device", "carry", "--ssh"]
    );
    assert_eq!(
        pairing_ssh::carry_argv(&reach, "C:/Users/me", "windows"),
        [
            "C:/Users/me/.local/bin/hide.cmd",
            "device",
            "carry",
            "--ssh"
        ]
    );
    let folder = Reach {
        command_folder: Some("/opt/hide/bin".to_owned()),
        ..reach
    };
    assert_eq!(
        pairing_ssh::carry_argv(&folder, "/home/me", "linux")[0],
        "/opt/hide/bin/hide"
    );
}
