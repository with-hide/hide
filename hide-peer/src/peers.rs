//! The connections of one machine (PRD D-62): the first exchange,
//! admission, which of two connections stays, and who may open what.
//!
//! One connection per pair of machines carries both directions. Each
//! direction has its own grant and its own generation ([`crate::gate`]): the
//! grant this machine gives a peer gates the streams that peer opens here,
//! and the grant a peer gives this machine gates the streams opened there.
//!
//! The rule for dialing (iroh#4639, #4600):
//! - no new dial while a live connection to that machine exists;
//! - an old connection is closed to its end before the machine is dialed
//!   again;
//! - a dial in flight is given up when the peer's own connection is
//!   admitted first;
//! - when both sides' dials did establish, the one the smaller key dialed
//!   stays ([`decide`]);
//! - a connection from the side that dialed the one held replaces it: the
//!   peer restarted or lost its end, and closing the new one as a duplicate
//!   would leave the pair with a dead connection;
//! - a connection that names the held one as the one it replaces (a move
//!   to a better path) replaces it whichever side dialed it,
//!   and one a move replaced is refused wherever it arrives late
//!   ([`decide`]).
//!
//! A machine that keeps a host climbs while the connection lives
//!: from the relay to SSH, and from SSH to a direct path
//! the other transport finds, each move a new connection that names the one
//! it replaces.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

use hide_kit::layout;
use hide_peer_protocol::addr::{ssh_alias, without_ssh};
use hide_peer_protocol::hello::{self, Agreed, Behind, GrantState, Hello, HelloFrame};
use hide_peer_protocol::pairing::ConfirmState;
use hide_peer_protocol::stream::{self, Status};
use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey, StreamKind};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};

use crate::gate::{CLOSED, Gate, Guarded};
use crate::limits;
use crate::store::{Change, Grant, GrantStatus, Grants, Host, Known, KnownState, StoreError, Via};
use crate::transport::Route;
use crate::transport::{PeerConn, PeerTransport, TransportError};
use crate::wire;

/// A peer as the rest of Hide names it: its connection id and its product
/// device id, paired. It is made only from the two records of a pairing, so
/// the two ids never travel as loose strings (D-29).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PeerIdentity {
    key: PeerKey,
    device: String,
}

impl PeerIdentity {
    pub fn key(&self) -> &PeerKey {
        &self.key
    }

    pub fn device(&self) -> &str {
        &self.device
    }
}

/// What happened to a connection, for whoever shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerEvent {
    Connected {
        key: PeerKey,
        path: PathKind,
    },
    Disconnected {
        key: PeerKey,
        code: CloseCode,
    },
    /// The connection's bytes take another path now: a direct one opened
    /// beside the relay, or the relay took over (D-86).
    PathChanged {
        key: PeerKey,
        path: PathKind,
    },
    /// The two builds cannot connect until one updates (D-40).
    UpdateNeeded {
        key: PeerKey,
        device: String,
        build: String,
        behind: Behind,
    },
    /// The grant the peer gives this machine is now `state`.
    Granted {
        key: PeerKey,
        state: GrantState,
    },
    /// The peer, which holds an active grant here, asks this machine to
    /// update itself.
    UpdateRequested {
        key: PeerKey,
    },
    UpdateAnswered {
        key: PeerKey,
        started: bool,
        reason: Option<String>,
    },
}

/// A stream of a known kind whose two halves end with the grant they run
/// under.
pub struct PeerStream<C: PeerConn> {
    pub kind: StreamKind,
    pub send: Guarded<C::Send>,
    pub recv: Guarded<C::Recv>,
}

/// A stream a peer opened and this machine accepted.
pub struct Incoming<C: PeerConn> {
    pub link: Link<C>,
    pub stream: PeerStream<C>,
}

#[derive(Debug)]
pub enum OpenError {
    /// This machine holds no active grant on the peer; the state says why.
    NoGrant(GrantState),
    /// The peer refused the stream.
    Refused(Status),
    Ended(TransportError),
    Io(io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGrant(state) => write!(f, "no active grant on the peer ({state:?})"),
            Self::Refused(status) => write!(f, "the peer refused the stream ({status:?})"),
            Self::Ended(error) => error.fmt(f),
            Self::Io(error) => write!(f, "the stream failed: {error}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub enum DialError {
    Transport(TransportError),
    TimedOut,
    UpdateNeeded(Behind),
    /// The peer's first exchange was not the contract's.
    Protocol(String),
    /// The connection was established and not kept.
    Refused(CloseCode),
}

impl fmt::Display for DialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::TimedOut => f.write_str("the peer did not answer in time"),
            Self::UpdateNeeded(behind) => write!(f, "an update is needed first ({behind:?})"),
            Self::Protocol(why) => write!(f, "the first exchange failed: {why}"),
            Self::Refused(code) => write!(f, "the connection was not kept ({})", code.name()),
        }
    }
}

impl std::error::Error for DialError {}

impl From<TransportError> for DialError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

impl From<io::Error> for DialError {
    fn from(error: io::Error) -> Self {
        Self::Protocol(error.to_string())
    }
}

/// What time it is, as the records stamp it: Unix milliseconds. A daemon
/// reads the system's clock; a test moves its own, so ten minutes of a
/// pending grant take no time.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> u64 + Send + Sync>);

impl Clock {
    pub fn system() -> Self {
        Self::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis() as u64)
        })
    }

    pub fn new(now_unix_ms: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self(Arc::new(now_unix_ms))
    }

    pub fn now(&self) -> u64 {
        (self.0)()
    }
}

/// Why a connection to a machine may stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    /// A grant is settled in either direction.
    Settled,
    /// Only a grant that still waits this long for its confirm.
    Pending { left: Duration },
    /// Nothing: a machine in the middle of pairing, or a stranger.
    None,
}

fn reason_in(grants: &Grants, known: &Known, key: &PeerKey, now_unix_ms: u64) -> Reason {
    if grants.current(key).is_some() || known.of(key).is_some() {
        return Reason::Settled;
    }
    match grants.pending_left(key, now_unix_ms) {
        Some(left) => Reason::Pending { left },
        None => Reason::None,
    }
}

/// What to do with a newly established connection to a machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// There is no other: keep it.
    Install,
    /// Keep it and close the one held with this code.
    Replace(CloseCode),
    /// Close it with this code and keep the one held.
    Refuse(CloseCode),
}

/// One connection as [`decide`] reads it: who dialed it, and the names its
/// dialer's hello gave it (`Hello::dial`, `Hello::replaces`), which both
/// ends read alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Named {
    pub dialed_by_us: bool,
    pub dial: u64,
    pub replaces: u64,
}

/// Decides between the connection held to `peer` and a new one. Both sides
/// decide the same from the same facts, so exactly one connection is left:
/// a connection a successor replaced (`superseded`, what this end installed
/// says it replaces) is refused wherever it arrives, a successor of the held
/// one replaces it (the new one names it, as the one it replaces or among
/// those its dialer knew were, `supersedes`), and otherwise the same side's
/// newer dial replaces its older one and of two sides' dials the smaller
/// key's stays.
pub fn decide(
    ours: &PeerKey,
    peer: &PeerKey,
    held: Option<Named>,
    new: Named,
    supersedes: &[u64],
    superseded: &[u64],
) -> Standing {
    if new.dial != 0 && superseded.contains(&new.dial) {
        return Standing::Refuse(CloseCode::Replaced);
    }
    let Some(held) = held else {
        return Standing::Install;
    };
    if held.dial != 0 && (new.replaces == held.dial || supersedes.contains(&held.dial)) {
        return Standing::Replace(CloseCode::Replaced);
    }
    if held.dialed_by_us == new.dialed_by_us {
        // The same side dialed again: its earlier connection is gone on its
        // end, whatever this end still believes.
        return Standing::Replace(CloseCode::Replaced);
    }
    let dialer = |by_us: bool| if by_us { ours } else { peer };
    if dialer(new.dialed_by_us) < dialer(held.dialed_by_us) {
        Standing::Replace(CloseCode::Duplicate)
    } else {
        Standing::Refuse(CloseCode::Duplicate)
    }
}

/// Whether this machine holds its side of host `key` back.
fn held_back(known: &Known, key: &PeerKey) -> bool {
    known.of(key).is_some_and(|host| host.state.held_back())
}

fn state_tag(state: GrantState) -> u8 {
    match state {
        GrantState::None => 0,
        GrantState::Pending => 1,
        GrantState::Active => 2,
        GrantState::Paused => 3,
        GrantState::Revoked => 4,
    }
}

fn state_of_tag(tag: u8) -> GrantState {
    match tag {
        1 => GrantState::Pending,
        2 => GrantState::Active,
        3 => GrantState::Paused,
        4 => GrantState::Revoked,
        _ => GrantState::None,
    }
}

/// How many streams of each kind a connection has accepted and not ended.
#[derive(Default)]
struct Counts([AtomicUsize; StreamKind::ALL.len()]);

/// One accepted stream's place among its kind; dropping it frees the place.
struct Place {
    counts: Arc<Counts>,
    kind: StreamKind,
}

/// A kind's place in [`Counts`]: its place in [`StreamKind::ALL`], since
/// tags need not follow each other.
fn slot(kind: StreamKind) -> usize {
    StreamKind::ALL
        .iter()
        .position(|listed| *listed == kind)
        .expect("every kind is in ALL")
}

impl Counts {
    fn take(self: &Arc<Self>, kind: StreamKind) -> Option<Place> {
        let count = &self.0[slot(kind)];
        if count.fetch_add(1, Ordering::AcqRel) >= limits::max_streams_of(kind) {
            count.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Place {
            counts: Arc::clone(self),
            kind,
        })
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        self.counts.0[slot(self.kind)].fetch_sub(1, Ordering::AcqRel);
    }
}

struct LinkInner<C: PeerConn> {
    conn: C,
    theirs: Hello,
    agreed: Agreed,
    /// The connection's name and the one it replaces, read from its
    /// dialer's hello: the same on both ends for its whole life.
    named: Named,
    /// The grant this machine gives the peer: gates what the peer opens.
    inbound: Arc<Gate>,
    /// The grant the peer gives this machine: gates what is opened there.
    /// It is shut while this machine holds its own side back
    /// ([`KnownState::held_back`]), whatever the peer grants.
    outbound: Arc<Gate>,
    /// The generation the peer last said its grant runs under, shut when
    /// the grant is not active: what `outbound` opens to again when this
    /// machine stops holding back.
    heard: AtomicU64,
    granted: AtomicU8,
    counts: Arc<Counts>,
    /// Streams whose header is still being read.
    headers: Arc<Semaphore>,
    frames: mpsc::Sender<HelloFrame>,
}

/// An admitted connection to one machine.
pub struct Link<C: PeerConn> {
    inner: Arc<LinkInner<C>>,
}

impl<C: PeerConn> Clone for Link<C> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<C: PeerConn> Link<C> {
    pub fn key(&self) -> PeerKey {
        self.inner.conn.peer()
    }

    /// The connection's number: the same link while it is the same.
    pub fn id(&self) -> u64 {
        self.inner.conn.id()
    }

