//! The transport for a carrier that is one byte stream (PRD D-27, D-62,
//! D-63, D-79): a WSL distribution and its PC, joined by the pipe `wsl.exe`
//! gives a program it starts in the distribution.
//!
//! Only the PC makes such a carrier ([`PipeTransport::dialing`]); the
//! distribution's core takes the ones that arrive on its local stream
//! ([`PipeTransport::arrived`]). A connection's streams are the carrier's
//! multiplexed ones ([`crate::mux::Pipe`]).
//!
//! Nothing below it authenticates, so each connection's first stream
//! carries the machine-key proof ([`crate::proof`]) before the connection
//! is handed over. An arriving carrier is admitted as any transport's
//! connection is: [`limits::MAX_ADMITTING`] proofs run at once, each for
//! [`limits::HELLO_WITHIN`] at most, and one that arrives past the cap is
//! ended before any of its bytes are read.
//!
//! The same transport carries a connection over the system `ssh`:
//! the daemon's carrier maker signs in to an alias, asking
//! nothing, and runs `hide device carry --ssh` there, whose input and output
//! are the carrier; the far daemon takes it as it takes a distribution's.
//! A connection says the path its carrier came by: [`PathKind::Ssh`] for
//! one an alias carries, [`PathKind::Local`] for a distribution's.
//!
//! [`Beside`] puts this transport beside another, so one [`crate::Peers`]
//! holds a machine's connections whatever carries each: an address that
//! names a distribution ([`Candidate::Wsl`]) is dialed here, an address
//! with only its SSH alias here too, and every other one there. An address
//! that names both an alias and ways the other transport dials is the
//! ladder's ([`Beside::dial`]): the other transport first, SSH from
//! [`limits::SSH_AFTER`], and at [`limits::DIRECT_WITHIN`] the connection
//! of the higher [`PathKind::rank`]. Below the seam it chooses between
//! carriers before anything is admitted; which of two admitted connections
//! stays is still [`crate::Peers`]'.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::Duration;

use hide_peer_protocol::addr::{ssh_alias, without_ssh};
use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Notify, Semaphore, mpsc, watch};
use tokio::time::Instant;

use crate::key::MachineKey;
use crate::mux::{Carrier, PROOF_PRIORITY, Pipe, PipeRecv, PipeSend};
use crate::proof::Proof;
use crate::transport::{DialBound, PeerConn, PeerTransport, RelayState, Route, TransportError};
use crate::{limits, proof};

/// Proved connections that wait to be accepted; one past it is closed
/// busy.
const BACKLOG: usize = 16;

/// The distribution `to` is reached through, when it names one.
pub fn distribution(to: &PeerAddr) -> Option<&str> {
    to.candidates.iter().find_map(|candidate| match candidate {
        Candidate::Wsl(name) => Some(name.as_str()),
        Candidate::Ip(_) | Candidate::Relay(_) | Candidate::Ssh(_) => None,
    })
}

/// The path a carrier to `to` takes: SSH for an address that names an
/// alias, inside one machine for a distribution.
fn path_to(to: &PeerAddr) -> PathKind {
    match ssh_alias(to) {
        Some(_) => PathKind::Ssh,
        None => PathKind::Local,
    }
}

/// What a machine knows of the SSH routes to the hosts it keeps:
/// the daemon's carrier maker owns them, with their waits
/// and what parks one; [`Beside`] and the keeper's climb ask it.
pub trait Routes: Send + Sync + 'static {
    /// Where the route to `key` through `alias` stands now: [`Route::Ready`],
    /// [`Route::Due`] or [`Route::Held`].
    fn route(&self, key: &PeerKey, alias: &str) -> Route;

    /// A connection over the route, or one a move installed, ended within
    /// [`limits::STAYED_AFTER`]: the route waits, so a path that keeps
    /// falling is signed in to at most once per wait. A parked route stays
    /// parked.
    fn fell(&self, key: &PeerKey);

    /// A connection over the route stayed [`limits::STAYED_AFTER`] and
    /// more: its wait starts over. Only this, a retry and a network change
    /// do; a proof alone does not, so a path that proves and falls again
    /// waits longer each time.
    fn stayed(&self, key: &PeerKey);

    /// The machine woke or its network changed: every waiting route is due
    /// and its wait starts over; a parked one stays parked.
    fn network_changed(&self);
}

type Dialing = Pin<Box<dyn Future<Output = io::Result<Carrier>> + Send>>;

/// Makes the carrier to a machine, on a machine that can, for a dial under
/// the bound it is handed.
type Dial = Box<dyn Fn(&PeerAddr, DialBound) -> Dialing + Send + Sync>;

type Accepted = mpsc::Receiver<(PipeSend, PipeRecv)>;

pub struct PipeTransport {
    key: Arc<MachineKey>,
    dial: Option<Dial>,
    /// The SSH routes this machine's carrier maker keeps, on a machine
    /// that makes SSH carriers.
    routes: Option<Arc<dyn Routes>>,
    runtime: tokio::runtime::Handle,
    /// The proofs of arrived carriers that run.
    admitting: Arc<Semaphore>,
    proved: mpsc::Sender<PipeConn>,
    proving: tokio::sync::Mutex<mpsc::Receiver<PipeConn>>,
    /// The connections that have not ended, for [`PeerTransport::close`].
    conns: Arc<Mutex<Vec<Pipe>>>,
    /// Counted down from the largest, so a transport this one stands
    /// beside, which counts up, never gives a connection the same number.
    next_id: Arc<AtomicU64>,
    closed: watch::Sender<bool>,
}

impl PipeTransport {
    /// A transport that only takes the carriers that arrive, as a
    /// distribution's core does. Called inside the runtime its proofs run
    /// on.
    pub fn accepting(key: MachineKey) -> Self {
        let (proved, proving) = mpsc::channel(BACKLOG);
        Self {
            key: Arc::new(key),
            dial: None,
            routes: None,
            runtime: tokio::runtime::Handle::current(),
            admitting: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            proved,
            proving: tokio::sync::Mutex::new(proving),
            conns: Arc::default(),
            next_id: Arc::new(AtomicU64::new(u64::MAX)),
            closed: watch::channel(false).0,
        }
    }

