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
//!   would leave the pair with a dead connection.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hide_kit::layout;
use hide_peer_protocol::hello::{self, Agreed, Behind, GrantState, Hello, HelloFrame};
use hide_peer_protocol::pairing::ConfirmState;
use hide_peer_protocol::stream::{self, Status};
use hide_peer_protocol::{CloseCode, PathKind, PeerAddr, PeerKey, StreamKind};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Semaphore, broadcast, mpsc, watch};

use crate::gate::{CLOSED, Gate, Guarded};
use crate::limits;
use crate::store::{Change, Grant, Grants, Known, KnownState, StoreError, Via};
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

/// Decides between the connection held to `peer` and a new one. Both sides
/// decide the same from the same facts, so exactly one connection is left.
pub fn decide(
    ours: &PeerKey,
    peer: &PeerKey,
    held_dialed_by_us: Option<bool>,
    new_dialed_by_us: bool,
) -> Standing {
    let Some(held_dialed_by_us) = held_dialed_by_us else {
        return Standing::Install;
    };
    if held_dialed_by_us == new_dialed_by_us {
        // The same side dialed again: its earlier connection is gone on its
        // end, whatever this end still believes.
        return Standing::Replace(CloseCode::Replaced);
    }
    let dialer = |by_us: bool| if by_us { ours } else { peer };
    if dialer(new_dialed_by_us) < dialer(held_dialed_by_us) {
        Standing::Replace(CloseCode::Duplicate)
    } else {
        Standing::Refuse(CloseCode::Duplicate)
    }
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

impl Counts {
    fn take(self: &Arc<Self>, kind: StreamKind) -> Option<Place> {
        let count = &self.0[kind.tag() as usize - 1];
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
        self.counts.0[self.kind.tag() as usize - 1].fetch_sub(1, Ordering::AcqRel);
    }
}

struct LinkInner<C: PeerConn> {
    conn: C,
    theirs: Hello,
    agreed: Agreed,
    /// The grant this machine gives the peer: gates what the peer opens.
    inbound: Arc<Gate>,
    /// The grant the peer gives this machine: gates what is opened there.
    outbound: Arc<Gate>,
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
}

struct Inner<T: PeerTransport> {
    transport: T,
    hello: Hello,
    /// The host's lock: every change of a grant is saved under it first.
    grants: Mutex<Grants>,
    known: Mutex<Known>,
    gates: Mutex<HashMap<PeerKey, Arc<Gate>>>,
    table: Mutex<Table<T::Conn>>,
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
    shutdown: watch::Sender<bool>,
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

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // A panic under one of these locks left a store that was saved first
    // and a table of plain values: both are still what they say.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl<T: PeerTransport> Peers<T> {
    /// Opens the two records and starts accepting. Call it on the runtime
    /// the connections run on. The receiver yields every stream a peer
    /// opens and this machine accepts; [`Peers::close`] ends everything
    /// started here.
    pub fn start(
        transport: T,
        config: Config<'_>,
    ) -> io::Result<(Self, mpsc::Receiver<Incoming<T::Conn>>)> {
        let mut grants = Grants::open(layout::peer_grants(config.state_dir))?;
        grants.expire(now_unix_ms()).map_err(io::Error::other)?;
        let known = Known::open(layout::peer_known(config.state_dir))?;
        let (incoming, accepted) = mpsc::channel(limits::MAX_STREAMS);
        let inner = Arc::new(Inner {
            transport,
            hello: Hello::ours(config.build, config.device, config.release),
            grants: Mutex::new(grants),
            known: Mutex::new(known),
            gates: Mutex::new(HashMap::new()),
            table: Mutex::new(Table {
                live: HashMap::new(),
                draining: HashMap::new(),
            }),
            admitted: watch::channel(0).0,
            dialing: Mutex::new(HashMap::new()),
            admitting: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            events: broadcast::channel(limits::MAX_EVENTS).0,
            incoming,
            woke: watch::channel(0).0,
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
        self.inner.woke.send_modify(|count| *count += 1);
    }

    /// Reads the host's record.
    pub fn grants<R>(&self, read: impl FnOnce(&Grants) -> R) -> R {
        read(&lock(&self.inner.grants))
    }

    /// Reads or changes the guest's record.
    pub fn known<R>(&self, change: impl FnOnce(&mut Known) -> R) -> R {
        change(&mut lock(&self.inner.known))
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
        lock(&self.inner.grants).grant_pending(key, device, name, via, now_unix_ms())
    }

    /// The guest's confirm of grant `id`.
    pub fn confirm(&self, key: &PeerKey, id: &str) -> Result<ConfirmState, StoreError> {
        let state = {
            let mut grants = lock(&self.inner.grants);
            let (state, change) = grants.confirm(key, id, now_unix_ms())?;
            if let Some(change) = change {
                self.inner.move_gate(change);
            }
            state
        };
        self.inner.settle(key);
        Ok(state)
    }

    pub fn pause(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.change(key, |grants| grants.pause(key).map(Some))
    }

    pub fn resume(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.change(key, |grants| grants.resume(key).map(Some))
    }

    /// Removes the grant given to `key`; with `id`, only that grant.
    /// Removing what is not there is not an error.
    pub fn revoke(&self, key: &PeerKey, id: Option<&str>) -> Result<(), StoreError> {
        self.change(key, |grants| grants.revoke(key, id, now_unix_ms()))
    }

    /// Forgets a host this machine was a guest of. Whether it was known.
    pub fn forget(&self, key: &PeerKey) -> Result<bool, StoreError> {
        let removed = lock(&self.inner.known).remove(key)?;
        self.inner.settle(key);
        Ok(removed)
    }

    /// Saves a change of the grant given to `key`, then moves that
    /// direction's generation, then tells the peer. The save is first: a
    /// failed one moves nothing.
    fn change(
        &self,
        key: &PeerKey,
        save: impl FnOnce(&mut Grants) -> Result<Option<Change>, StoreError>,
    ) -> Result<(), StoreError> {
        {
            let mut grants = lock(&self.inner.grants);
            if let Some(change) = save(&mut grants)? {
                // Under the lock that saved it, so two changes move the
                // gate in the order they were saved.
                self.inner.move_gate(change);
            }
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

    /// Whether this machine and `key` are paired in either direction, a
    /// pending grant included.
    fn related(&self, key: &PeerKey) -> bool {
        lock(&self.grants)
            .list()
            .iter()
            .any(|grant| grant.key == *key)
            || lock(&self.known).of(key).is_some()
    }

    fn gate(&self, key: &PeerKey, generation: impl FnOnce() -> u64) -> Arc<Gate> {
        Arc::clone(
            lock(&self.gates)
                .entry(*key)
                .or_insert_with(|| Gate::new(generation())),
        )
    }

    /// Moves the generation of the grant given to `change.key`: every
    /// stream that peer opened under the earlier one ends.
    fn move_gate(&self, change: Change) {
        self.gate(&change.key, || change.generation)
            .set(change.generation);
    }

    /// After a record changed: tells the peer the grant it now has, and
    /// closes the connection when no grant is left in either direction.
    fn settle(&self, key: &PeerKey) {
        let Some(link) = self.link(key) else {
            return;
        };
        let change = lock(&self.grants).state_of(key);
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
        loop {
            let conn = tokio::select! {
                _ = shutdown.wait_for(|stop| *stop) => return,
                conn = self.transport.accept() => conn,
            };
            match conn {
                Ok(conn) => {
                    // At the cap a connection is closed before any of its
                    // bytes are read.
                    let Ok(permit) = Arc::clone(&self.admitting).try_acquire_owned() else {
                        conn.close(CloseCode::NoGrant);
                        continue;
                    };
                    let inner = Arc::clone(&self);
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Err(error) = inner.establish(conn).await {
                            crate::diagnostic!(json!({
                                "component": "peer",
                                "event": "connection.not_admitted",
                                "reason": error.to_string(),
                            }));
                        }
                    });
                }
                Err(TransportError::Closed) => return,
                Err(error) => crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "accept.failed",
                    "reason": error.to_string(),
                })),
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
            let mut table = lock(&self.table);
            if let Some(conns) = table.draining.get_mut(key) {
                conns.retain(|draining| draining.id() != conn.id());
                if conns.is_empty() {
                    table.draining.remove(key);
                }
            }
        }
    }

    async fn dial(self: &Arc<Self>, to: &PeerAddr) -> Result<Link<T::Conn>, DialError> {
        let turn = Arc::clone(lock(&self.dialing).entry(to.key).or_default());
        let dialed = {
            let _turn = turn.lock().await;
            self.dial_in_turn(to).await
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

    async fn dial_in_turn(self: &Arc<Self>, to: &PeerAddr) -> Result<Link<T::Conn>, DialError> {
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
        let failed = match self.establish(conn.clone()).await {
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
    ) -> Result<
        (
            Hello,
            <T::Conn as PeerConn>::Send,
            <T::Conn as PeerConn>::Recv,
        ),
        DialError,
    > {
        let ours = self
            .hello
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
        let key = conn.peer();
        let exchanged =
            tokio::time::timeout(limits::HELLO_WITHIN, self.first_exchange(&conn)).await;
        let (theirs, send, recv) = match exchanged {
            Ok(Ok(exchanged)) => exchanged,
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
                // never pushes out a connection that works.
                self.emit(PeerEvent::UpdateNeeded {
                    key,
                    device: theirs.device.clone(),
                    build: theirs.build.clone(),
                    behind,
                });
                conn.close(CloseCode::UpdateNeeded);
                return Err(DialError::UpdateNeeded(behind));
            }
        };
        let (frames, queued) = mpsc::channel(limits::MAX_EVENTS);
        let counts = Arc::new(Counts::default());
        // The hello stream holds its kind's one place for the connection's
        // life, so a second one is refused at the limit.
        let hello_place = counts.take(StreamKind::Hello);
        let related = self.related(&key);
        let link = {
            let mut table = lock(&self.table);
            let held = table
                .live
                .get(&key)
                .filter(|link| link.inner.conn.ended().is_none())
                .map(Link::dialed);
            let standing = decide(&self.transport.key(), &key, held, conn.dialed());
            if let Standing::Refuse(code) = standing {
                drop(table);
                conn.close(code);
                return Err(DialError::Refused(code));
            }
            if !related {
                let ungranted = table
                    .live
                    .keys()
                    .filter(|other| **other != key && !self.related(other))
                    .count();
                if ungranted >= limits::MAX_UNGRANTED {
                    drop(table);
                    conn.close(CloseCode::NoGrant);
                    return Err(DialError::Refused(CloseCode::NoGrant));
                }
            }
            let granting = lock(&self.grants).state_of(&key);
            let link = Link {
                inner: Arc::new(LinkInner {
                    conn: conn.clone(),
                    theirs,
                    agreed,
                    inbound: self.gate(&key, || granting.generation),
                    outbound: Gate::new(CLOSED),
                    granted: AtomicU8::new(state_tag(GrantState::None)),
                    counts,
                    headers: Arc::new(Semaphore::new(limits::MAX_STREAMS)),
                    frames,
                }),
            };
            if let Some(old) = table.live.insert(key, link.clone())
                && let Standing::Replace(code) = standing
            {
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
        self.admitted.send_modify(|count| *count += 1);
        tokio::spawn(Arc::clone(self).write_hello_frames(link.clone(), send, queued, hello_place));
        tokio::spawn(Arc::clone(self).read_hello_frames(link.clone(), recv));
        tokio::spawn(Arc::clone(self).accept_streams(link.clone()));
        tokio::spawn(Arc::clone(self).watch_end(link.clone(), related));
        self.emit(PeerEvent::Connected {
            key,
            path: link.path(),
        });
        Ok(link)
    }

    async fn write_hello_frames(
        self: Arc<Self>,
        link: Link<T::Conn>,
        mut send: <T::Conn as PeerConn>::Send,
        mut queued: mpsc::Receiver<HelloFrame>,
        _place: Option<Place>,
    ) {
        loop {
            let frame = tokio::select! {
                frame = queued.recv() => frame,
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
        loop {
            let body = match wire::read_message(&mut recv, hello::MAX_HELLO).await {
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
                    link.inner.outbound.set(generation);
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

    /// Ends a connection's place in the table when it ends, and closes one
    /// that stays without a grant in either direction for too long.
    async fn watch_end(self: Arc<Self>, link: Link<T::Conn>, related: bool) {
        let key = link.key();
        let code = if related {
            link.closed().await
        } else {
            tokio::select! {
                code = link.closed() => code,
                () = tokio::time::sleep(limits::UNGRANTED_FOR) => {
                    if !self.related(&key) {
                        link.close(CloseCode::NoGrant);
                    }
                    link.closed().await
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

    /// The guest's redial: one connection at a time, a short wait after a
    /// drop, a longer one after each failure.
    async fn keep(self: Arc<Self>, key: PeerKey, mut stopped: watch::Receiver<bool>) {
        let mut shutdown = self.shutdown.subscribe();
        let mut woke = self.woke.subscribe();
        let mut wait = limits::REDIAL_FIRST;
        loop {
            // A host this machine forgot or paused is not dialed.
            let Some(to) = lock(&self.known)
                .of(&key)
                .filter(|host| host.state != KnownState::PausedByMe)
                .map(|host| PeerAddr::new(key, host.candidates.clone()))
            else {
                return;
            };
            let over = async {
                match self.dial(&to).await {
                    Ok(link) => {
                        link.closed().await;
                        true
                    }
                    Err(_) => false,
                }
            };
            let connected = tokio::select! {
                connected = over => connected,
                _ = stopped.changed() => return,
                _ = shutdown.wait_for(|stop| *stop) => return,
            };
            if connected {
                wait = limits::REDIAL_FIRST;
            }
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

fn next_wait(wait: Duration) -> Duration {
    (wait * 2).min(limits::REDIAL_MOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    #[test]
    fn exactly_one_of_two_connections_stays_and_both_sides_agree() {
        let (small, large) = (key(1), key(2));
        // The first connection to a machine.
        assert_eq!(decide(&small, &large, None, true), Standing::Install);

        // Both dialed and both established: the smaller key's dial stays,
        // whichever arrived second, seen from either side.
        assert_eq!(
            decide(&small, &large, Some(false), true),
            Standing::Replace(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&small, &large, Some(true), false),
            Standing::Refuse(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&large, &small, Some(true), false),
            Standing::Replace(CloseCode::Duplicate)
        );
        assert_eq!(
            decide(&large, &small, Some(false), true),
            Standing::Refuse(CloseCode::Duplicate)
        );

        // The same side dialed again: the new one replaces the old one and
        // is never closed as a duplicate.
        for ours in [small, large] {
            let peer = if ours == small { large } else { small };
            for by_us in [true, false] {
                assert_eq!(
                    decide(&ours, &peer, Some(by_us), by_us),
                    Standing::Replace(CloseCode::Replaced)
                );
            }
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