    pub fn dialed(&self) -> bool {
        self.inner.conn.dialed()
    }

    pub fn path(&self) -> PathKind {
        self.inner.conn.path()
    }

    /// The dialer's name for this connection (`Hello::dial`); `0` from a
    /// build that names none.
    pub fn dial_name(&self) -> u64 {
        self.inner.named.dial
    }

    /// Whether the two builds agreed that a connection may run over SSH
    /// and move between paths.
    pub fn can_move(&self) -> bool {
        self.inner.agreed.has(hello::SSH_PATH)
    }

    /// What the two builds agreed to speak. A caller acts on a capability
    /// only when [`Agreed::has`] it.
    pub fn agreed(&self) -> &Agreed {
        &self.inner.agreed
    }

    /// The peer's first exchange: its build and the device id it claims.
    pub fn their_hello(&self) -> &Hello {
        &self.inner.theirs
    }

    /// The state of the grant the peer gives this machine.
    pub fn granted(&self) -> GrantState {
        state_of_tag(self.inner.granted.load(Ordering::Acquire))
    }

    /// Opens a stream of `kind` to the peer. A kind that needs a grant
    /// opens only under the active one the peer gave, and ends with it.
    pub async fn open(&self, kind: StreamKind) -> Result<PeerStream<C>, OpenError> {
        let pass = if kind.needs_grant() {
            let pass = self.inner.outbound.pass();
            if !pass.holds() {
                return Err(OpenError::NoGrant(self.granted()));
            }
            Some(pass)
        } else {
            None
        };
        let generation = pass.as_ref().map_or(CLOSED, |pass| pass.generation());
        let (mut send, mut recv) = self
            .inner
            .conn
            .open(kind.priority())
            .await
            .map_err(OpenError::Ended)?;
        send.write_all(&stream::encode_header(kind, generation))
            .await?;
        send.flush().await?;
        let mut answer = [0u8; stream::ANSWER_BYTES];
        tokio::time::timeout(limits::HELLO_WITHIN, recv.read_exact(&mut answer))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no answer to a stream"))??;
        match stream::decode_answer(answer) {
            Some((Status::Accepted, _)) => Ok(PeerStream {
                kind,
                send: Guarded::new(send, pass.as_ref().map(|pass| pass.twin()), None),
                recv: Guarded::new(recv, pass, None),
            }),
            Some((status, _)) => Err(OpenError::Refused(status)),
            None => Err(OpenError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "the answer to a stream is not the contract's",
            ))),
        }
    }

    /// Asks the peer to update itself. Only a side that holds an active
    /// grant on the peer may ask (D-40).
    pub fn request_update(&self) -> Result<(), OpenError> {
        if !self.inner.outbound.pass().holds() {
            return Err(OpenError::NoGrant(self.granted()));
        }
        self.tell(HelloFrame::Update);
        Ok(())
    }

    pub fn answer_update(&self, started: bool, reason: Option<String>) {
        self.tell(HelloFrame::UpdateAnswer { started, reason });
    }

    /// Queues a frame for the hello stream. A peer that reads none of them
    /// is not connected in any useful sense: the connection is closed.
    fn tell(&self, frame: HelloFrame) {
        if self.inner.frames.try_send(frame).is_err() {
            self.close(CloseCode::Lost);
        }
    }

    pub fn close(&self, code: CloseCode) {
        self.inner.conn.close(code);
    }

    /// Resolves when the connection is over, with the code it ended with.
    pub async fn closed(&self) -> CloseCode {
        self.inner.conn.closed().await
    }

    /// Decides a stream the peer opened: its status, and when accepted its
    /// kind, its hold on the grant and its place among its kind.
    fn judge(
        &self,
        tag: u8,
        generation: u64,
    ) -> (
        Status,
        Option<(StreamKind, Option<crate::gate::Pass>, Place)>,
    ) {
        let Some(kind) = StreamKind::from_tag(tag) else {
            return (Status::UnknownKind, None);
        };
        let pass = if kind.needs_grant() {
            let pass = self.inner.inbound.pass();
            if pass.generation() == CLOSED {
                return (Status::NoGrant, None);
            }
            if pass.generation() != generation {
                return (Status::Stale, None);
            }
            Some(pass)
        } else {
            None
        };
        match self.inner.counts.take(kind) {
            Some(place) => (Status::Accepted, Some((kind, pass, place))),
            None => (Status::Limit, None),
        }
    }
}

struct Table<C: PeerConn> {
    live: HashMap<PeerKey, Link<C>>,
    /// Connections closed here whose end has not been seen yet. A machine
    /// is dialed again only when it has none.
    draining: HashMap<PeerKey, Vec<C>>,
    /// The names of connections that were ended for another, per machine:
    /// ones this end refused or replaced, and ones the connections it
    /// installed say they replace. Each kept [`limits::SUPERSEDED_FOR`], at
    /// most [`limits::MAX_SUPERSEDED`]: one that arrives late is refused
    /// ([`decide`]), and each hello this end dials carries them.
    superseded: HashMap<PeerKey, Vec<(u64, Instant)>>,
}

impl<C: PeerConn> Table<C> {
    /// The names of `key`'s connections a move replaced that are still
    /// remembered.
    fn superseded(&mut self, key: &PeerKey) -> Vec<u64> {
        let now = Instant::now();
        let Some(kept) = self.superseded.get_mut(key) else {
            return Vec::new();
        };
        kept.retain(|(_, at)| now.duration_since(*at) < limits::SUPERSEDED_FOR);
        let names = kept.iter().map(|(name, _)| *name).collect();
        if kept.is_empty() {
            self.superseded.remove(key);
        }
        names
    }

    /// Remembers `name` as ended for another. A name already known keeps
    /// the time it was first heard, so two ends that name it to each other
    /// do not keep it alive; every machine's names are aged here, and at
    /// most [`limits::MAX_SUPERSEDED_PEERS`] machines keep any, the one
    /// whose newest name is oldest going first.
    fn supersede(&mut self, key: PeerKey, name: u64) {
        let now = Instant::now();
        self.superseded.retain(|_, kept| {
            kept.retain(|(_, at)| now.duration_since(*at) < limits::SUPERSEDED_FOR);
            !kept.is_empty()
        });
        let kept = self.superseded.entry(key).or_default();
        if kept.iter().any(|(known, _)| *known == name) {
            return;
        }
        kept.push((name, now));
        if kept.len() > limits::MAX_SUPERSEDED {
            kept.remove(0);
        }
        while self.superseded.len() > limits::MAX_SUPERSEDED_PEERS {
            let oldest = self
                .superseded
                .iter()
                .filter(|(peer, _)| **peer != key)
                .min_by_key(|(_, kept)| kept.last().map(|(_, at)| *at))
                .map(|(peer, _)| *peer);
            match oldest {
                Some(peer) => self.superseded.remove(&peer),
                None => break,
            };
        }
    }
}

struct Inner<T: PeerTransport> {
    transport: T,
    hello: Hello,
    /// The host's lock: every change of a grant is saved under it first.
    grants: Mutex<Grants>,
    known: Mutex<Known>,
    /// Taken after `grants` and `known`, never before them.
    table: Mutex<Table<T::Conn>>,
    clock: Clock,
    pacing: Pacing,
    /// Counts changes of either record; a connection without a settled
    /// grant watches it to read again how long it may stay.
    relation: watch::Sender<u64>,
    /// Counts admissions; a dial in flight watches it.
    admitted: watch::Sender<u64>,
    /// One dial to a machine at a time: a second caller waits for the first
    /// and takes its connection.
    dialing: Mutex<HashMap<PeerKey, Arc<tokio::sync::Mutex<()>>>>,
    /// Connections whose first exchange is running.
    admitting: Arc<Semaphore>,
    events: broadcast::Sender<PeerEvent>,
    incoming: mpsc::Sender<Incoming<T::Conn>>,
    /// Counts wakes and network changes; a redial wait watches it.
    woke: watch::Sender<u64>,
    /// The hosts whose next first exchange asks them to update
    /// ([`Peers::ask_update`]): never more than the hosts this machine
    /// is an active guest of.
    update_asks: Mutex<HashSet<PeerKey>>,
    shutdown: watch::Sender<bool>,
}

/// The times of the climb and the beat: [`limits`]' own in a
/// daemon. A test shortens them to see a probe or a silent carrier without
/// waiting minutes for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacing {
    pub relay_grace: Duration,
    pub climb_first: Duration,
    pub climb_most: Duration,
    pub beat_every: Duration,
    pub beat_dead: Duration,
}

impl Default for Pacing {
    fn default() -> Self {
        Self {
            relay_grace: limits::RELAY_GRACE,
            climb_first: limits::CLIMB_FIRST,
            climb_most: limits::CLIMB_MOST,
            beat_every: limits::BEAT_EVERY,
            beat_dead: limits::BEAT_DEAD,
        }
    }
}

/// What a machine needs to say who it is.
pub struct Config<'a> {
    pub state_dir: &'a Path,
    /// This build's version, as an update notice names it.
    pub build: &'a str,
    /// This machine's product device id.
    pub device: &'a str,
    /// Whether this build is an official release.
    pub release: bool,
    /// What this machine calls itself, said in every hello so a paired
    /// machine shows a new name from its next connection.
    pub name: Option<&'a str>,
    pub clock: Clock,
    /// The climb's and the beat's times; [`Pacing::default`] in a daemon.
    pub pacing: Pacing,
}

/// The connections of one machine, over one transport.
pub struct Peers<T: PeerTransport> {
    inner: Arc<Inner<T>>,
}

impl<T: PeerTransport> Clone for Peers<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // A panic under one of these locks left a store that was saved first
    // and a table of plain values: both are still what they say.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A [`Peers`] that does not keep it open: what a carrier maker inside the
/// transport holds to reach the records it was opened with.
pub struct WeakPeers<T: PeerTransport> {
    inner: std::sync::Weak<Inner<T>>,
}

impl<T: PeerTransport> WeakPeers<T> {
    pub fn upgrade(&self) -> Option<Peers<T>> {
        self.inner.upgrade().map(|inner| Peers { inner })
    }
}