    /// A transport that also makes carriers, as the PC does: `dial` makes
    /// the one to the machine an address names, for a dial under the bound
    /// it is handed, and is dropped when the dial is given up.
    pub fn dialing<F>(
        key: MachineKey,
        dial: impl Fn(&PeerAddr, DialBound) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = io::Result<Carrier>> + Send + 'static,
    {
        Self {
            dial: Some(Box::new(move |to, bound| Box::pin(dial(to, bound)))),
            ..Self::accepting(key)
        }
    }

    /// With the SSH routes the carrier maker keeps.
    pub fn with_routes(mut self, routes: Arc<dyn Routes>) -> Self {
        self.routes = Some(routes);
        self
    }

    /// Where the route to `key` through `alias` stands; held on a machine
    /// that keeps none.
    fn route(&self, key: &PeerKey, alias: &str) -> Route {
        self.routes
            .as_ref()
            .map_or(Route::Held, |routes| routes.route(key, alias))
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// A carrier a peer made arrived, by `path`: [`PathKind::Ssh`] for one
    /// `hide device carry --ssh` hands over, [`PathKind::Local`] for a
    /// distribution's PC. Its proof runs in a task of its own, so one that
    /// never finishes holds up no other; past the cap, or once the
    /// transport closed, the carrier is ended unread.
    pub fn arrived(&self, carrier: Carrier, path: PathKind) {
        if self.is_closed() {
            return carrier.end();
        }
        let Ok(admitted) = Arc::clone(&self.admitting).try_acquire_owned() else {
            return carrier.end();
        };
        let Ok((pipe, mut accepted)) = Pipe::start(carrier, false) else {
            return;
        };
        let key = Arc::clone(&self.key);
        let (proved, conns) = (self.proved.clone(), Arc::clone(&self.conns));
        let id = self.next_id.fetch_sub(1, Ordering::AcqRel);
        let closed = self.closed.subscribe();
        self.runtime.spawn(async move {
            let unproved = Unproved(Some(pipe));
            let proving = async {
                let (mut send, mut recv) = accepted.recv().await?;
                proof::acceptor(&key, &mut send, &mut recv).await.ok()?
            };
            let peer = tokio::time::timeout(limits::HELLO_WITHIN, proving).await;
            drop(admitted);
            let Ok(Some(peer)) = peer else {
                return;
            };
            let conn = PipeConn::new(unproved.proved(), peer, id, false, path, accepted);
            track(&conns, &conn.pipe);
            // Closed while the proof ran, or nobody accepts as fast as
            // carriers arrive.
            if *closed.borrow() {
                conn.close(CloseCode::Shutdown);
            } else if let Err(refused) = proved.try_send(conn) {
                refused.into_inner().close(CloseCode::Busy);
            }
        });
    }
}

fn track(conns: &Mutex<Vec<Pipe>>, pipe: &Pipe) {
    let mut conns = conns.lock().expect("the connection list's lock");
    conns.retain(|known| known.ended().is_none());
    conns.push(pipe.clone());
}

/// A carrier whose other end has not proved its key: given up, it is closed.
struct Unproved(Option<Pipe>);

impl Unproved {
    fn proved(mut self) -> Pipe {
        self.0.take().expect("held until proved")
    }
}

impl Drop for Unproved {
    fn drop(&mut self) {
        if let Some(pipe) = self.0.take() {
            pipe.close(CloseCode::Protocol.number());
        }
    }
}

fn unreachable(why: impl Into<String>) -> TransportError {
    TransportError::Unreachable(why.into())
}

impl PipeTransport {
    /// Runs the key proof against `key` on a carrier the caller holds,
    /// within `within`, and closes the carrier whatever it found: nothing is
    /// admitted and no `told` hears it. What adding an alias to a paired
    /// machine checks before it records the alias: an
    /// operator's press, bounded on its own.
    pub async fn check(
        &self,
        mut carrier: Carrier,
        key: &PeerKey,
        within: Duration,
    ) -> io::Result<Proof> {
        // A check changes no route: whoever made the carrier hears nothing.
        drop(carrier.take_told());
        let (pipe, _accepted) = Pipe::start(carrier, true)?;
        let proving = async {
            let (mut send, mut recv) = pipe.open(PROOF_PRIORITY)?;
            proof::dialer(&self.key, key, &mut send, &mut recv).await
        };
        let found = tokio::time::timeout(within, proving).await;
        pipe.close(CloseCode::Probe.number());
        found.map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "the key proof did not finish in time",
            )
        })?
    }
}

impl PeerTransport for PipeTransport {
    type Conn = PipeConn;

    fn key(&self) -> PeerKey {
        self.key.public()
    }

    /// A machine is never told where a pipe leads: only its own PC makes
    /// one.
    fn candidates(&self) -> Vec<Candidate> {
        Vec::new()
    }

    async fn dial(&self, to: &PeerAddr, bound: DialBound) -> Result<PipeConn, TransportError> {
        if self.is_closed() {
            return Err(TransportError::Closed);
        }
        let dial = self
            .dial
            .as_ref()
            .ok_or_else(|| unreachable("this machine makes no carrier of this kind"))?;
        let mut carrier = dial(to, bound)
            .await
            .map_err(|error| unreachable(format!("no carrier: {error}")))?;
        // Told once, before the dial returns, what the proof found or that
        // the carrier ended first; a dial given up before then tells
        // nothing.
        let mut told = carrier.take_told();
        let mut tell = |found: Option<Proof>| {
            if let Some(told) = told.take() {
                told(found);
            }
        };
        let (pipe, accepted) = match Pipe::start(carrier, true) {
            Ok(started) => started,
            Err(error) => {
                tell(None);
                return Err(unreachable(format!("the carrier did not start: {error}")));
            }
        };
        let unproved = Unproved(Some(pipe.clone()));
        let proving = async {
            let (mut send, mut recv) = pipe.open(PROOF_PRIORITY)?;
            proof::dialer(&self.key, &to.key, &mut send, &mut recv).await
        };
        let found = match proving.await {
            Ok(found) => found,
            Err(error) => {
                tell(None);
                return Err(unreachable(format!(
                    "the key proof did not finish: {error}"
                )));
            }
        };
        tell(Some(found));
        match found {
            Proof::Proved => {}
            Proof::OtherKey(_) => {
                return Err(unreachable(
                    "the machine proved another key than the one dialed",
                ));
            }
            Proof::NotProved => {
                return Err(unreachable(
                    "the machine did not prove the key that was dialed",
                ));
            }
        }
        let id = self.next_id.fetch_sub(1, Ordering::AcqRel);
        let conn = PipeConn::new(unproved.proved(), to.key, id, true, path_to(to), accepted);
        track(&self.conns, &conn.pipe);
        Ok(conn)
    }

    async fn accept(&self) -> Result<PipeConn, TransportError> {
        let mut closed = self.closed.subscribe();
        let mut proving = self.proving.lock().await;
        tokio::select! {
            // This transport holds a sender, so this is a connection.
            conn = proving.recv() => conn.ok_or(TransportError::Closed),
            _ = closed.wait_for(|closed| *closed) => Err(TransportError::Closed),
        }
    }

    async fn relay(&self) -> RelayState {
        RelayState::None
    }

    async fn close(&self) {
        self.closed.send_replace(true);
        let conns = std::mem::take(&mut *self.conns.lock().expect("the connection list's lock"));
        for pipe in conns {
            pipe.close(CloseCode::Shutdown.number());
        }
    }
}

#[derive(Clone)]
pub struct PipeConn {
    pipe: Pipe,
    peer: PeerKey,
    id: u64,
    dialed: bool,
    /// The way its carrier came: `ssh` or `wsl.exe`.
    path: PathKind,
    accepting: Arc<tokio::sync::Mutex<Accepted>>,
}

impl PipeConn {
    fn new(
        pipe: Pipe,
        peer: PeerKey,
        id: u64,
        dialed: bool,
        path: PathKind,
        accepted: Accepted,
    ) -> Self {
        Self {
            pipe,
            peer,
            id,
            dialed,
            path,
            accepting: Arc::new(tokio::sync::Mutex::new(accepted)),
        }
    }

    fn over(&self) -> TransportError {
        TransportError::Ended(self.ended().unwrap_or(CloseCode::Lost))
    }
}

impl PeerConn for PipeConn {
    type Send = PipeSend;
    type Recv = PipeRecv;

    fn peer(&self) -> PeerKey {
        self.peer
    }

    fn id(&self) -> u64 {
        self.id
    }

    fn dialed(&self) -> bool {
        self.dialed
    }

    fn path(&self) -> PathKind {
        self.path
    }

    /// One carrier is the only path a pipe has, so it never moves; the
    /// caller's race with [`PeerConn::closed`] ends the wait.
    async fn path_changed(&self, _from: PathKind) -> PathKind {
        std::future::pending().await
    }

    async fn open(&self, priority: i32) -> Result<(PipeSend, PipeRecv), TransportError> {
        self.pipe
            .open(priority)
            .map_err(|error| match self.ended() {
                Some(code) => TransportError::Ended(code),
                // Every place of this end's is taken; the connection goes on.
                None => unreachable(error.to_string()),
            })
    }

    async fn accept(&self) -> Result<(PipeSend, PipeRecv), TransportError> {
        let mut accepting = self.accepting.lock().await;
        accepting.recv().await.ok_or_else(|| self.over())
    }

    fn close(&self, code: CloseCode) {
        self.pipe.close(code.number());
    }

    fn ended(&self) -> Option<CloseCode> {
        self.pipe.ended().map(CloseCode::from_number)
    }

    async fn closed(&self) -> CloseCode {
        let mut closed = self.pipe.closed();
        match closed.wait_for(Option::is_some).await {
            Ok(code) => code.map_or(CloseCode::Lost, CloseCode::from_number),
            Err(_) => CloseCode::Lost,
        }
    }
}

/// A transport with a pipe transport beside it, as one: an address that
/// names a distribution is dialed over a pipe, an address with an SSH alias
/// through the ladder ([`Beside::dial`]), and every other one over `main`;
/// a connection of either is accepted. The machine's addresses and its
/// relay are `main`'s.
pub struct Beside<T: PeerTransport> {
    main: T,
    pipe: Arc<PipeTransport>,
    ssh: Arc<SshSide<Either<T::Conn, PipeConn>>>,
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // Small maps of handles: whole whatever panicked beside them.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What [`Beside`] holds of its SSH attempts beyond one dial.
struct SshSide<C> {
    /// The attempt a first dial stopped waiting for, per machine: the
    /// climb takes it ([`Route::Handed`]). One per key at most.
    handed: Mutex<HashMap<PeerKey, Arc<SshAttempt>>>,
    /// Connections the ladder or a probe did not choose, closed and not yet
    /// over: a machine is dialed again only after its own are, as
    /// [`crate::Peers`] waits for the connections it closed.
    draining: Mutex<HashMap<PeerKey, Vec<C>>>,
    /// Ends every attempt still running when the transport closes.
    closed: watch::Sender<bool>,
}

impl<C: PeerConn> SshSide<C> {
    /// Closes a connection that was not chosen, before its first exchange,
    /// and keeps it until its end.
    fn unchosen(self: &Arc<Self>, conn: C) {
        conn.close(CloseCode::Probe);
        let key = conn.peer();
        lock(&self.draining)
            .entry(key)
            .or_default()
            .push(conn.clone());
        let side = Arc::clone(self);
        tokio::spawn(async move {
            let _ = tokio::time::timeout(limits::BYE_WITHIN, conn.closed()).await;
            let mut draining = lock(&side.draining);
            if let Some(conns) = draining.get_mut(&key) {
                conns.retain(|kept| kept.id() != conn.id());
                if conns.is_empty() {
                    draining.remove(&key);
                }
            }
        });
    }