impl<T: PeerTransport> Peers<T> {
    /// A handle that does not keep these connections and records open.
    pub fn downgrade(&self) -> WeakPeers<T> {
        WeakPeers {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Opens the two records and starts accepting. Call it on the runtime
    /// the connections run on. The receiver yields every stream a peer
    /// opens and this machine accepts; [`Peers::close`] ends everything
    /// started here.
    pub fn start(
        transport: T,
        config: Config<'_>,
    ) -> io::Result<(Self, mpsc::Receiver<Incoming<T::Conn>>)> {
        let mut grants = Grants::open(layout::peer_grants(config.state_dir))?;
        grants
            .expire(config.clock.now())
            .map_err(io::Error::other)?;
        let known = Known::open(layout::peer_known(config.state_dir))?;
        let (incoming, accepted) = mpsc::channel(limits::MAX_STREAMS);
        let inner = Arc::new(Inner {
            transport,
            hello: Hello {
                name: config.name.and_then(crate::link::clean_name),
                ..Hello::ours(config.build, config.device, config.release)
            },
            grants: Mutex::new(grants),
            known: Mutex::new(known),
            table: Mutex::new(Table {
                live: HashMap::new(),
                draining: HashMap::new(),
                superseded: HashMap::new(),
            }),
            clock: config.clock,
            pacing: config.pacing,
            relation: watch::channel(0).0,
            admitted: watch::channel(0).0,
            dialing: Mutex::new(HashMap::new()),
            admitting: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            events: broadcast::channel(limits::MAX_EVENTS).0,
            incoming,
            woke: watch::channel(0).0,
            update_asks: Mutex::default(),
            shutdown: watch::channel(false).0,
        });
        tokio::spawn(Arc::clone(&inner).accept_connections());
        Ok((Self { inner }, accepted))
    }

    pub fn key(&self) -> PeerKey {
        self.inner.transport.key()
    }

    pub fn transport(&self) -> &T {
        &self.inner.transport
    }

    pub fn events(&self) -> broadcast::Receiver<PeerEvent> {
        self.inner.events.subscribe()
    }

    /// The live connection to `key`, when there is one.
    pub fn link(&self, key: &PeerKey) -> Option<Link<T::Conn>> {
        self.inner.link(key)
    }

    /// The peer as the rest of Hide names it, when it is paired in either
    /// direction.
    pub fn identity(&self, key: &PeerKey) -> Option<PeerIdentity> {
        let device = lock(&self.inner.grants)
            .current(key)
            .map(|grant| grant.device.clone())
            .or_else(|| {
                lock(&self.inner.known)
                    .of(key)
                    .map(|host| host.device.clone())
            })?;
        Some(PeerIdentity { key: *key, device })
    }

    /// Connects to a machine, or returns the connection already live.
    pub async fn dial(&self, to: &PeerAddr) -> Result<Link<T::Conn>, DialError> {
        self.inner.dial(to).await
    }

    /// Keeps this machine connected to a host it knows: dials, and dials
    /// again after a drop, until the returned [`Keeper`] is dropped or the
    /// host is forgotten.
    pub fn keep(&self, key: PeerKey) -> Keeper {
        let (stop, stopped) = watch::channel(false);
        tokio::spawn(Arc::clone(&self.inner).keep(key, stopped));
        Keeper { _stop: stop }
    }

    /// Tells every redial wait that the machine woke or its network
    /// changed: the next dial is not held back by an earlier failure (B15).
    pub fn network_changed(&self) {
        self.inner.transport.network_changed();
        self.inner.woke.send_modify(|count| *count += 1);
        // A timer does not run while the machine sleeps: what a connection
        // may stay for is read again against the clock.
        self.inner.relation_changed();
    }

    /// Reads the host's record, with the time to read it at.
    pub fn grants<R>(&self, read: impl FnOnce(&Grants, u64) -> R) -> R {
        read(&lock(&self.inner.grants), self.inner.clock.now())
    }

    /// What the streams `key` opens here run under now.
    pub fn granting(&self, key: &PeerKey) -> Change {
        self.grants(|grants, now| grants.state_of(key, now))
    }

    /// The time the records are stamped with, as Unix milliseconds.
    pub fn now(&self) -> u64 {
        self.inner.clock.now()
    }

    /// The host this machine is a guest of under `key`.
    pub fn host(&self, key: &PeerKey) -> Option<Host> {
        lock(&self.inner.known).of(key).cloned()
    }

    /// Every host this machine is a guest of.
    pub fn hosts(&self) -> Vec<Host> {
        lock(&self.inner.known).list().to_vec()
    }

    /// Moves a host this machine is a guest of to `state`: saved first,
    /// then the streams opened there end when this machine now holds its
    /// side back, and open again under what the host last granted when it
    /// no longer does (B25).
    pub fn set_host(&self, key: &PeerKey, state: KnownState) -> Result<(), StoreError> {
        {
            let mut known = lock(&self.inner.known);
            known.set_state(key, state)?;
            if let Some(link) = lock(&self.inner.table).live.get(key) {
                link.inner.outbound.set(if state.held_back() {
                    CLOSED
                } else {
                    link.inner.heard.load(Ordering::Acquire)
                });
            }
        }
        self.inner.relation_changed();
        Ok(())
    }

    /// Asks the host `key` to update itself although the two builds share
    /// no number: the next first exchange with it says so once
    /// ([`Hello::update`]), and the host hears it only while this machine
    /// holds an active grant there (D-40). A connection that lives asks
    /// with [`Link::request_update`] instead. Refused for a machine this
    /// one is no active guest of.
    pub fn ask_update(&self, key: &PeerKey) -> Result<(), OpenError> {
        let known = lock(&self.inner.known);
        let active = |key: &PeerKey| {
            known
                .of(key)
                .is_some_and(|host| host.state == KnownState::Active)
        };
        if !active(key) {
            return Err(OpenError::NoGrant(GrantState::None));
        }
        // An ask of a host that was removed before it was said goes with
        // it, so the set never holds more than the hosts this machine opens.
        let mut asks = lock(&self.inner.update_asks);
        asks.retain(|asked| active(asked));
        asks.insert(*key);
        Ok(())
    }

    /// Reads or changes the guest's record.
    pub fn known<R>(&self, change: impl FnOnce(&mut Known) -> R) -> R {
        let changed = change(&mut lock(&self.inner.known));
        self.inner.relation_changed();
        changed
    }

    /// Saves a pending grant for a guest the operator allowed. It opens
    /// nothing and touches no connection (D-30).
    pub fn grant_pending(
        &self,
        key: PeerKey,
        device: &str,
        name: &str,
        via: Via,
    ) -> Result<Grant, StoreError> {
        let now = self.inner.clock.now();
        let grant = lock(&self.inner.grants).grant_pending(key, device, name, via, now)?;
        self.inner.relation_changed();
        Ok(grant)
    }

    /// The guest's confirm of grant `id`. One that changes nothing (the
    /// grant is settled already, or is not there) touches no connection, so
    /// its answer reaches the guest on the stream it asked on.
    pub fn confirm(&self, key: &PeerKey, id: &str) -> Result<ConfirmState, StoreError> {
        let (state, changed) = {
            let mut grants = lock(&self.inner.grants);
            let (state, change) = grants.confirm(key, id, self.inner.clock.now())?;
            if let Some(change) = change {
                self.inner.move_gate(change);
            }
            (state, change.is_some())
        };
        if changed {
            self.inner.settle(key);
        }
        Ok(state)
    }

    /// Pausing what is paused, like resuming what is active, changes
    /// nothing: the streams running under the grant go on.
    pub fn pause(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.change(key, |grants| grants.pause(key))
    }

    pub fn resume(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.change(key, |grants| grants.resume(key))
    }

    /// Removes the grant given to `key`; with `id`, only that grant.
    /// Removing what is not there is not an error.
    pub fn revoke(&self, key: &PeerKey, id: Option<&str>) -> Result<(), StoreError> {
        let now = self.inner.clock.now();
        self.change(key, |grants| grants.revoke(key, id, now))
    }

    /// Gives the machine under `key` the name this machine's operator
    /// chose, or with `None` takes it back to its own, in the grant it holds
    /// here and in the host record it keeps of it, whichever it has; the
    /// name it gives itself stays as it is, and neither moves a generation
    /// or a connection. Whether either knew it.
    pub fn rename(&self, key: &PeerKey, chosen: Option<&str>) -> Result<bool, StoreError> {
        let granted = lock(&self.inner.grants).rename(key, chosen)?;
        let known = lock(&self.inner.known).rename(key, chosen)?;
        Ok(granted || known)
    }

    /// Forgets a host this machine was a guest of. Whether it was known.
    pub fn forget(&self, key: &PeerKey) -> Result<bool, StoreError> {
        let removed = lock(&self.inner.known).remove(key)?;
        self.inner.settle(key);
        Ok(removed)
    }

    /// Saves a change of the grant given to `key`, then moves that
    /// direction's generation, then tells the peer. The save is first: a
    /// failed one moves nothing. A request that changes nothing tells
    /// nobody and closes nothing: a machine that asks for the removal of a
    /// grant it does not hold is answered on the stream it asked on.
    fn change(
        &self,
        key: &PeerKey,
        save: impl FnOnce(&mut Grants) -> Result<Option<Change>, StoreError>,
    ) -> Result<(), StoreError> {
        {
            let mut grants = lock(&self.inner.grants);
            let Some(change) = save(&mut grants)? else {
                return Ok(());
            };
            // Under the lock that saved it, so two changes move the gate
            // in the order they were saved.
            self.inner.move_gate(change);
        }
        self.inner.settle(key);
        Ok(())
    }

    /// Closes every connection and stops dialing and accepting.
    pub async fn close(&self) {
        self.inner.shutdown.send_replace(true);
        let links: Vec<_> = lock(&self.inner.table).live.values().cloned().collect();
        for link in links {
            link.close(CloseCode::Shutdown);
        }
        self.inner.transport.close().await;
    }
}

/// Holds a machine connected; dropping it stops the redial.
pub struct Keeper {
    _stop: watch::Sender<bool>,
}

impl<T: PeerTransport> Inner<T> {
    /// The live connection to `key`. One that has ended and is still on
    /// its way out of the table is not live.
    fn link(&self, key: &PeerKey) -> Option<Link<T::Conn>> {
        lock(&self.table)
            .live
            .get(key)
            .filter(|link| link.inner.conn.ended().is_none())
            .cloned()
    }

    fn emit(&self, event: PeerEvent) {
        // No listener is not a failure: the daemon may not show peers yet.
        let _ = self.events.send(event);
    }

    /// Why a connection to `key` may stay, as the two records stand now. A
    /// pending grant past its time is no reason.
    fn reason(&self, key: &PeerKey) -> Reason {
        let grants = lock(&self.grants);
        let known = lock(&self.known);
        reason_in(&grants, &known, key, self.clock.now())
    }

    /// Whether this machine and `key` are paired in either direction, a
    /// pending grant that still waits included.
    fn related(&self, key: &PeerKey) -> bool {
        self.reason(key) != Reason::None
    }

    fn relation_changed(&self) {
        self.relation.send_modify(|count| *count += 1);
    }

    /// Keeps the name a paired machine calls itself in its hello, in the
    /// grant it holds here and the host record kept of it, so a machine
    /// that was renamed is shown by its new name from this connection on,
    /// without pairing again. A hello that says none, or a name with
    /// nothing showable, keeps the name it paired under; a stranger's
    /// hello names nothing here, since neither record has its key.
    fn take_name(&self, key: &PeerKey, name: Option<&str>) {
        let Some(name) = name.and_then(crate::link::clean_name) else {
            return;
        };
        let granted = lock(&self.grants).take_name(key, &name);
        let known = lock(&self.known).take_name(key, &name);
        match (granted, known) {
            (Ok(false), Ok(false)) => {}
            (Ok(_), Ok(_)) => self.relation_changed(),
            (granted, known) => {
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "peer.name_not_saved",
                    "peer": key.short(),
                    "reason": granted.err().or(known.err()).map(|error| error.to_string()),
                }));
                self.relation_changed();
            }
        }
    }

    /// Moves the generation of the grant given to `change.key`: every
    /// stream that peer opened under the earlier one ends. The gate is the
    /// live connection's own, so nothing is kept for a machine that is not
    /// connected; the next connection reads its generation from the record.
    /// Called under the grants lock, which admission holds too, so a
    /// connection is either in the table before the change or reads it.
    fn move_gate(&self, change: Change) {
        if let Some(link) = lock(&self.table).live.get(&change.key) {
            link.inner.inbound.set(change.generation);
        }
    }

    /// After a record changed: tells the peer the grant it now has, and
    /// closes the connection when no grant is left in either direction.
    fn settle(&self, key: &PeerKey) {
        self.relation_changed();
        let Some(link) = self.link(key) else {
            return;
        };
        let change = lock(&self.grants).state_of(key, self.clock.now());
        link.tell(HelloFrame::Grant {
            state: change.state,
            generation: change.generation,
        });
        if !self.related(key) {
            link.close(CloseCode::NoGrant);
        }
    }

    async fn accept_connections(self: Arc<Self>) {
        let mut shutdown = self.shutdown.subscribe();
        let mut woke = self.woke.subscribe();
        // A transport that cannot accept (its port is taken, there is no
        // network) is asked again as a machine is dialed again: after a
        // wait that doubles, and at once when the network changed. Only
        // the change between failing and accepting is recorded.
        let mut wait = limits::REDIAL_FIRST;
        let mut failing = false;
        loop {
            let conn = tokio::select! {
                _ = shutdown.wait_for(|stop| *stop) => return,
                conn = self.transport.accept() => conn,
            };
            match conn {
                Ok(conn) => {
                    if failing {
                        failing = false;
                        crate::diagnostic!(json!({
                            "component": "peer",
                            "event": "accept.recovered",
                        }));
                    }
                    wait = limits::REDIAL_FIRST;
                    // At the cap a connection is closed before any of its
                    // bytes are read.
                    let Ok(permit) = Arc::clone(&self.admitting).try_acquire_owned() else {
                        conn.close(CloseCode::Busy);
                        continue;
                    };
                    let inner = Arc::clone(&self);
                    tokio::spawn(async move {
                        let _permit = permit;
                        let established = inner.establish(conn).await;
                        // A connection the other side dialed to try a path
                        // and did not choose is no failure: a probe repeats
                        // for as long as a pair has no direct path.
                        let probed = matches!(
                            established,
                            Err(
                                DialError::Transport(TransportError::Ended(CloseCode::Probe))
                                    | DialError::Refused(CloseCode::Probe)
                            )
                        );
                        if let Err(error) = established.as_ref()
                            && !probed
                        {
                            crate::diagnostic!(json!({
                                "component": "peer",
                                "event": "connection.not_admitted",
                                "reason": error.to_string(),
                            }));
                        }
                    });
                }
                Err(TransportError::Closed) => return,
                Err(error) => {
                    if !failing {
                        failing = true;
                        crate::diagnostic!(json!({
                            "component": "peer",
                            "event": "accept.failed",
                            "reason": error.to_string(),
                        }));
                    }
                    tokio::select! {
                        () = tokio::time::sleep(wait) => wait = next_wait(wait),
                        changed = woke.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            wait = limits::REDIAL_FIRST;
                        }
                        _ = shutdown.wait_for(|stop| *stop) => return,
                    }
                }
            }
        }
    }

    /// Waits until every connection to `key` closed here has ended.
    async fn drained(&self, key: &PeerKey) {
        loop {
            let Some(conn) = lock(&self.table)
                .draining
                .get(key)
                .and_then(|conns| conns.first().cloned())
            else {
                return;
            };
            conn.closed().await;
            self.undrain(key, &conn);
        }
    }

    /// `conn`, closed here, ended: it no longer holds up a dial to `key`.
    fn undrain(&self, key: &PeerKey, conn: &T::Conn) {
        let mut table = lock(&self.table);
        if let Some(conns) = table.draining.get_mut(key) {
            conns.retain(|draining| draining.id() != conn.id());
            if conns.is_empty() {
                table.draining.remove(key);
            }
        }
    }

    async fn dial(self: &Arc<Self>, to: &PeerAddr) -> Result<Link<T::Conn>, DialError> {
        self.dial_replacing(to, 0).await
    }

    /// A dial that names `replaces`, the connection to `to` that ended on
    /// this end before it, so a late first exchange of that one is refused
    /// on the other ([`decide`]); `0` names none.
    async fn dial_replacing(
        self: &Arc<Self>,
        to: &PeerAddr,
        replaces: u64,
    ) -> Result<Link<T::Conn>, DialError> {
        let turn = Arc::clone(lock(&self.dialing).entry(to.key).or_default());
        let dialed = {
            let _turn = turn.lock().await;
            self.dial_in_turn(to, replaces).await
        };
        drop(turn);
        // The entry goes with its last caller.
        let mut dialing = lock(&self.dialing);
        if dialing
            .get(&to.key)
            .is_some_and(|turn| Arc::strong_count(turn) == 1)
        {
            dialing.remove(&to.key);
        }
        dialed
    }

    async fn dial_in_turn(
        self: &Arc<Self>,
        to: &PeerAddr,
        replaces: u64,
    ) -> Result<Link<T::Conn>, DialError> {
        let mut admitted = self.admitted.subscribe();
        if let Some(link) = self.link(&to.key) {
            return Ok(link);
        }
        self.drained(&to.key).await;
        // Only the transport's dial is given up for the peer's connection.
        // Once ours is established both sides see two, and `decide` leaves
        // the same one on both.
        let conn = tokio::select! {
            link = self.arrived(&to.key, &mut admitted) => return Ok(link),
            conn = tokio::time::timeout(limits::DIAL_WITHIN, self.transport.dial(to)) => {
                conn.map_err(|_| DialError::TimedOut)??
            }
        };
        let failed = match self.establish_replacing(conn.clone(), replaces).await {
            Ok(link) => return Ok(link),
            Err(failed) => failed,
        };
        if let Some(link) = self.link(&to.key) {
            return Ok(link);
        }
        // The peer closed ours as the duplicate of the one it dialed, which
        // may have cut our first exchange short. Its own is being admitted
        // here: that one is the connection.
        let lost_to_theirs = conn.ended() == Some(CloseCode::Duplicate)
            || matches!(failed, DialError::Refused(CloseCode::Duplicate));
        if lost_to_theirs
            && let Ok(link) =
                tokio::time::timeout(limits::HELLO_WITHIN, self.arrived(&to.key, &mut admitted))
                    .await
        {
            return Ok(link);
        }
        Err(failed)
    }

    /// Resolves when a connection to `key` is live.
    async fn arrived(&self, key: &PeerKey, admitted: &mut watch::Receiver<u64>) -> Link<T::Conn> {
        loop {
            if let Some(link) = self.link(key) {
                return link;
            }
            if admitted.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    async fn first_exchange(
        &self,
        conn: &T::Conn,
        asks_update: bool,
        dial: u64,
        replaces: u64,
        supersedes: Vec<u64>,
    ) -> Result<
        (
            Hello,
            <T::Conn as PeerConn>::Send,
            <T::Conn as PeerConn>::Recv,
        ),
        DialError,
    > {
        let ours = Hello {
            update: asks_update,
            dial,
            replaces,
            supersedes,
            ..self.hello.clone()
        }
        .encode()
        .map_err(|error| DialError::Protocol(error.to_string()))?;
        let refused = |why: &str| DialError::Protocol(why.to_owned());
        let (send, mut recv) = if conn.dialed() {
            let (mut send, mut recv) = conn.open(StreamKind::Hello.priority()).await?;
            send.write_all(&stream::encode_header(StreamKind::Hello, CLOSED))
                .await?;
            send.write_all(&ours).await?;
            send.flush().await?;
            let mut answer = [0u8; stream::ANSWER_BYTES];
            recv.read_exact(&mut answer).await?;
            if !matches!(stream::decode_answer(answer), Some((Status::Accepted, _))) {
                return Err(refused("the hello stream was refused"));
            }
            (send, recv)
        } else {
            let (mut send, mut recv) = conn.accept().await?;
            let mut header = [0u8; stream::HEADER_BYTES];
            recv.read_exact(&mut header).await?;
            if stream::decode_header(header).0 != StreamKind::Hello.tag() {
                return Err(refused("the first stream is not the hello stream"));
            }
            send.write_all(&stream::encode_answer(Status::Accepted, CLOSED))
                .await?;
            send.write_all(&ours).await?;
            send.flush().await?;
            (send, recv)
        };
        let mut prefix = [0u8; hello::PREFIX_BYTES];
        recv.read_exact(&mut prefix).await?;
        let length = hello::body_length(prefix).ok_or_else(|| refused("not a hello"))?;
        let mut body = vec![0u8; length];
        recv.read_exact(&mut body).await?;
        let theirs =
            Hello::decode(&body).map_err(|error| DialError::Protocol(error.to_string()))?;
        Ok((theirs, send, recv))
    }

    /// The first exchange and admission of a connection either side dialed.
    async fn establish(self: &Arc<Self>, conn: T::Conn) -> Result<Link<T::Conn>, DialError> {
        self.establish_replacing(conn, 0).await
    }

    /// [`Inner::establish`] of a connection this side dialed to replace the
    /// one named `replaces` (a move), or `0`.
    async fn establish_replacing(
        self: &Arc<Self>,
        conn: T::Conn,
        replaces: u64,
    ) -> Result<Link<T::Conn>, DialError> {
        let key = conn.peer();
        let asks_update = lock(&self.update_asks).contains(&key);
        // The dialer names each connection, and says which ones it knows
        // were replaced; the accepting side says none.
        let (dial, replaces, supersedes) = match conn.dialed() {
            true => (
                crate::key::dial_name()?,
                replaces,
                lock(&self.table).superseded(&key),
            ),
            false => (0, 0, Vec::new()),
        };
        let exchanged = tokio::time::timeout(
            limits::HELLO_WITHIN,
            self.first_exchange(&conn, asks_update, dial, replaces, supersedes.clone()),
        )
        .await;
        let (theirs, send, recv) = match exchanged {
            Ok(Ok(exchanged)) => {
                // Said once: the host read this exchange.
                if asks_update {
                    lock(&self.update_asks).remove(&key);
                }
                exchanged
            }
            Ok(Err(error)) => {
                conn.close(CloseCode::Protocol);
                return Err(error);
            }
            Err(_) => {
                conn.close(CloseCode::Protocol);
                return Err(DialError::TimedOut);
            }
        };
        let agreed = match hello::agree(&self.hello, &theirs) {
            Ok(agreed) => agreed,
            Err(behind) => {
                // Decided before admission, so a build that cannot connect
                // never pushes out a connection that works. Only a paired
                // machine's is shown: a stranger's hello carries words it
                // chose, and those go nowhere.
                if self.related(&key) {
                    self.emit(PeerEvent::UpdateNeeded {
                        key,
                        device: theirs.device.clone(),
                        build: theirs.build.clone(),
                        behind,
                    });
                } else {
                    crate::diagnostic!(json!({
                        "component": "peer",
                        "event": "hello.incompatible",
                        "peer": key.short(),
                        "behind": format!("{behind:?}"),
                    }));
                }
                // The other side asks this machine to update, in the one
                // exchange the two builds share. Only a guest that holds an
                // active grant here is heard, as on a connection that lives.
                if theirs.update && behind != Behind::Theirs {
                    let granted = lock(&self.grants)
                        .current(&key)
                        .is_some_and(|grant| grant.state == GrantStatus::Active);
                    if granted {
                        self.emit(PeerEvent::UpdateRequested { key });
                    } else {
                        crate::diagnostic!(json!({
                            "component": "peer",
                            "event": "update.refused",
                            "peer": key.short(),
                            "reason": "the peer holds no active grant here",
                        }));
                    }
                }
                conn.close(CloseCode::UpdateNeeded);
                return Err(DialError::UpdateNeeded(behind));
            }
        };
        let moves = agreed.has(hello::SSH_PATH);
        // An SSH connection exists only between builds that both name it:
        // the far `carry --ssh` refuses any other first, so this holds as a
        // second wall.
        if conn.path() == PathKind::Ssh && !moves {
            conn.close(CloseCode::Probe);
            return Err(DialError::Protocol(
                "an SSH connection with a build that does not name ssh_path".to_owned(),
            ));
        }
        // Both ends read the names from the dialer's hello. With a build
        // that names none, every connection names nothing and the rule is
        // the one both builds share.
        let (named, supersedes) = match (moves, conn.dialed()) {
            (false, by_us) => (
                Named {
                    dialed_by_us: by_us,
                    dial: 0,
                    replaces: 0,
                },
                Vec::new(),
            ),
            (true, true) => (
                Named {
                    dialed_by_us: true,
                    dial,
                    replaces,
                },
                supersedes,
            ),
            (true, false) => (
                Named {
                    dialed_by_us: false,
                    dial: theirs.dial,
                    replaces: theirs.replaces,
                },
                theirs
                    .supersedes
                    .iter()
                    .copied()
                    .filter(|name| *name != 0)
                    .take(hello::MAX_SUPERSEDES)
                    .collect(),
            ),
        };
        let (frames, queued) = mpsc::channel(limits::MAX_EVENTS);
        let counts = Arc::new(Counts::default());
        // The hello stream holds its kind's one place for the connection's
        // life, so a second one is refused at the limit.
        let hello_place = counts.take(StreamKind::Hello);
        let link = {
            // Under the host's lock: a grant change either finds this
            // connection in the table or is read here.
            let grants = lock(&self.grants);
            let known = lock(&self.known);
            let now = self.clock.now();
            let unrelated = |key: &PeerKey| reason_in(&grants, &known, key, now) == Reason::None;
            let mut table = lock(&self.table);
            let held = table
                .live
                .get(&key)
                .filter(|link| link.inner.conn.ended().is_none())
                .map(|link| link.inner.named);
            let superseded = table.superseded(&key);
            let standing = decide(
                &self.transport.key(),
                &key,
                held,
                named,
                &supersedes,
                &superseded,
            );
            if let Standing::Refuse(code) = standing {
                // Ended here for another: dead on both ends, whenever the
                // other hears it.
                if named.dial != 0 {
                    table.supersede(key, named.dial);
                }
                conn.close(code);
                return Err(DialError::Refused(code));
            }
            if unrelated(&key) {
                let ungranted = table
                    .live
                    .keys()
                    .filter(|other| **other != key && unrelated(other))
                    .count();
                if ungranted >= limits::MAX_UNGRANTED {
                    conn.close(CloseCode::Busy);
                    return Err(DialError::Refused(CloseCode::Busy));
                }
            }
            let granting = grants.state_of(&key, now);
            let link = Link {
                inner: Arc::new(LinkInner {
                    conn: conn.clone(),
                    theirs,
                    agreed,
                    named,
                    inbound: Gate::new(granting.generation),
                    outbound: Gate::new(CLOSED),
                    heard: AtomicU64::new(CLOSED),
                    granted: AtomicU8::new(state_tag(GrantState::None)),
                    counts,
                    headers: Arc::new(Semaphore::new(limits::MAX_STREAMS)),
                    frames,
                }),
            };
            // Read under the same lock as the install, so a predecessor
            // that arrives later always finds it.
            // The names the dialer knew first, then the one this connection
            // replaces, so at the cap the name row 1 needs most is kept.
            for name in supersedes
                .into_iter()
                .chain(std::iter::once(named.replaces))
            {
                if name != 0 {
                    table.supersede(key, name);
                }
            }
            if let Some(old) = table.live.insert(key, link.clone())
                && let Standing::Replace(code) = standing
            {
                if old.inner.named.dial != 0 {
                    table.supersede(key, old.inner.named.dial);
                }
                old.close(code);
                table
                    .draining
                    .entry(key)
                    .or_default()
                    .push(old.inner.conn.clone());
            }
            link.tell(HelloFrame::Grant {
                state: granting.state,
                generation: granting.generation,
            });
            link
        };
        self.take_name(&key, link.their_hello().name.as_deref());
        self.admitted.send_modify(|count| *count += 1);
        tokio::spawn(Arc::clone(self).write_hello_frames(link.clone(), send, queued, hello_place));
        tokio::spawn(Arc::clone(self).read_hello_frames(link.clone(), recv));
        tokio::spawn(Arc::clone(self).accept_streams(link.clone()));
        tokio::spawn(Arc::clone(self).watch_end(link.clone()));
        // Read once, so a move before the watcher first runs is still heard.
        let path = link.path();
        tokio::spawn(Arc::clone(self).watch_path(link.clone(), path));
        self.emit(PeerEvent::Connected { key, path });
        Ok(link)
    }

    async fn write_hello_frames(
        self: Arc<Self>,
        link: Link<T::Conn>,
        mut send: <T::Conn as PeerConn>::Send,
        mut queued: mpsc::Receiver<HelloFrame>,
        _place: Option<Place>,
    ) {
        // An SSH connection beats, so a carrier that went silent is closed
        // on both ends.
        let beats = link.path() == PathKind::Ssh && link.can_move();
        let mut beat = tokio::time::interval_at(
            Instant::now() + self.pacing.beat_every,
            self.pacing.beat_every,
        );
        beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let frame = tokio::select! {
                frame = queued.recv() => frame,
                _ = beat.tick(), if beats => Some(HelloFrame::Beat),
                _ = link.closed() => None,
            };
            let Some(frame) = frame else {
                return;
            };
            if wire::write_message(&mut send, &frame).await.is_err() {
                // The hello stream is the connection's spine: without it
                // neither side learns of a grant change.
                link.close(CloseCode::Lost);
                return;
            }
        }
    }

    async fn read_hello_frames(
        self: Arc<Self>,
        link: Link<T::Conn>,
        mut recv: <T::Conn as PeerConn>::Recv,
    ) {
        let key = link.key();
        let beats = link.path() == PathKind::Ssh && link.can_move();
        loop {
            let read = wire::read_message(&mut recv, hello::MAX_HELLO);
            let read = match beats {
                true => match tokio::time::timeout(self.pacing.beat_dead, read).await {
                    Ok(read) => read,
                    // Nothing for three beats: the carrier went silent.
                    Err(_) => {
                        link.close(CloseCode::Lost);
                        return;
                    }
                },
                false => read.await,
            };
            let body = match read {
                Ok(Some(body)) => body,
                Ok(None) | Err(_) => break,
            };
            match HelloFrame::parse(&body) {
                Ok(Some(HelloFrame::Grant { state, generation })) => {
                    link.inner
                        .granted
                        .store(state_tag(state), Ordering::Release);
                    let generation = if state == GrantState::Active {
                        generation
                    } else {
                        CLOSED
                    };
                    {
                        // Under the guest's lock, as `set_host` is: the
                        // gate is opened by whichever of the two ran last.
                        let known = lock(&self.known);
                        link.inner.heard.store(generation, Ordering::Release);
                        link.inner.outbound.set(if held_back(&known, &key) {
                            CLOSED
                        } else {
                            generation
                        });
                    }
                    self.emit(PeerEvent::Granted { key, state });
                }
                Ok(Some(HelloFrame::Update)) => {
                    // Only a peer that holds an active grant here may ask.
                    if link.inner.inbound.generation() != CLOSED {
                        self.emit(PeerEvent::UpdateRequested { key });
                    } else {
                        crate::diagnostic!(json!({
                            "component": "peer",
                            "event": "update.refused",
                            "peer": key.short(),
                            "reason": "the peer holds no active grant here",
                        }));
                    }
                }
                Ok(Some(HelloFrame::UpdateAnswer { started, reason })) => {
                    self.emit(PeerEvent::UpdateAnswered {
                        key,
                        started,
                        reason,
                    });
                }
                // Read is heard: the wait above starts over.
                Ok(Some(HelloFrame::Beat)) => {}
                // A frame of a later build.
                Ok(None) => {}
                Err(_) => break,
            }
        }
        link.close(CloseCode::Protocol);
    }

    async fn accept_streams(self: Arc<Self>, link: Link<T::Conn>) {
        while let Ok((send, recv)) = link.inner.conn.accept().await {
            // Past the cap of streams still reading their header, a stream
            // is dropped unread, which closes it.
            let Ok(permit) = Arc::clone(&link.inner.headers).try_acquire_owned() else {
                continue;
            };
            let inner = Arc::clone(&self);
            let link = link.clone();
            tokio::spawn(async move {
                let _permit = permit;
                inner.accept_stream(link, send, recv).await;
            });
        }
    }

    async fn accept_stream(
        &self,
        link: Link<T::Conn>,
        mut send: <T::Conn as PeerConn>::Send,
        mut recv: <T::Conn as PeerConn>::Recv,
    ) {
        let mut header = [0u8; stream::HEADER_BYTES];
        let read = tokio::time::timeout(limits::HELLO_WITHIN, recv.read_exact(&mut header)).await;
        if !matches!(read, Ok(Ok(_))) {
            return;
        }
        let (tag, generation) = stream::decode_header(header);
        let (status, accepted) = link.judge(tag, generation);
        let answer = stream::encode_answer(status, link.inner.inbound.generation());
        if send.write_all(&answer).await.is_err() || send.flush().await.is_err() {
            return;
        }
        let Some((kind, pass, place)) = accepted else {
            return;
        };
        let hold: Arc<dyn Any + Send + Sync> = Arc::new(place);
        let stream = PeerStream {
            kind,
            send: Guarded::new(
                send,
                pass.as_ref().map(|pass| pass.twin()),
                Some(Arc::clone(&hold)),
            ),
            recv: Guarded::new(recv, pass, Some(hold)),
        };
        if self.incoming.send(Incoming { link, stream }).await.is_err() {
            crate::diagnostic!(json!({
                "component": "peer",
                "event": "stream.unclaimed",
                "kind": kind.name(),
            }));
        }
    }

    /// When a connection admitted at `admitted` has to have a settled grant
    /// by, as the records stand now. `None` while one is settled.
    fn leave_by(&self, key: &PeerKey, admitted: Instant) -> Option<Instant> {
        let ungranted = admitted + limits::UNGRANTED_FOR;
        match self.reason(key) {
            Reason::Settled => None,
            // The grant's own wait may end later than the connection's.
            Reason::Pending { left } => Some(ungranted.max(Instant::now() + left)),
            Reason::None => Some(ungranted),
        }
    }

    /// Says when the link's bytes move off `path`, the path it was admitted
    /// on, and each later move, until it ends.
    async fn watch_path(self: Arc<Self>, link: Link<T::Conn>, mut path: PathKind) {
        let key = link.key();
        loop {
            tokio::select! {
                _ = link.closed() => return,
                now = link.inner.conn.path_changed(path) => {
                    path = now;
                    self.emit(PeerEvent::PathChanged { key, path });
                }
            }
        }
    }

    /// Ends a connection's place in the table when it ends, and closes one
    /// that stays without a settled grant in either direction for too long.
    /// How long is read again whenever a record changes, so a pending grant
    /// that was never confirmed, or a grant removed as its guest connected,
    /// does not keep a connection the limit should have closed.
    async fn watch_end(self: Arc<Self>, link: Link<T::Conn>) {
        let key = link.key();
        let admitted = Instant::now();
        let mut relation = self.relation.subscribe();
        let code = loop {
            // Seen before the read, so a change after it wakes the wait.
            relation.borrow_and_update();
            let by = self.leave_by(&key, admitted);
            let overstayed = async {
                match by {
                    Some(by) => tokio::time::sleep_until(by).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                code = link.closed() => break code,
                changed = relation.changed() => {
                    if changed.is_err() {
                        break link.closed().await;
                    }
                }
                () = overstayed => {
                    // Read again at the time: a grant may have settled, or
                    // a newer pending one may wait longer.
                    let over = self
                        .leave_by(&key, admitted)
                        .is_some_and(|by| by <= Instant::now());
                    if over {
                        link.close(CloseCode::NoGrant);
                        break link.closed().await;
                    }
                }
            }
        };
        {
            let mut table = lock(&self.table);
            if table
                .live
                .get(&key)
                .is_some_and(|live| live.id() == link.id())
            {
                table.live.remove(&key);
            }
            if let Some(conns) = table.draining.get_mut(&key) {
                conns.retain(|conn| conn.id() != link.id());
                if conns.is_empty() {
                    table.draining.remove(&key);
                }
            }
        }
        link.inner.outbound.set(CLOSED);
        self.emit(PeerEvent::Disconnected { key, code });
    }

    /// Dials `to` to move the live connection `held` to a better path
    ///: in the same turn as a dial, through the same
    /// first exchange, naming `held` as the connection it replaces. Unlike
    /// a dial it returns no live connection in place of its own and races
    /// no arriving one; and a connection that does not beat `held`'s path,
    /// read again before its hello (a hole punch may have landed during an
    /// SSH sign-in) or within [`limits::DIRECT_WITHIN`] for a probe, is
    /// closed [`CloseCode::Probe`] unadmitted.
    async fn dial_move(
        self: &Arc<Self>,
        to: &PeerAddr,
        held: &Link<T::Conn>,
        over_route: bool,
    ) -> Result<Link<T::Conn>, NotMoved> {
        let turn = Arc::clone(lock(&self.dialing).entry(to.key).or_default());
        let moved = {
            let _turn = turn.lock().await;
            self.move_in_turn(to, held, over_route).await
        };
        drop(turn);
        let mut dialing = lock(&self.dialing);
        if dialing
            .get(&to.key)
            .is_some_and(|turn| Arc::strong_count(turn) == 1)
        {
            dialing.remove(&to.key);
        }
        moved
    }

    async fn move_in_turn(
        self: &Arc<Self>,
        to: &PeerAddr,
        held: &Link<T::Conn>,
        over_route: bool,
    ) -> Result<Link<T::Conn>, NotMoved> {
        self.drained(&to.key).await;
        let start = Instant::now();
        let dialing = async {
            match over_route {
                true => self.transport.dial_route(to).await,
                false => self.transport.dial(to).await,
            }
        };
        let conn = match tokio::time::timeout(limits::DIAL_WITHIN, dialing).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(_)) | Err(_) => return Err(NotMoved::NotDialed),
        };
        // From here the connection is this move's: whichever way the move
        // ends short of an install, its being given up included, it is
        // closed and drained before the machine is dialed again.
        let unadmitted = Unadmitted {
            inner: Arc::clone(self),
            conn: Some(conn.clone()),
        };
        let beats = |conn: &T::Conn| {
            conn.chosen_path()
                .is_some_and(|path| path.rank() > held.path().rank())
        };
        while !beats(&conn) {
            if start.elapsed() >= limits::DIRECT_WITHIN || conn.ended().is_some() {
                return Err(NotMoved::NotTaken(DialError::Refused(CloseCode::Probe)));
            }
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: a transport's chosen path has no change event before it chose one
        }
        let from = held.path();
        let link = self
            .establish_replacing(conn, held.dial_name())
            .await
            .map_err(NotMoved::NotTaken)?;
        unadmitted.admitted();
        crate::diagnostic!(json!({
            "component": "peer",
            "event": "peer.moved",
            "peer": to.key.short(),
            "from": format!("{from:?}"),
            "to": format!("{:?}", link.path()),
        }));
        Ok(link)
    }

    /// One step of the climb on the live connection `link`: what it waits
    /// for, and the move it makes, if any. A connection between builds that
    /// cannot move never climbs; one to a machine with no SSH alias waits
    /// for the records to change.
    async fn climb(
        self: &Arc<Self>,
        link: &Link<T::Conn>,
        to: &PeerAddr,
        probe: &mut Probe,
        woke: &mut watch::Receiver<u64>,
        relation: &mut watch::Receiver<u64>,
    ) -> Option<Link<T::Conn>> {
        if !link.can_move() {
            // A capability cannot change inside a connection.
            return std::future::pending().await;
        }
        let Some(alias) = ssh_alias(to) else {
            // An alias the operator adds later is read when the record
            // changes or the network does, and the caller reads the
            // address again. The caller marked the records seen before it
            // read them, so a change since is not missed.
            tokio::select! {
                _ = relation.changed() => {}
                _ = woke.changed() => {}
            }
            return None;
        };
        let path = link.path();
        let over_ssh = PeerAddr::new(to.key, vec![Candidate::Ssh(alias.to_owned())]);
        if path.rank() < PathKind::Ssh.rank() {
            let to_ssh = || async {
                // A carrier that proved and was not taken (its first
                // exchange failed, or the held path became direct meanwhile)
                // waits as a fall does; a sign-in that failed was counted
                // by its own attempt. Each failure is counted once.
                match self.dial_move(&over_ssh, link, true).await {
                    Ok(moved) => Some(moved),
                    Err(NotMoved::NotTaken(error)) => {
                        crate::diagnostic!(json!({
                            "component": "peer",
                            "event": "peer.move_not_taken",
                            "peer": to.key.short(),
                            "reason": error.to_string(),
                        }));
                        self.transport.route_fell(&to.key);
                        None
                    }
                    Err(NotMoved::NotDialed) => None,
                }
            };
            match self.transport.route(to) {
                Route::Handed => return to_ssh().await,
                Route::Ready | Route::Due => {
                    // The other transport's hole punch gets its time first.
                    tokio::select! {
                        () = tokio::time::sleep(self.pacing.relay_grace) => {}
                        _ = link.inner.conn.path_changed(path) => return None,
                    }
                    if link.path().rank() >= PathKind::Ssh.rank() {
                        return None;
                    }
                    return to_ssh().await;
                }
                // A wait runs out, or the network changes: read again.
                Route::Held | Route::None => {
                    tokio::select! {
                        () = tokio::time::sleep(self.pacing.relay_grace) => {}
                        _ = link.inner.conn.path_changed(path) => {}
                        _ = woke.changed() => {}
                    }
                    return None;
                }
            }
        }
        if path == PathKind::Ssh && link.dialed() {
            // The side that dialed SSH looks for a direct path beside it.
            tokio::select! {
                () = tokio::time::sleep_until(probe.at) => {}
                changed = woke.changed() => {
                    if changed.is_err() {
                        return std::future::pending().await;
                    }
                    probe.wait = self.pacing.climb_first;
                }
            }
            let moved = self.dial_move(&without_ssh(to), link, false).await.ok();
            if moved.is_none() {
                probe.wait = (probe.wait * 2).min(self.pacing.climb_most);
                probe.at = Instant::now() + probe.wait;
            }
            return moved;
        }
        // A direct path moves inside its own connection; SSH the other side
        // dialed is that side's to probe.
        link.inner.conn.path_changed(path).await;
        None
    }

    /// Holds the connection `link` to the host this machine keeps, and the
    /// connections that take its place, climbing while one lives. Answers
    /// how long they lived together, and the name of the last one, which
    /// the next dial names as the one it replaces.
    async fn hold(self: &Arc<Self>, link: Link<T::Conn>, to: &PeerAddr) -> (Duration, u64) {
        let key = to.key;
        let started = Instant::now();
        let mut woke = self.woke.subscribe();
        let mut relation = self.relation.subscribe();
        let mut current = link;
        // When the current connection came, and whether a move brought it.
        let (mut since, mut moved) = (Instant::now(), false);
        let mut probe = Probe::after(since, self.pacing.climb_first);
        loop {
            // The host's address as the records say it now: an alias the
            // operator added since the dial is climbed to. The records are
            // marked seen first, so a change after this read wakes the
            // climb.
            relation.borrow_and_update();
            let to = lock(&self.known)
                .of(&key)
                .map(Host::dial_addr)
                .unwrap_or_else(|| to.clone());
            let next = {
                let climbing = self.climb(&current, &to, &mut probe, &mut woke, &mut relation);
                tokio::pin!(climbing);
                let climbed = tokio::select! {
                    biased;
                    moved_to = &mut climbing => match moved_to {
                        Some(link) => Ok(link),
                        None => continue,
                    },
                    code = current.closed() => Err(code),
                };
                match climbed {
                    Ok(link) => link,
                    Err(code) => {
                        let replaced = matches!(code, CloseCode::Replaced | CloseCode::Duplicate);
                        // A path that keeps falling is signed in to at most
                        // once per the route's wait.
                        let stayed = since.elapsed() >= limits::STAYED_AFTER;
                        if !replaced && !stayed && (moved || current.path() == PathKind::Ssh) {
                            self.transport.route_fell(&key);
                        }
                        // An SSH connection that stayed starts the route's
                        // wait over; a short one keeps the wait it doubled.
                        if !replaced && stayed && current.path() == PathKind::Ssh {
                            self.transport.route_stayed(&key);
                        }
                        let successor = match self.link(&key) {
                            Some(link) => Some(link),
                            // The other side installed a move first: its
                            // own, which is on its way here, or the one this
                            // climb has in flight, whose answer is, so the
                            // climb runs on until one of them is installed.
                            None if replaced => {
                                let mut admitted = self.admitted.subscribe();
                                let successor = async {
                                    tokio::select! {
                                        link = self.arrived(&key, &mut admitted) => link,
                                        Some(link) = &mut climbing => link,
                                    }
                                };
                                tokio::time::timeout(limits::HELLO_WITHIN, successor)
                                    .await
                                    .ok()
                            }
                            None => None,
                        };
                        match successor.filter(|link| link.id() != current.id()) {
                            Some(link) => link,
                            None => return (started.elapsed(), current.dial_name()),
                        }
                    }
                }
            };
            current = next;
            (since, moved) = (Instant::now(), true);
            probe = Probe::after(since, self.pacing.climb_first);
        }
    }

    /// The guest's redial: one connection at a time, a short wait after a
    /// drop, a longer one after each failure.
    async fn keep(self: Arc<Self>, key: PeerKey, mut stopped: watch::Receiver<bool>) {
        let mut shutdown = self.shutdown.subscribe();
        let mut woke = self.woke.subscribe();
        let mut wait = limits::REDIAL_FIRST;
        // The connection that ended before this dial, named by it.
        let mut ended = 0;
        loop {
            // A host this machine forgot or paused, or one that dropped the
            // pairing, is not dialed.
            let Some(to) = lock(&self.known)
                .of(&key)
                .filter(|host| host.state.dialed())
                .map(Host::dial_addr)
            else {
                return;
            };
            let over = async {
                match self.dial_replacing(&to, ended).await {
                    Ok(link) => Some(self.hold(link, &to).await),
                    Err(_) => None,
                }
            };
            let lived = tokio::select! {
                held = over => held.map(|(lived, last)| {
                    ended = last;
                    lived
                }),
                _ = stopped.changed() => return,
                _ = shutdown.wait_for(|stop| *stop) => return,
            };
            wait = wait_after(wait, lived);
            tokio::select! {
                () = tokio::time::sleep(wait) => wait = next_wait(wait),
                changed = woke.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    wait = limits::REDIAL_FIRST;
                }
                _ = stopped.changed() => return,
                _ = shutdown.wait_for(|stop| *stop) => return,
            }
        }
    }
}