    /// Waits, [`limits::BYE_WITHIN`] at most, until the connections to
    /// `key` that were not chosen ended.
    async fn drained(&self, key: &PeerKey) {
        let over = Instant::now() + limits::BYE_WITHIN;
        loop {
            let Some(conn) = lock(&self.draining)
                .get(key)
                .and_then(|conns| conns.first().cloned())
            else {
                return;
            };
            if tokio::time::timeout_at(over, conn.closed()).await.is_err() {
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "ssh.unchosen_not_drained",
                    "peer": key.short(),
                }));
                return;
            }
            let mut draining = lock(&self.draining);
            if let Some(conns) = draining.get_mut(key) {
                conns.retain(|kept| kept.id() != conn.id());
                if conns.is_empty() {
                    draining.remove(key);
                }
            }
        }
    }
}

/// Where an SSH attempt stands.
enum Attempted {
    Running,
    Done(Result<PipeConn, String>),
    /// Its connection went to whoever took it, or was closed.
    Taken,
}

/// One SSH carrier attempt: a task of its own, limited by the bound of the
/// dial that started it, wherever it is (the ladder, handed, the climb), and
/// handed that bound as its own dial's. Dropping it ends the task, which
/// ends its sign-in or its carrier, and closes a connection it proved that
/// nobody took; only the bound running out records that the bound ended it.
struct SshAttempt {
    bound: DialBound,
    state: Mutex<Attempted>,
    changed: Notify,
    task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl SshAttempt {
    /// An attempt for a dial under `bound`.
    fn start(pipe: &Arc<PipeTransport>, to: PeerAddr, bound: DialBound) -> Arc<Self> {
        let attempt = Arc::new(Self {
            bound: bound.clone(),
            state: Mutex::new(Attempted::Running),
            changed: Notify::new(),
            task: Mutex::new(None),
        });
        let (pipe, running) = (Arc::clone(pipe), Arc::downgrade(&attempt));
        let task = tokio::spawn(async move {
            let done = match bound.limit(pipe.dial(&to, bound.clone())).await {
                Some(Ok(conn)) => Ok(conn),
                Some(Err(error)) => Err(error.to_string()),
                None => Err("the SSH attempt did not finish in its time".to_owned()),
            };
            match running.upgrade() {
                Some(attempt) => {
                    *lock(&attempt.state) = Attempted::Done(done);
                    attempt.changed.notify_waiters();
                }
                // Nobody waits: a connection it made is nobody's.
                None => {
                    if let Ok(conn) = done {
                        conn.close(CloseCode::Probe);
                    }
                }
            }
        });
        *lock(&attempt.task) = Some(task.abort_handle());
        attempt
    }

    fn finished(&self) -> bool {
        !matches!(*lock(&self.state), Attempted::Running)
    }

    /// Whether it proved a connection that nobody took yet.
    fn proved(&self) -> bool {
        matches!(*lock(&self.state), Attempted::Done(Ok(_)))
    }

    /// Its connection once it has one, or why it has none.
    async fn take(&self) -> Result<PipeConn, String> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = lock(&self.state);
                match std::mem::replace(&mut *state, Attempted::Taken) {
                    Attempted::Running => *state = Attempted::Running,
                    Attempted::Done(done) => return done,
                    Attempted::Taken => return Err("the SSH attempt was taken".to_owned()),
                }
            }
            changed.await;
        }
    }

    /// Ends the attempt: its task, and a connection it proved that nobody
    /// took, which goes to `side`'s draining list.
    fn end<M: PeerConn>(&self, side: &Arc<SshSide<Either<M, PipeConn>>>) {
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
        let left = std::mem::replace(&mut *lock(&self.state), Attempted::Taken);
        if let Attempted::Done(Ok(conn)) = left {
            side.unchosen(Either::Pipe(conn));
        }
        self.changed.notify_waiters();
    }
}

/// Ends what a ladder holds when it ends without choosing it: the attempt
/// it did not hand, and the connections it did not take.
struct Ladder<'a, M: PeerConn> {
    side: &'a Arc<SshSide<Either<M, PipeConn>>>,
    attempt: Option<Arc<SshAttempt>>,
    main: Option<M>,
    ssh: Option<PipeConn>,
}

impl<'a, M: PeerConn> Ladder<'a, M> {
    fn new(side: &'a Arc<SshSide<Either<M, PipeConn>>>) -> Self {
        Self {
            side,
            attempt: None,
            main: None,
            ssh: None,
        }
    }
}

impl<M: PeerConn> Drop for Ladder<'_, M> {
    fn drop(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            attempt.end(self.side);
        }
        if let Some(conn) = self.main.take() {
            self.side.unchosen(Either::Main(conn));
        }
        if let Some(conn) = self.ssh.take() {
            self.side.unchosen(Either::Pipe(conn));
        }
    }
}

/// Whether `conn` reads a path the transport chose and that beats SSH.
fn direct<C: PeerConn>(conn: &C) -> bool {
    conn.chosen_path()
        .is_some_and(|path| path.rank() > PathKind::Ssh.rank())
}

impl<T: PeerTransport> Beside<T> {
    /// Both carry the same machine key.
    pub fn new(main: T, pipe: PipeTransport) -> Self {
        assert!(main.key() == pipe.key(), "one machine, one key");
        Self {
            main,
            pipe: Arc::new(pipe),
            ssh: Arc::new(SshSide {
                handed: Mutex::default(),
                draining: Mutex::default(),
                closed: watch::channel(false).0,
            }),
        }
    }

    pub fn main(&self) -> &T {
        &self.main
    }

    pub fn pipe(&self) -> &PipeTransport {
        &self.pipe
    }

    /// The attempt kept for `key`, when one is.
    fn handed(&self, key: &PeerKey) -> Option<Arc<SshAttempt>> {
        lock(&self.ssh.handed).get(key).cloned()
    }