/// Why a move took no connection.
#[derive(Debug)]
enum NotMoved {
    /// The transport's dial gave none: what it tried counted itself (an SSH
    /// attempt settles its own route).
    NotDialed,
    /// A connection was dialed and proved, and was not installed: it did
    /// not beat the held path by its window, or its first exchange failed.
    NotTaken(DialError),
}

/// A connection a move dialed and has not installed: dropped, it is closed
/// as a probe and kept among the machine's draining connections, so the
/// machine is dialed again only after its end.
struct Unadmitted<T: PeerTransport> {
    inner: Arc<Inner<T>>,
    conn: Option<T::Conn>,
}

impl<T: PeerTransport> Unadmitted<T> {
    fn admitted(mut self) {
        self.conn = None;
    }
}

impl<T: PeerTransport> Drop for Unadmitted<T> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        conn.close(CloseCode::Probe);
        let key = conn.peer();
        lock(&self.inner.table)
            .draining
            .entry(key)
            .or_default()
            .push(conn.clone());
        // Its end takes it off the list even when the machine is never
        // dialed again (it was forgotten while the move ran).
        let inner = Arc::downgrade(&self.inner);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                conn.closed().await;
                if let Some(inner) = inner.upgrade() {
                    inner.undrain(&key, &conn);
                }
            });
        }
    }
}