    /// Keeps `attempt` for the climb to take: until it fails, until the
    /// earlier of [`limits::TAKE_WITHIN`] after it proved and its own
    /// bound, or until the transport closes.
    fn hand(&self, key: PeerKey, attempt: Arc<SshAttempt>) {
        let earlier = lock(&self.ssh.handed).insert(key, Arc::clone(&attempt));
        if let Some(earlier) = earlier {
            earlier.end(&self.ssh);
        }
        let side = Arc::clone(&self.ssh);
        let mut closed = self.ssh.closed.subscribe();
        tokio::spawn(async move {
            let watched = async {
                loop {
                    let changed = attempt.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    if attempt.finished() {
                        break;
                    }
                    changed.await;
                }
                if attempt.proved() {
                    let over = (Instant::now() + limits::TAKE_WITHIN).min(attempt.bound.over());
                    tokio::time::sleep_until(over).await;
                }
            };
            tokio::select! {
                () = watched => {}
                _ = closed.wait_for(|closed| *closed) => {}
            }
            let mut handed = lock(&side.handed);
            if handed
                .get(&key)
                .is_some_and(|kept| Arc::ptr_eq(kept, &attempt))
            {
                handed.remove(&key);
            }
            drop(handed);
            attempt.end(&side);
        });
    }

    /// The climb's dial (an address with only the alias), under `bound`:
    /// the attempt the first dial handed on, which keeps its own bound, else
    /// a new one.
    async fn ssh_alone(
        &self,
        to: &PeerAddr,
        bound: DialBound,
    ) -> Result<Either<T::Conn, PipeConn>, TransportError> {
        let attempt = lock(&self.ssh.handed)
            .remove(&to.key)
            .unwrap_or_else(|| SshAttempt::start(&self.pipe, to.clone(), bound));
        let mut held = Ladder::<T::Conn>::new(&self.ssh);
        held.attempt = Some(Arc::clone(&attempt));
        let taken = attempt.take().await;
        drop(held);
        taken.map(Either::Pipe).map_err(TransportError::Unreachable)
    }

    /// The first dial of a machine with an SSH alias whose route is ready:
    /// `main` at once, SSH from [`limits::SSH_AFTER`]
    /// unless `main` showed a direct path by then, and the connection of
    /// the higher rank at [`limits::DIRECT_WITHIN`], for a dial under
    /// `bound`. A relay connection taken then hands a sign-in still running
    /// on, for the climb. What is not chosen is closed before its first
    /// exchange.
    async fn ladder(
        &self,
        to: &PeerAddr,
        alias: &str,
        bound: DialBound,
    ) -> Result<Either<T::Conn, PipeConn>, TransportError> {
        let start = Instant::now();
        let main_to = without_ssh(to);
        let ssh_to = PeerAddr::new(to.key, vec![Candidate::Ssh(alias.to_owned())]);
        let main_dial = self.main.dial(&main_to, bound.clone());
        tokio::pin!(main_dial);
        let mut held = Ladder::<T::Conn>::new(&self.ssh);
        let (mut main_failed, mut ssh_failed) = (None::<String>, None::<String>);
        let mut ssh_started = false;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            let window_over = start.elapsed() >= limits::DIRECT_WITHIN;
            if held.main.as_ref().is_some_and(direct) {
                return Ok(Either::Main(held.main.take().expect("checked")));
            }
            if held.ssh.is_some() && (window_over || main_failed.is_some()) {
                return Ok(Either::Pipe(held.ssh.take().expect("checked")));
            }
            if held.main.is_some() && (window_over || ssh_failed.is_some()) {
                // A sign-in still running finishes as the climb's move.
                if let Some(attempt) = held.attempt.take() {
                    self.hand(to.key, attempt);
                }
                return Ok(Either::Main(held.main.take().expect("checked")));
            }
            if let (Some(main), Some(ssh)) = (&main_failed, &ssh_failed) {
                return Err(TransportError::Unreachable(format!(
                    "{main}; over SSH: {ssh}"
                )));
            }
            if !ssh_started && (start.elapsed() >= limits::SSH_AFTER || main_failed.is_some()) {
                ssh_started = true;
                held.attempt = Some(SshAttempt::start(&self.pipe, ssh_to.clone(), bound.clone()));
            }
            let attempt = held.attempt.clone();
            let attempted = async {
                match &attempt {
                    Some(attempt) => attempt.take().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                dialed = &mut main_dial, if held.main.is_none() && main_failed.is_none() => {
                    match dialed {
                        Ok(conn) => held.main = Some(conn),
                        Err(error) => main_failed = Some(error.to_string()),
                    }
                }
                taken = attempted => {
                    held.attempt = None;
                    match taken {
                        Ok(conn) => held.ssh = Some(conn),
                        Err(error) => ssh_failed = Some(error),
                    }
                }
                () = tokio::time::sleep_until(start + limits::SSH_AFTER), if !ssh_started => {}
                () = tokio::time::sleep_until(start + limits::DIRECT_WITHIN), if !window_over => {}
                _ = tick.tick(), if held.main.is_some() => {}
            }
        }
    }
}

impl<T: PeerTransport> PeerTransport for Beside<T> {
    type Conn = Either<T::Conn, PipeConn>;

    fn key(&self) -> PeerKey {
        self.main.key()
    }

    fn candidates(&self) -> Vec<Candidate> {
        self.main.candidates()
    }

    async fn dial(&self, to: &PeerAddr, bound: DialBound) -> Result<Self::Conn, TransportError> {
        self.ssh.drained(&to.key).await;
        if distribution(to).is_some() {
            return self.pipe.dial(to, bound).await.map(Either::Pipe);
        }
        let Some(alias) = ssh_alias(to) else {
            return self.main.dial(to, bound).await.map(Either::Main);
        };
        match self.pipe.route(&to.key, alias) {
            Route::Ready => self.ladder(to, alias, bound).await,
            // Nothing else carries the pair, and no climb runs without a
            // connection: a due route is dialed after `main` failed, or
            // found no path within the window a first dial gives it.
            Route::Due => {
                let main_to = without_ssh(to);
                let main = tokio::time::timeout(
                    limits::DIRECT_WITHIN,
                    self.main.dial(&main_to, bound.clone()),
                );
                let failed = match main.await {
                    Ok(Ok(conn)) => return Ok(Either::Main(conn)),
                    Ok(Err(failed)) => failed.to_string(),
                    Err(_) => "no path opened in its window".to_owned(),
                };
                let ssh_to = PeerAddr::new(to.key, vec![Candidate::Ssh(alias.to_owned())]);
                self.ssh_alone(&ssh_to, bound).await.map_err(|ssh| {
                    TransportError::Unreachable(format!("{failed}; over SSH: {ssh}"))
                })
            }
            Route::Held | Route::Handed | Route::None => self
                .main
                .dial(&without_ssh(to), bound)
                .await
                .map(Either::Main),
        }
    }