/// When the side that dialed an SSH connection next looks for a direct path
/// beside it, and how long it waits after one it did not find.
struct Probe {
    at: Instant,
    wait: Duration,
}

impl Probe {
    fn after(since: Instant, first: Duration) -> Self {
        Self {
            at: since + first,
            wait: first,
        }
    }
}

fn next_wait(wait: Duration) -> Duration {
    (wait * 2).min(limits::REDIAL_MOST)
}

/// The wait before the next dial, after a connection that lived `lived`
/// (`None`: the dial failed). Only a connection that stayed starts the wait
/// over: a host that admits and closes at once is dialed as rarely as one
/// that does not answer.
fn wait_after(wait: Duration, lived: Option<Duration>) -> Duration {
    match lived {
        Some(lived) if lived >= limits::STAYED_AFTER => limits::REDIAL_FIRST,
        _ => wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::MachineKey;
    use crate::loopback::{LoopConn, Net};
    use crate::transport::RelayState;
    use hide_peer_protocol::Candidate;

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    fn config(state_dir: &Path) -> Config<'_> {
        Config {
            state_dir,
            build: "1.3.0-test",
            device: "test-device",
            release: false,
            name: None,
            clock: Clock::system(),
            pacing: Pacing::default(),
        }
    }

    /// A transport that cannot accept: its port is taken, or there is no
    /// network. It counts how often it is asked.
    struct Unbound {
        asked: Arc<AtomicUsize>,
    }

    impl PeerTransport for Unbound {
        type Conn = LoopConn;

        fn key(&self) -> PeerKey {
            key(1)
        }

        fn candidates(&self) -> Vec<Candidate> {
            Vec::new()
        }

        async fn dial(&self, _to: &PeerAddr) -> Result<LoopConn, TransportError> {
            Err(TransportError::Unreachable("unbound".into()))
        }

        async fn accept(&self) -> Result<LoopConn, TransportError> {
            self.asked.fetch_add(1, Ordering::AcqRel);
            // Lets the caller's other work run, as a bind that fails does.
            tokio::task::yield_now().await;
            Err(TransportError::Unreachable("the port is taken".into()))
        }

        async fn relay(&self) -> RelayState {
            RelayState::None
        }

        async fn close(&self) {}
    }

    /// Lets every task run until it waits.
    async fn settled() {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_transport_that_cannot_accept_is_asked_again_as_a_machine_is_dialed_again() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let transport = Unbound {
            asked: Arc::clone(&asked),
        };
        let (peers, _incoming) = Peers::start(transport, config(dir.path())).unwrap();
        let asked = || asked.load(Ordering::Acquire);
        let millisecond = Duration::from_millis(1);

        settled().await;
        assert_eq!(asked(), 1, "asked once, and then it waits");
        tokio::time::advance(limits::REDIAL_FIRST - millisecond).await;
        settled().await;
        assert_eq!(asked(), 1);
        tokio::time::advance(millisecond).await;
        settled().await;
        assert_eq!(asked(), 2);

        // The wait doubled.
        tokio::time::advance(limits::REDIAL_FIRST).await;
        settled().await;
        assert_eq!(asked(), 2);
        tokio::time::advance(limits::REDIAL_FIRST).await;
        settled().await;
        assert_eq!(asked(), 3);

        // The network changed: asked at once, and the wait starts over.
        peers.network_changed();
        settled().await;
        assert_eq!(asked(), 4);
        tokio::time::advance(limits::REDIAL_FIRST).await;
        settled().await;
        assert_eq!(asked(), 5);
        peers.close().await;
    }

    #[tokio::test]
    async fn a_machine_that_came_without_a_grant_and_left_leaves_nothing_behind() {
        let net = Net::new();
        let host_dir = tempfile::tempdir().unwrap();
        let (host, _incoming) = Peers::start(
            net.join(MachineKey::generate().unwrap()),
            config(host_dir.path()),
        )
        .unwrap();
        let mut events = host.events();
        let to = PeerAddr::new(host.key(), Vec::new());
        // More of them than may be connected without a grant at once: each
        // one's place is free again when it left.
        for _ in 0..limits::MAX_UNGRANTED + 4 {
            let dir = tempfile::tempdir().unwrap();
            let (guest, _incoming) = Peers::start(
                net.join(MachineKey::generate().unwrap()),
                config(dir.path()),
            )
            .unwrap();
            guest.dial(&to).await.expect("a stranger is admitted");
            guest.close().await;
            let left = guest.key();
            tokio::time::timeout(Duration::from_secs(20), async {
                loop {
                    let event = events.recv().await.expect("the host's events");
                    if matches!(event, PeerEvent::Disconnected { key, .. } if key == left) {
                        return;
                    }
                }
            })
            .await
            .expect("the host saw the stranger leave");
        }
        {
            let table = lock(&host.inner.table);
            assert!(table.live.is_empty());
            assert!(table.draining.is_empty());
        }
        assert!(lock(&host.inner.dialing).is_empty());
        host.close().await;
    }

    #[test]
    fn only_a_connection_that_stayed_starts_the_redial_wait_over() {
        let waited = Duration::from_secs(16);
        // The dial failed.
        assert_eq!(wait_after(waited, None), waited);
        // Admitted and closed at once: dialed as rarely as no answer.
        assert_eq!(wait_after(waited, Some(Duration::from_secs(2))), waited);
        assert_eq!(
            wait_after(waited, Some(limits::STAYED_AFTER)),
            limits::REDIAL_FIRST
        );
    }

    fn table() -> Table<LoopConn> {
        Table {
            live: HashMap::new(),
            draining: HashMap::new(),
            superseded: HashMap::new(),
        }
    }

    /// The names of ended connections are kept to their cap per machine,
    /// the one an install adds last (the one it replaces) staying, so a
    /// late predecessor is still refused past the cap; a name heard again
    /// keeps the time it was first heard.
    #[tokio::test(start_paused = true)]
    async fn ended_names_past_the_cap_still_refuse_the_late_predecessor() {
        let (ours, peer) = (key(1), key(2));
        let mut table = table();
        // More names than a hello carries, then the one replaced.
        let replaced = limits::MAX_SUPERSEDED as u64 + 2;
        for name in 1..=replaced {
            table.supersede(peer, name);
        }
        let kept = table.superseded(&peer);
        assert_eq!(kept.len(), limits::MAX_SUPERSEDED);
        assert_eq!(kept.last(), Some(&replaced));
        let late = Named {
            dialed_by_us: false,
            dial: replaced,
            replaces: 0,
        };
        let held = Named {
            dialed_by_us: false,
            dial: replaced + 1,
            replaces: replaced,
        };
        assert_eq!(
            decide(&ours, &peer, Some(held), late, &[], &kept),
            Standing::Refuse(CloseCode::Replaced)
        );

        // Heard again half way, a name still ends when first heard.
        let mut table = self::table();
        table.supersede(peer, 7);
        tokio::time::advance(limits::SUPERSEDED_FOR / 2).await;
        table.supersede(peer, 7);
        tokio::time::advance(limits::SUPERSEDED_FOR / 2).await;
        assert!(table.superseded(&peer).is_empty());
    }

    /// Ended names are kept for at most so many machines, the one whose
    /// newest name is oldest going first, and a machine whose names aged
    /// is dropped when another's are kept, whether or not it is asked again.
    #[tokio::test(start_paused = true)]
    async fn ended_names_are_kept_for_a_bounded_set_of_machines() {
        let mut table = table();
        let machines: Vec<PeerKey> = (0..=limits::MAX_SUPERSEDED_PEERS)
            .map(|_| MachineKey::generate().unwrap().public())
            .collect();
        for machine in &machines {
            table.supersede(*machine, 1);
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        assert_eq!(table.superseded.len(), limits::MAX_SUPERSEDED_PEERS);
        assert!(!table.superseded.contains_key(&machines[0]));
        assert!(table.superseded.contains_key(machines.last().unwrap()));

        tokio::time::advance(limits::SUPERSEDED_FOR).await;
        table.supersede(machines[0], 2);
        assert_eq!(table.superseded.len(), 1);
    }

    fn named(dialed_by_us: bool) -> Named {
        Named {
            dialed_by_us,
            dial: 0,
            replaces: 0,
        }
    }

    #[test]
    fn exactly_one_of_two_connections_stays_and_both_sides_agree() {
        let (small, large) = (key(1), key(2));
        let none: &[u64] = &[];
        // The first connection to a machine.
        assert_eq!(
            decide(&small, &large, None, named(true), none, none),
            Standing::Install
        );

        // Both dialed and both established: the smaller key's dial stays,
        // whichever arrived second, seen from either side.
        assert_eq!(
            decide(&small, &large, Some(named(false)), named(true), none, none),
            Standing::Replace(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&small, &large, Some(named(true)), named(false), none, none),
            Standing::Refuse(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&large, &small, Some(named(true)), named(false), none, none),
            Standing::Replace(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&large, &small, Some(named(false)), named(true), none, none),
            Standing::Refuse(CloseCode::Duplicate)
        );

        // The same side dialed again: the new one replaces the old one and
        // is never closed as a duplicate.
        for ours in [small, large] {
            let peer = if ours == small { large } else { small };
            for by_us in [true, false] {
                assert_eq!(
                    decide(&ours, &peer, Some(named(by_us)), named(by_us), none, none),
                    Standing::Replace(CloseCode::Replaced)
                );
            }
        }
        exactly_one_stays_whatever_the_order();
    }

    /// How a connection of the model below came to be dialed, as the dial
    /// rule allows: a first dial at the start (both sides may), a redial by
    /// a side whose own connection ended there, or a move by a side while it
    /// held the connection the move names.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Made {
        First,
        Redial,
        Move(usize),
    }

    #[derive(Clone, Copy, Debug)]
    struct Modeled {
        /// 0 or 1: which end dialed it.
        dialer: usize,
        dial: u64,
        made: Made,
    }

    /// The name a connection's hello gives as the one it replaces: a
    /// move's, the one it moves; a redial's, the one that ended on its
    /// dialer's end before it (`Pair::redialed`).
    fn replaces(conns: &[Modeled], pair: &Pair, conn: usize) -> u64 {
        match conns[conn].made {
            Made::Move(of) => conns[of].dial,
            Made::Redial => pair.redialed[conn],
            Made::First => 0,
        }
    }

    /// Both ends of a pair, each with its table and its list of what a move
    /// replaced, and where each connection stands.
    #[derive(Clone, Debug, Default)]
    struct Pair {
        held: [Option<usize>; 2],
        superseded: [Vec<u64>; 2],
        launched: Vec<bool>,
        /// What each redial names as replaced.
        redialed: Vec<u64>,
        /// What each connection's dialer knew was replaced when it dialed.
        supersedes: Vec<Vec<u64>>,
        /// What happened, in order, for a failure to say.
        trace: Vec<String>,
        arrived: Vec<[bool; 2]>,
        closed: Vec<bool>,
        seen: Vec<[bool; 2]>,
    }

    fn close(pair: &mut Pair, conn: usize, by: usize) {
        if !pair.closed[conn] {
            pair.closed[conn] = true;
            pair.seen[conn][by] = true;
        }
        if pair.held[by] == Some(conn) {
            pair.held[by] = None;
        }
    }

    fn see_close(pair: &mut Pair, conn: usize, end: usize) {
        pair.seen[conn][end] = true;
        if pair.held[end] == Some(conn) {
            pair.held[end] = None;
        }
    }

    fn launch(pair: &mut Pair, conns: &[Modeled], conn: usize) {
        let dialer = conns[conn].dialer;
        // A side redials only once its own connection ended there, and
        // names it.
        if conns[conn].made == Made::Redial
            && let Some(held) = pair.held[dialer]
        {
            pair.redialed[conn] = conns[held].dial;
            close(pair, held, dialer);
        }
        pair.supersedes[conn] = pair.superseded[dialer].clone();
        pair.launched[conn] = true;
    }

    fn may_launch(pair: &Pair, conns: &[Modeled], conn: usize) -> bool {
        let dialer = conns[conn].dialer;
        // A redial and a move take the one dial turn a machine has: every
        // earlier dial of this side finished its first exchange here.
        let turn = || {
            (0..conn)
                .filter(|earlier| conns[*earlier].dialer == dialer)
                .all(|earlier| pair.arrived[earlier][dialer])
        };
        !pair.launched[conn]
            && match conns[conn].made {
                // A dial never starts beside a live connection.
                Made::First => turn() && pair.held[dialer].is_none(),
                Made::Redial => turn(),
                Made::Move(of) => turn() && pair.held[dialer] == Some(of),
            }
    }

    fn arrive(pair: &mut Pair, conns: &[Modeled], keys: [PeerKey; 2], conn: usize, end: usize) {
        pair.arrived[conn][end] = true;
        // A connection this end saw closed never finishes its first
        // exchange here.
        if pair.seen[conn][end] {
            return;
        }
        let view = |at: usize| Named {
            dialed_by_us: conns[at].dialer == end,
            dial: conns[at].dial,
            replaces: replaces(conns, pair, at),
        };
        let held = pair.held[end];
        let standing = decide(
            &keys[end],
            &keys[1 - end],
            held.map(view),
            view(conn),
            &pair.supersedes[conn],
            &pair.superseded[end],
        );
        match standing {
            Standing::Refuse(_) => {
                pair.superseded[end].push(conns[conn].dial);
                close(pair, conn, end);
            }
            Standing::Install | Standing::Replace(_) => {
                let named = std::iter::once(replaces(conns, pair, conn))
                    .chain(pair.supersedes[conn].clone())
                    .filter(|name| *name != 0)
                    .collect::<Vec<_>>();
                pair.superseded[end].extend(named);
                if let Some(held) = held {
                    pair.superseded[end].push(conns[held].dial);
                    close(pair, held, end);
                }
                pair.held[end] = Some(conn);
            }
        }
    }

    /// Runs every order the events of `conns` can take, and answers, per
    /// order in which every connection was dialed, the one the pair kept.
    fn every_order(conns: &[Modeled], keys: [PeerKey; 2], pair: &Pair, left: &mut Vec<usize>) {
        let mut moved = false;
        let mut next = |what: String, change: &dyn Fn(&mut Pair)| {
            moved = true;
            let mut next = pair.clone();
            change(&mut next);
            next.trace.push(format!("{what} -> held {:?}", next.held));
            every_order(conns, keys, &next, left);
        };
        for conn in 0..conns.len() {
            if may_launch(pair, conns, conn) {
                next(format!("dial {conn}"), &|pair| launch(pair, conns, conn));
            }
            for end in 0..2 {
                if pair.launched[conn] && !pair.arrived[conn][end] {
                    next(format!("{conn} at {end}"), &|pair| {
                        arrive(pair, conns, keys, conn, end)
                    });
                }
                if pair.closed[conn] && !pair.seen[conn][end] {
                    next(format!("{end} sees {conn} closed"), &|pair| {
                        see_close(pair, conn, end)
                    });
                }
            }
        }
        if !moved {
            assert_eq!(
                pair.held[0], pair.held[1],
                "the two ends hold different connections: {conns:?} {pair:?}"
            );
            let kept =
                pair.held[0].unwrap_or_else(|| panic!("no connection left: {conns:?} {pair:?}"));
            if pair.launched.iter().all(|launched| *launched) {
                left.push(kept);
            }
        }
    }

    /// Every set of connections from a first dial, a fresh redial, a move
    /// naming the held connection and a move naming an older one, from
    /// either side, under either key order and in every order their first
    /// exchanges finish and their closes are seen on the two ends, leaves
    /// one connection, the same on both ends.
    fn exactly_one_stays_whatever_the_order() {
        let conn = |dialer, dial, made| Modeled { dialer, dial, made };
        let mut scenarios: Vec<Vec<Modeled>> = Vec::new();
        for first in 0..2 {
            let other = 1 - first;
            // Both sides dialed at once.
            scenarios.push(vec![
                conn(first, 1, Made::First),
                conn(other, 2, Made::First),
            ]);
            for side in 0..2 {
                // A redial, and a move naming the held connection.
                scenarios.push(vec![
                    conn(first, 1, Made::First),
                    conn(side, 2, Made::Redial),
                ]);
                scenarios.push(vec![
                    conn(first, 1, Made::First),
                    conn(side, 2, Made::Move(0)),
                ]);
                // A move naming an older connection than the one kept.
                scenarios.push(vec![
                    conn(first, 1, Made::First),
                    conn(other, 2, Made::First),
                    conn(side, 3, Made::Move(0)),
                ]);
                // A move, and a redial against it.
                for redial in 0..2 {
                    scenarios.push(vec![
                        conn(first, 1, Made::First),
                        conn(side, 2, Made::Move(0)),
                        conn(redial, 3, Made::Redial),
                    ]);
                }
                // A move of the held connection from each side at once.
                scenarios.push(vec![
                    conn(first, 1, Made::First),
                    conn(side, 2, Made::Move(0)),
                    conn(1 - side, 3, Made::Move(0)),
                ]);
                // A chain: a move, then a move of the move from either side.
                for then in 0..2 {
                    scenarios.push(vec![
                        conn(first, 1, Made::First),
                        conn(side, 2, Made::Move(0)),
                        conn(then, 3, Made::Move(1)),
                    ]);
                }
            }
        }
        for conns in &scenarios {
            let count = conns.len();
            let mut dialed_all = false;
            for keys in [[key(1), key(2)], [key(2), key(1)]] {
                let pair = Pair {
                    launched: vec![false; count],
                    redialed: vec![0; count],
                    supersedes: vec![Vec::new(); count],
                    arrived: vec![[false; 2]; count],
                    closed: vec![false; count],
                    seen: vec![[false; 2]; count],
                    ..Pair::default()
                };
                let mut left = Vec::new();
                every_order(conns, keys, &pair, &mut left);
                dialed_all |= !left.is_empty();
                // A move of the connection both ends kept, or a move of
                // that move, is what is left, however the exchanges went.
                let kept_moved = matches!(
                    conns.iter().map(|conn| conn.made).collect::<Vec<_>>()[..],
                    [Made::First, Made::Move(0)] | [Made::First, Made::Move(0), Made::Move(1)]
                );
                if kept_moved {
                    assert!(
                        left.iter().all(|kept| *kept == count - 1),
                        "{conns:?} under {keys:?} left {left:?}"
                    );
                }
            }
            assert!(dialed_all, "{conns:?}: no order dialed them all");
        }
    }

    #[test]
    fn the_redial_wait_doubles_to_its_most() {
        let mut wait = limits::REDIAL_FIRST;
        let mut waits = Vec::new();
        for _ in 0..8 {
            waits.push(wait.as_secs());
            wait = next_wait(wait);
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60]);
    }
}