    async fn accept(&self) -> Result<Self::Conn, TransportError> {
        // Both waits keep nothing of their own between polls: a connection
        // the one that lost had ready is still there for the next call.
        tokio::select! {
            conn = self.main.accept() => conn.map(Either::Main),
            conn = self.pipe.accept() => conn.map(Either::Pipe),
        }
    }

    async fn relay(&self) -> RelayState {
        self.main.relay().await
    }

    /// The climb's dial: the attempt the first dial handed on, else a new
    /// SSH carrier.
    async fn dial_route(
        &self,
        to: &PeerAddr,
        bound: DialBound,
    ) -> Result<Self::Conn, TransportError> {
        let Some(alias) = ssh_alias(to) else {
            return Err(TransportError::Unreachable(
                "the address names no SSH alias".to_owned(),
            ));
        };
        self.ssh.drained(&to.key).await;
        let ssh_to = PeerAddr::new(to.key, vec![Candidate::Ssh(alias.to_owned())]);
        self.ssh_alone(&ssh_to, bound).await
    }

    fn route(&self, to: &PeerAddr) -> Route {
        let Some(alias) = ssh_alias(to) else {
            return Route::None;
        };
        if distribution(to).is_some() {
            return Route::None;
        }
        if self.handed(&to.key).is_some() {
            return Route::Handed;
        }
        self.pipe.route(&to.key, alias)
    }

    fn route_fell(&self, key: &PeerKey) {
        if let Some(routes) = &self.pipe.routes {
            routes.fell(key);
        }
    }

    fn route_stayed(&self, key: &PeerKey) {
        if let Some(routes) = &self.pipe.routes {
            routes.stayed(key);
        }
    }

    fn network_changed(&self) {
        if let Some(routes) = &self.pipe.routes {
            routes.network_changed();
        }
        self.main.network_changed();
    }

    async fn close(&self) {
        self.ssh.closed.send_replace(true);
        let handed: Vec<_> = lock(&self.ssh.handed).drain().map(|(_, a)| a).collect();
        for attempt in handed {
            attempt.end(&self.ssh);
        }
        self.pipe.close().await;
        self.main.close().await;
    }
}

/// A connection, a stream's half or anything else of one of two transports.
#[derive(Clone)]
pub enum Either<M, P> {
    Main(M),
    Pipe(P),
}

impl<M: PeerConn, P: PeerConn> PeerConn for Either<M, P> {
    type Send = Either<M::Send, P::Send>;
    type Recv = Either<M::Recv, P::Recv>;

    fn peer(&self) -> PeerKey {
        match self {
            Self::Main(conn) => conn.peer(),
            Self::Pipe(conn) => conn.peer(),
        }
    }

    fn id(&self) -> u64 {
        match self {
            Self::Main(conn) => conn.id(),
            Self::Pipe(conn) => conn.id(),
        }
    }

    fn dialed(&self) -> bool {
        match self {
            Self::Main(conn) => conn.dialed(),
            Self::Pipe(conn) => conn.dialed(),
        }
    }

    fn path(&self) -> PathKind {
        match self {
            Self::Main(conn) => conn.path(),
            Self::Pipe(conn) => conn.path(),
        }
    }

    fn chosen_path(&self) -> Option<PathKind> {
        match self {
            Self::Main(conn) => conn.chosen_path(),
            Self::Pipe(conn) => conn.chosen_path(),
        }
    }

    async fn path_changed(&self, from: PathKind) -> PathKind {
        match self {
            Self::Main(conn) => conn.path_changed(from).await,
            Self::Pipe(conn) => conn.path_changed(from).await,
        }
    }

    async fn open(&self, priority: i32) -> Result<(Self::Send, Self::Recv), TransportError> {
        match self {
            Self::Main(conn) => {
                let (send, recv) = conn.open(priority).await?;
                Ok((Either::Main(send), Either::Main(recv)))
            }
            Self::Pipe(conn) => {
                let (send, recv) = conn.open(priority).await?;
                Ok((Either::Pipe(send), Either::Pipe(recv)))
            }
        }
    }

    async fn accept(&self) -> Result<(Self::Send, Self::Recv), TransportError> {
        match self {
            Self::Main(conn) => {
                let (send, recv) = conn.accept().await?;
                Ok((Either::Main(send), Either::Main(recv)))
            }
            Self::Pipe(conn) => {
                let (send, recv) = conn.accept().await?;
                Ok((Either::Pipe(send), Either::Pipe(recv)))
            }
        }
    }

    fn close(&self, code: CloseCode) {
        match self {
            Self::Main(conn) => conn.close(code),
            Self::Pipe(conn) => conn.close(code),
        }
    }

    fn ended(&self) -> Option<CloseCode> {
        match self {
            Self::Main(conn) => conn.ended(),
            Self::Pipe(conn) => conn.ended(),
        }
    }

    async fn closed(&self) -> CloseCode {
        match self {
            Self::Main(conn) => conn.closed().await,
            Self::Pipe(conn) => conn.closed().await,
        }
    }
}

impl<M: AsyncRead + Unpin, P: AsyncRead + Unpin> AsyncRead for Either<M, P> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_read(context, buffer),
            Self::Pipe(half) => Pin::new(half).poll_read(context, buffer),
        }
    }
}

impl<M: AsyncWrite + Unpin, P: AsyncWrite + Unpin> AsyncWrite for Either<M, P> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_write(context, buffer),
            Self::Pipe(half) => Pin::new(half).poll_write(context, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_flush(context),
            Self::Pipe(half) => Pin::new(half).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_shutdown(context),
            Self::Pipe(half) => Pin::new(half).poll_shutdown(context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::Net;
    use hide_platform::ipc::LocalStream;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A hang guard, never a pause.
    const WITHIN: Duration = Duration::from_secs(20);

    async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
        tokio::time::timeout(WITHIN, wait)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    fn key() -> MachineKey {
        MachineKey::generate().unwrap()
    }

    fn copy(key: &MachineKey) -> MachineKey {
        MachineKey::from_secret(&key.secret())
    }

    /// A PC whose every carrier arrives at `distribution`, as `wsl.exe`'s
    /// arrives at the distribution's core.
    fn pc(key: MachineKey, distribution: &Arc<PipeTransport>) -> PipeTransport {
        let distribution = Arc::clone(distribution);
        PipeTransport::dialing(key, move |_, _| {
            let (near, far) = LocalStream::pair().unwrap();
            distribution.arrived(far.into(), PathKind::Local);
            async move { Ok(Carrier::from(near)) }
        })
    }

    fn in_wsl(key: PeerKey) -> PeerAddr {
        PeerAddr::new(key, vec![Candidate::Wsl("Ubuntu".to_owned())])
    }

    /// The PC dials, both ends know the other's proved key, and a stream
    /// either end opens crosses; a close is read with its code on the other
    /// end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pc_and_its_distribution_connect_and_open_streams_both_ways() {
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to, DialBound::start()), distro.accept())
        })
        .await;
        let (ours, theirs) = (dialed.unwrap(), accepted.unwrap());
        assert_eq!((ours.peer(), theirs.peer()), (distro.key(), pc.key()));
        assert!(ours.dialed() && !theirs.dialed());
        assert_eq!(ours.path(), PathKind::Local);

        for (opener, taker) in [(&ours, &theirs), (&theirs, &ours)] {
            let (mut send, _recv) = opener.open(0).await.unwrap();
            send.write_all(b"over the pipe").await.unwrap();
            send.shutdown().await.unwrap();
            let (_send, mut recv) = within("the stream", taker.accept()).await.unwrap();
            let mut read = Vec::new();
            within("its bytes", recv.read_to_end(&mut read))
                .await
                .unwrap();
            assert_eq!(read, b"over the pipe");
        }

        ours.close(CloseCode::Replaced);
        assert_eq!(
            within("the close", theirs.closed()).await,
            CloseCode::Replaced
        );
        assert_eq!(ours.ended(), Some(CloseCode::Replaced));
        assert!(matches!(
            theirs.open(0).await,
            Err(TransportError::Ended(CloseCode::Replaced))
        ));
    }

    /// A distribution that holds another key than the one dialed is not
    /// connected to, and a machine that makes no carrier dials nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_machine_that_does_not_prove_the_dialed_key_is_not_connected_to() {
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let other = key().public();
        let refused = within("the refusal", pc.dial(&in_wsl(other), DialBound::start())).await;
        assert!(matches!(refused, Err(TransportError::Unreachable(_))));
        let no_carrier = distro.dial(&in_wsl(pc.key()), DialBound::start()).await;
        assert!(matches!(no_carrier, Err(TransportError::Unreachable(_))));
    }

    /// As many arrived carriers prove at once as may and no more: one past
    /// the cap is ended unread, a silent one is given up after
    /// `HELLO_WITHIN`, and an honest dialer connects once a place is free.
    #[tokio::test]
    async fn arrived_carriers_are_admitted_under_a_count_and_a_time() {
        use std::io::Read;
        let distro = Arc::new(PipeTransport::accepting(key()));
        let mut silent = Vec::new();
        for _ in 0..limits::MAX_ADMITTING {
            let (near, far) = LocalStream::pair().unwrap();
            distro.arrived(far.into(), PathKind::Local);
            silent.push(near);
        }
        assert_eq!(distro.admitting.available_permits(), 0);
        // Past the cap: ended before any of its bytes are read.
        let (mut near, far) = LocalStream::pair().unwrap();
        distro.arrived(far.into(), PathKind::Local);
        near.set_read_timeout(Some(WITHIN)).unwrap();
        assert_eq!(near.read(&mut [0_u8; 1]).unwrap(), 0);

        // The time is the table's; the test only shortens the wait for it.
        // Each proof's task runs once first, so its wait started before the
        // clock is moved.
        tokio::task::yield_now().await;
        tokio::time::pause();
        tokio::time::advance(limits::HELLO_WITHIN + Duration::from_secs(1)).await;
        tokio::time::resume();
        within("the silent carriers to be given up", async {
            while distro.admitting.available_permits() < limits::MAX_ADMITTING {
                tokio::task::yield_now().await;
            }
        })
        .await;
        for mut near in silent {
            near.set_read_timeout(Some(WITHIN)).unwrap();
            let mut rest = Vec::new();
            // The greeting, then the end.
            near.read_to_end(&mut rest).unwrap();
        }
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to, DialBound::start()), distro.accept())
        })
        .await;
        dialed.unwrap();
        accepted.unwrap();
    }

    /// Closing the transport ends its connections with the shutdown code,
    /// ends a waiting accept, and takes no carrier afterwards.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_the_transport_ends_its_connections_and_takes_no_more() {
        use std::io::Read;
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to, DialBound::start()), distro.accept())
        })
        .await;
        let (ours, _theirs) = (dialed.unwrap(), accepted.unwrap());
        let waiting = {
            let distro = Arc::clone(&distro);
            tokio::spawn(async move { distro.accept().await })
        };
        distro.close().await;
        assert_eq!(
            within("the close", ours.closed()).await,
            CloseCode::Shutdown
        );
        assert!(matches!(
            within("the accept", waiting).await.unwrap(),
            Err(TransportError::Closed)
        ));
        let (mut near, far) = LocalStream::pair().unwrap();
        distro.arrived(far.into(), PathKind::Local);
        near.set_read_timeout(Some(WITHIN)).unwrap();
        assert_eq!(near.read(&mut [0_u8; 1]).unwrap(), 0);
    }

    /// A connection says the way its carrier came: one `hide device carry
    /// --ssh` handed over reads SSH on both ends, a distribution's reads
    /// inside one machine.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_arrived_ssh_carrier_reads_ssh_and_a_wsl_one_local() {
        let far = Arc::new(PipeTransport::accepting(key()));
        for (path, to) in [
            (
                PathKind::Ssh,
                PeerAddr::new(far.key(), vec![Candidate::Ssh("studio".to_owned())]),
            ),
            (PathKind::Local, in_wsl(far.key())),
        ] {
            let arrives = Arc::clone(&far);
            let near = PipeTransport::dialing(key(), move |_, _| {
                let (near, theirs) = LocalStream::pair().unwrap();
                arrives.arrived(theirs.into(), path);
                async move { Ok(Carrier::from(near)) }
            });
            let (dialed, accepted) = within("the connection", async {
                tokio::join!(near.dial(&to, DialBound::start()), far.accept())
            })
            .await;
            assert_eq!(
                (dialed.unwrap().path(), accepted.unwrap().path()),
                (path, path)
            );
        }
    }

    /// A handed attempt that proved just before its own bound is kept only
    /// until that bound, not `TAKE_WITHIN` past its proof: it leaves the
    /// handed list and its connection ends on both machines.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_handed_attempt_proved_near_its_bound_ends_at_that_bound() {
        let far = Arc::new(PipeTransport::accepting(key()));
        let arrives = Arc::clone(&far);
        let ours = key();
        let twin = MachineKey::from_secret(&ours.secret());
        let pipe = PipeTransport::dialing(twin, move |_, _| {
            let (near, theirs) = LocalStream::pair().unwrap();
            arrives.arrived(theirs.into(), PathKind::Ssh);
            async move { Ok(Carrier::from(near)) }
        });
        let beside = Beside::new(Net::new().join(ours), pipe);
        let to = PeerAddr::new(far.key(), vec![Candidate::Ssh("studio".to_owned())]);
        let left = Duration::from_millis(800);
        let attempt = SshAttempt::start(
            &beside.pipe,
            to.clone(),
            DialBound::at(Instant::now() + left),
        );
        let theirs = within("the far end", far.accept()).await.unwrap();
        within("the proof", async {
            while !attempt.proved() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        let proved = Instant::now();
        beside.hand(to.key, attempt);
        within("its end", async {
            while beside.handed(&to.key).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            proved.elapsed() < limits::TAKE_WITHIN,
            "kept past its bound: {:?}",
            proved.elapsed()
        );
        within("the far connection's end", theirs.closed()).await;
    }

    /// A dial tells its carrier what the proof found, once, before it
    /// returns; a check tells nothing, admits nothing and closes its carrier.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dial_tells_its_carrier_the_proof_once_and_a_check_tells_nothing() {
        let far = Arc::new(PipeTransport::accepting(key()));
        let told = Arc::new(Mutex::new(Vec::new()));
        let (arrives, heard) = (Arc::clone(&far), Arc::clone(&told));
        let near = PipeTransport::dialing(key(), move |_, _| {
            let (near, theirs) = LocalStream::pair().unwrap();
            arrives.arrived(theirs.into(), PathKind::Ssh);
            let heard = Arc::clone(&heard);
            async move { Ok(Carrier::from(near).told(move |proof| heard.lock().unwrap().push(proof))) }
        });
        let to = PeerAddr::new(far.key(), vec![Candidate::Ssh("studio".to_owned())]);
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(near.dial(&to, DialBound::start()), far.accept())
        })
        .await;
        dialed.unwrap();
        accepted.unwrap();
        let other = PeerAddr::new(key().public(), vec![Candidate::Ssh("other".to_owned())]);
        assert!(
            within("the refusal", near.dial(&other, DialBound::start()))
                .await
                .is_err()
        );
        assert_eq!(
            *told.lock().unwrap(),
            vec![Some(Proof::Proved), Some(Proof::OtherKey(far.key()))]
        );

        // A check: the proof's outcome, no told, nothing to accept.
        let (mine, theirs) = LocalStream::pair().unwrap();
        far.arrived(theirs.into(), PathKind::Ssh);
        let heard = Arc::clone(&told);
        let carrier = Carrier::from(mine).told(move |proof| heard.lock().unwrap().push(proof));
        let checked = within("the check", near.check(carrier, &far.key(), WITHIN))
            .await
            .unwrap();
        assert_eq!(checked, Proof::Proved);
        assert_eq!(told.lock().unwrap().len(), 2, "a check tells nothing");
        // wait-ok: proves an absence, that nothing reaches the far queue.
        let accepted = tokio::time::timeout(Duration::from_millis(300), far.accept()).await;
        // A carrier proved and closed at once may still reach the far
        // queue; it is over there.
        if let Ok(Ok(conn)) = accepted {
            within("its end", conn.closed()).await;
        }
    }

    /// Beside another transport, an address that names a distribution goes
    /// over the pipe and every other one over the main transport, and one
    /// accept takes a connection of either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn beside_another_transport_an_address_picks_its_carrier() {
        let net = Net::new();
        let (pc_key, distro_key, far_key) = (key(), key(), key());
        let distro_pipe = Arc::new(PipeTransport::accepting(copy(&distro_key)));
        // The distribution is on no wire the PC's main transport reaches.
        let pc = Beside::new(net.join(copy(&pc_key)), pc(copy(&pc_key), &distro_pipe));
        let far = net.join(copy(&far_key));

        let to_distro = in_wsl(distro_key.public());
        let over_pipe = within("the pipe's connection", async {
            tokio::join!(
                pc.dial(&to_distro, DialBound::start()),
                distro_pipe.accept()
            )
        })
        .await;
        assert!(matches!(over_pipe.0, Ok(Either::Pipe(_))));
        over_pipe.1.unwrap();
        let plain = PeerAddr::new(far_key.public(), Vec::new());
        let over_main = within("the main connection", async {
            tokio::join!(pc.dial(&plain, DialBound::start()), far.accept())
        })
        .await;
        assert!(matches!(over_main.0, Ok(Either::Main(_))));
        // Without the distribution's name the address is the main
        // transport's, which does not reach it.
        let unnamed = PeerAddr::new(distro_key.public(), Vec::new());
        assert!(pc.dial(&unnamed, DialBound::start()).await.is_err());

        // A machine that dials the PC arrives through the same accept.
        let to_pc = PeerAddr::new(pc_key.public(), Vec::new());
        let (dialed, accepted) = within("the dial to the PC", async {
            tokio::join!(far.dial(&to_pc, DialBound::start()), pc.accept())
        })
        .await;
        dialed.unwrap();
        assert!(matches!(accepted, Ok(Either::Main(_))));
    }
}
