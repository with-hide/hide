//! The requests of a browser page drawn in another machine's screen leave
//! through that machine (docs/BROWSER_DISPLAYS.md, A page in another
//! computer's screen; security spec 8.3).
//!
//! A guest draws the host's browser displays in its own desktop app, in a
//! session whose proxy is a listener of this daemon on its own loopback
//! ([`ScreenNet::ensure`]). Each connection Chromium makes to it carries one
//! `page` stream of the one connection between the two machines, opened
//! under the grant's generation as the screen's streams are, and the host's
//! half ([`ScreenNet::serve`]) answers that stream as an HTTP/1.1 forward
//! proxy that dials on its own network: its loopback, its LAN, its resolver.
//! Nothing of a page's request reaches this machine's network.
//!
//! The guest's listener is kept for its owner, the desktop process that
//! asked, while that process lives and the machine stays paired under the
//! key, so a page loaded before a sleep or a drop loads its next request
//! after it with no reload: a drop, a pause or a moved generation ends
//! tunnels, never the listener. Its connections need its secret in
//! `Proxy-Authorization`, so another account of this machine cannot use the
//! host's network through it. The host's places are the `page` stream's cap,
//! shared by a pair's listeners; a quiet tunnel gives its place to a new one.
//! On the relay a pair's page bytes are held to a daily budget, reserved
//! before each chunk is written.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use hide_peer::protocol::{PathKind, PeerKey, StreamKind};
use http_body_util::{BodyExt, Empty, combinators::BoxBody};
use hyper::body::{Bytes, Incoming as Body};
use hyper::{Method, Request, Response, StatusCode, header};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::owned_task::AbortOnDrop;
use crate::peer_host::{Incoming, Link, PeerHost};

/// Listeners one daemon holds at once; one more ask is refused, never
/// answered by closing a listener a page still points at.
pub const MAX_LISTENERS: usize = 8;
/// Tunnels a pair carries at once: the `page` stream's cap on the host.
pub const PLACES: usize = hide_peer::limits::max_streams_of(StreamKind::Page);
/// Connections waiting for a place, and how long each waits.
const MAX_WAITING: usize = 64;
const PLACE_WITHIN: Duration = Duration::from_secs(30);
/// A tunnel that moved no byte either way for this long gives its place to
/// a new one when every place is held.
const QUIET_FOR: Duration = Duration::from_secs(60);
/// The first request head of a connection: its bound and its wait, and how
/// many connections may be reading theirs at once before their secret is
/// known, so a process that is no page cannot hold the pair's places.
const MAX_HEAD: usize = 16 * 1024;
const HEAD_WITHIN: Duration = Duration::from_secs(10);
const MAX_UNPROVEN: usize = 64;
/// The longest one address of a name may take to answer a dial, so a dead
/// address family leaves time for the next.
const DIAL_EACH: Duration = Duration::from_secs(3);
/// How long an accept that failed waits before the next, and how long a
/// stream the host refused for its count waits before asking again.
const ACCEPT_AGAIN: Duration = Duration::from_millis(100);
const OPEN_AGAIN: Duration = Duration::from_millis(50);
/// Pairs whose relay bytes of the day are kept; past it, a new pair counts
/// with the others in one shared record, so no allowance is ever reset.
const MAX_BUDGETS: usize = 256;
/// The most bytes one copy writes at once, each reserved first on the relay.
pub const CHUNK: usize = 64 * 1024;
/// How long the host's dial and the guest's stream open may take.
const DIAL_WITHIN: Duration = Duration::from_secs(10);
/// How often a listener checks that its owner lives and the machine is
/// still paired under its key, beside each connection it accepts.
const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Page bytes a pair may carry over the relay in one UTC day.
pub const RELAY_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How the guest reaches a paired host: the link while this machine may
/// open it under the key, or why not, in the words `/peer-screen` answers.
pub trait Reach: Send + Sync + 'static {
    fn link(&self, device: &str, key: &PeerKey) -> Result<Link, &'static str>;
    /// The pairing this machine holds with `device` under `key`
    /// ([`pairing_of`] its grant), whatever its state; none when it holds
    /// none.
    fn pairing(&self, device: &str, key: &PeerKey) -> Option<String>;
}

impl Reach for Arc<PeerHost> {
    fn link(&self, device: &str, key: &PeerKey) -> Result<Link, &'static str> {
        crate::remote_screen::host_link_on(self, device, key).map(|(link, _)| link)
    }

    fn pairing(&self, device: &str, key: &PeerKey) -> Option<String> {
        let host = self
            .peers()?
            .hosts()
            .into_iter()
            .find(|host| host.device == device && host.key == *key)?;
        Some(pairing_of(&host.grant))
    }
}

/// The name a pairing is told by outside this daemon: the first half of the
/// SHA-256 of the grant the host gave, in hex. A pause keeps the grant, and
/// pairing again makes a new one, so the desktop's login storage of a
/// machine, named with it, is never one an ended pairing filled
/// (docs/BROWSER_DISPLAYS.md, A page in another computer's screen).
pub fn pairing_of(grant: &str) -> String {
    hex::encode(&ring::digest::digest(&ring::digest::SHA256, grant.as_bytes()).as_ref()[..16])
}

/// Whether a process started at `started` is still `pid`.
fn owner_lives(pid: i32, started: u64) -> bool {
    hide_node::pane_proof::process_start(pid) == Some(started)
}

/// A listener's one owner: a desktop process seeing one machine under one
/// key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Owner {
    device: String,
    key: PeerKey,
    pid: i32,
}

struct Listener {
    owner: Owner,
    started: u64,
    port: u16,
    secret: String,
    /// The connection (link id) on which the host closed a `page` stream
    /// before any byte came back: a build with no handler for the kind
    /// (security spec 7). It names that connection alone, so a host updated
    /// and connected again is asked afresh, and the first byte back on any
    /// tunnel clears it.
    unsupported: Mutex<Option<u64>>,
    /// Each refusal's reason, and each kind of accept failure, logged once.
    refused: Mutex<HashSet<String>>,
    /// Connections reading their head before their secret is known.
    unproven: Arc<tokio::sync::Semaphore>,
    task: Mutex<Option<AbortOnDrop>>,
    /// Accept failures a test makes the next accepts report.
    #[cfg(test)]
    fail_accepts: std::sync::atomic::AtomicUsize,
}

impl Listener {
    fn refused(&self, reason: &str) {
        if lock(&self.refused).insert(reason.to_owned()) {
            hide_core::diagnostic!(json!({
                "component": "screen_net", "kind": "screen_net.refused",
                "device": self.owner.device, "reason": reason,
            }));
        }
    }
}

/// What the desktop is told of its listener.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Ensured {
    pub port: u16,
    pub secret: String,
    /// The pairing the listener's requests ride ([`pairing_of`]).
    pub pairing: String,
    /// `ready`, or why a request through it fails now.
    pub state: &'static str,
}

/// One tunnel's place among its pair's: when it last moved a byte, and how
/// it is told to give the place up.
struct Slot {
    last: Mutex<Instant>,
    reclaim: Notify,
}

impl Slot {
    fn touch(&self) {
        *lock(&self.last) = Instant::now();
    }
}

#[derive(Default)]
struct PlacesHeld {
    held: Vec<Arc<Slot>>,
    waiting: usize,
}

/// A pair's places for tunnels.
#[derive(Default)]
struct Places {
    inner: Mutex<PlacesHeld>,
    freed: Notify,
}

/// A tunnel's place, given back when dropped.
struct Place {
    places: Arc<Places>,
    slot: Arc<Slot>,
}

impl Drop for Place {
    fn drop(&mut self) {
        let mut inner = lock(&self.places.inner);
        let before = inner.held.len();
        inner.held.retain(|held| !Arc::ptr_eq(held, &self.slot));
        if inner.held.len() != before {
            self.places.freed.notify_one();
        }
    }
}

/// A connection counted among its pair's waiting ones, given back however
/// its wait ends: a place, the bound, or its task dropped.
struct Waiting<'a> {
    places: &'a Places,
    counted: bool,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if self.counted {
            lock(&self.places.inner).waiting -= 1;
        }
    }
}

impl Places {
    /// A place: a free one, else the quietest tunnel's when it moved nothing
    /// for `QUIET_FOR`, else the first freed within `PLACE_WITHIN` with at
    /// most `MAX_WAITING` waiting; `None` past either.
    async fn take(self: &Arc<Self>) -> Option<Place> {
        let deadline = Instant::now() + PLACE_WITHIN;
        let mut waiting = Waiting {
            places: self,
            counted: false,
        };
        loop {
            {
                let mut inner = lock(&self.inner);
                let now = Instant::now();
                let quietest = inner
                    .held
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, slot)| *lock(&slot.last))
                    .filter(|(_, slot)| now.duration_since(*lock(&slot.last)) >= QUIET_FOR)
                    .map(|(at, _)| at);
                if inner.held.len() >= PLACES
                    && let Some(at) = quietest
                {
                    let quiet = inner.held.remove(at);
                    quiet.reclaim.notify_one();
                }
                if inner.held.len() < PLACES {
                    if waiting.counted {
                        inner.waiting -= 1;
                        waiting.counted = false;
                    }
                    let slot = Arc::new(Slot {
                        last: Mutex::new(now),
                        reclaim: Notify::new(),
                    });
                    inner.held.push(Arc::clone(&slot));
                    return Some(Place {
                        places: Arc::clone(self),
                        slot,
                    });
                }
                if !waiting.counted {
                    if inner.waiting >= MAX_WAITING {
                        return None;
                    }
                    inner.waiting += 1;
                    waiting.counted = true;
                }
            }
            if tokio::time::timeout_at(deadline, self.freed.notified())
                .await
                .is_err()
            {
                return None;
            }
        }
    }
}

/// A pair's page bytes on the relay in one UTC day: every chunk a
/// tunnel reads while its connection takes the relay is reserved here
/// first, so what the guest reads and sends never passes the budget. Once a
/// reservation fails, every relay tunnel of the pair ends at its next one
/// until the day turns.
pub struct RelayBudget {
    limit: u64,
    day: Mutex<(u64, u64, bool)>,
    today: fn() -> u64,
}

fn utc_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 86_400)
}

impl RelayBudget {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            day: Mutex::new((utc_day(), 0, false)),
            today: utc_day,
        }
    }

    fn turned(&self) -> MutexGuard<'_, (u64, u64, bool)> {
        let today = (self.today)();
        let mut day = lock(&self.day);
        if day.0 != today {
            *day = (today, 0, false);
        }
        day
    }

    /// Reserves `bytes`, or says the day's budget is spent.
    pub fn reserve(&self, bytes: u64) -> bool {
        let mut day = self.turned();
        if day.2 || day.1.saturating_add(bytes) > self.limit {
            day.2 = true;
            return false;
        }
        day.1 += bytes;
        true
    }

    /// Holds up to `most` bytes for a read, as many as the day has left;
    /// none, and the day spent, when it has none. What the read does not
    /// use goes back when the hold goes ([`Held::keep`]).
    fn hold(&self, most: u64) -> Option<Held<'_>> {
        let mut day = self.turned();
        let left = self.limit.saturating_sub(day.1).min(most);
        if day.2 || left == 0 {
            day.2 = true;
            return None;
        }
        day.1 += left;
        Some(Held {
            budget: self,
            day: day.0,
            bytes: left,
            kept: 0,
        })
    }

    pub fn spent(&self) -> bool {
        self.turned().2
    }

    pub fn used(&self) -> u64 {
        self.turned().1
    }
}

/// Bytes of the day's budget held for one read on the relay. The part the
/// read did not use goes back when it goes, so a read that is cancelled
/// (its tunnel ended, its place reclaimed) keeps nothing; a hold of a day
/// that has turned gives nothing back to the new one.
struct Held<'a> {
    budget: &'a RelayBudget,
    day: u64,
    bytes: u64,
    kept: u64,
}

impl Held<'_> {
    /// How many bytes the read may take.
    fn room(&self) -> usize {
        usize::try_from(self.bytes).unwrap_or(usize::MAX)
    }

    /// The read took `bytes`; they stay counted.
    fn keep(&mut self, bytes: u64) {
        self.kept = bytes.min(self.bytes);
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let mut day = lock(&self.budget.day);
        if day.0 == self.day {
            day.1 = day.1.saturating_sub(self.bytes - self.kept);
        }
    }
}

/// Both halves: the guest's listeners and the host's proxy.
pub struct ScreenNet {
    reach: Box<dyn Reach>,
    listeners: Mutex<HashMap<Owner, Arc<Listener>>>,
    places: Mutex<HashMap<PeerKey, Arc<Places>>>,
    budgets: Mutex<HashMap<PeerKey, Arc<RelayBudget>>>,
    budget: u64,
    /// Names only the host's half answers, for tests
    /// (`HIDE_TEST_SCREEN_NET_HOSTS`).
    hosts: HashMap<String, IpAddr>,
    tunnels: AtomicU64,
}

impl ScreenNet {
    pub fn new(reach: impl Reach, hosts: HashMap<String, IpAddr>) -> Arc<Self> {
        Self::with_budget(reach, hosts, RELAY_BUDGET)
    }

    pub fn with_budget(
        reach: impl Reach,
        hosts: HashMap<String, IpAddr>,
        budget: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            reach: Box::new(reach),
            listeners: Mutex::default(),
            places: Mutex::default(),
            budgets: Mutex::default(),
            budget,
            hosts,
            tunnels: AtomicU64::new(0),
        })
    }

    /// The pair's places. A pair's record goes once nothing holds it and no
    /// listener of it is left; `retire` does that when the last listener
    /// goes, and every ask here sweeps a record whose last tunnel ended
    /// after that, so the records kept are those of pairs with a listener
    /// or a tunnel, at most `MAX_LISTENERS` and their tunnels' pairs.
    fn places_of(&self, key: PeerKey) -> Arc<Places> {
        let listening: HashSet<PeerKey> = lock(&self.listeners)
            .keys()
            .map(|owner| owner.key)
            .collect();
        let mut places = lock(&self.places);
        places.retain(|held, kept| {
            *held == key || listening.contains(held) || Arc::strong_count(kept) > 1
        });
        Arc::clone(places.entry(key).or_default())
    }

    /// The pair's relay budget of the day. At most `MAX_BUDGETS` are kept:
    /// a record of another day or with nothing used goes first, and when
    /// every record holds bytes of today a new pair counts in the shared
    /// overflow record, so no pair's allowance is reset.
    pub fn budget_of(&self, key: PeerKey) -> Arc<RelayBudget> {
        let mut budgets = lock(&self.budgets);
        if let Some(budget) = budgets.get(&key) {
            return Arc::clone(budget);
        }
        if budgets.len() >= MAX_BUDGETS {
            budgets.retain(|_, budget| {
                Arc::strong_count(budget) > 1 || budget.used() > 0 || budget.spent()
            });
        }
        if budgets.len() >= MAX_BUDGETS {
            return Arc::clone(
                budgets
                    .entry(PeerKey::from_bytes([0; 32]))
                    .or_insert_with(|| Arc::new(RelayBudget::new(self.budget))),
            );
        }
        Arc::clone(
            budgets
                .entry(key)
                .or_insert_with(|| Arc::new(RelayBudget::new(self.budget))),
        )
    }

    /// What is kept for a pair once no listener of it is left: its places
    /// go when no tunnel holds or waits for one, and its budget when it
    /// counted nothing today.
    fn retire(&self, key: PeerKey) {
        if lock(&self.listeners).keys().any(|owner| owner.key == key) {
            return;
        }
        lock(&self.places).retain(|held, places| *held != key || Arc::strong_count(places) > 1);
        lock(&self.budgets).retain(|held, budget| {
            *held != key || Arc::strong_count(budget) > 1 || budget.used() > 0 || budget.spent()
        });
    }

    /// How many pairs' records are kept now.
    pub fn kept(&self) -> (usize, usize) {
        (lock(&self.places).len(), lock(&self.budgets).len())
    }

    /// The listener `pid` asked for `device` under `key`, opened on the
    /// first ask and the same while its owner holds it, with where requests
    /// through it stand now. `not_paired` opens nothing; `net_limit` when
    /// every place is held by another owner's.
    pub async fn ensure(
        self: &Arc<Self>,
        device: &str,
        key: PeerKey,
        pid: i32,
    ) -> Result<Ensured, &'static str> {
        let started =
            hide_node::pane_proof::process_start(pid).ok_or("browser_owner_unavailable")?;
        let now = self.reach.link(device, &key);
        if matches!(now, Err("not_paired")) {
            return Err("not_paired");
        }
        let pairing = self.reach.pairing(device, &key).ok_or("not_paired")?;
        let owner = Owner {
            device: device.to_owned(),
            key,
            pid,
        };
        let listener = match self.held(&owner, started) {
            Some(listener) => listener,
            None => self.open(owner, started).await?,
        };
        let unsupported = *lock(&listener.unsupported);
        let state = match now {
            Ok(link) if unsupported == Some(link.id()) => "unsupported_there",
            Ok(link) if link.path() == PathKind::Relay && self.budget_of(key).spent() => {
                "relay_budget"
            }
            Ok(_) => "ready",
            Err(reason) => reason,
        };
        Ok(Ensured {
            port: listener.port,
            secret: listener.secret.clone(),
            pairing,
            state,
        })
    }

    fn held(&self, owner: &Owner, started: u64) -> Option<Arc<Listener>> {
        let mut listeners = lock(&self.listeners);
        match listeners.get(owner) {
            Some(listener) if listener.started == started => Some(Arc::clone(listener)),
            // The pid was reused by another process: its listener is not the
            // new one's, and closes.
            Some(_) => {
                if let Some(earlier) = listeners.remove(owner) {
                    lock(&earlier.task).take();
                }
                drop(listeners);
                self.retire(owner.key);
                None
            }
            None => None,
        }
    }

    async fn open(
        self: &Arc<Self>,
        owner: Owner,
        started: u64,
    ) -> Result<Arc<Listener>, &'static str> {
        if lock(&self.listeners).len() >= MAX_LISTENERS {
            return Err("net_limit");
        }
        let socket = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .map_err(|_| "net_unavailable")?;
        let port = socket.local_addr().map_err(|_| "net_unavailable")?.port();
        let mut secret = [0_u8; 16];
        getrandom::getrandom(&mut secret).map_err(|_| "net_unavailable")?;
        let listener = Arc::new(Listener {
            owner: owner.clone(),
            started,
            port,
            secret: hex::encode(secret),
            unsupported: Mutex::new(None),
            refused: Mutex::default(),
            unproven: Arc::new(tokio::sync::Semaphore::new(MAX_UNPROVEN)),
            task: Mutex::new(None),
            #[cfg(test)]
            fail_accepts: std::sync::atomic::AtomicUsize::new(0),
        });
        {
            let mut listeners = lock(&self.listeners);
            // Another ask of the same owner opened one meanwhile: that one
            // is kept, and this socket closes.
            if let Some(held) = listeners.get(&owner)
                && held.started == started
            {
                return Ok(Arc::clone(held));
            }
            if listeners.len() >= MAX_LISTENERS {
                return Err("net_limit");
            }
            listeners.insert(owner.clone(), Arc::clone(&listener));
        }
        let task = tokio::spawn(Arc::clone(self).accept(Arc::clone(&listener), socket));
        *lock(&listener.task) = Some(AbortOnDrop(task.abort_handle()));
        hide_core::diagnostic!(json!({
            "component": "screen_net", "kind": "screen_net.listening",
            "device": owner.device, "live": lock(&self.listeners).len(),
        }));
        Ok(listener)
    }

    /// The desktop let go of its listener for `device` under `key`.
    pub fn release(&self, device: &str, key: PeerKey, pid: i32) -> bool {
        let owner = Owner {
            device: device.to_owned(),
            key,
            pid,
        };
        let removed = lock(&self.listeners).remove(&owner);
        let released = removed.is_some_and(|listener| {
            lock(&listener.task).take();
            true
        });
        self.retire(key);
        released
    }

    pub fn listening(&self) -> usize {
        lock(&self.listeners).len()
    }

    /// Whether `listener` may go on: its owner lives and its machine is
    /// still paired under its key.
    fn holds(&self, listener: &Listener) -> bool {
        owner_lives(listener.owner.pid, listener.started)
            && !matches!(
                self.reach.link(&listener.owner.device, &listener.owner.key),
                Err("not_paired")
            )
    }

    fn close(&self, listener: &Arc<Listener>, reason: &'static str) {
        let mut listeners = lock(&self.listeners);
        if listeners
            .get(&listener.owner)
            .is_some_and(|held| Arc::ptr_eq(held, listener))
        {
            listeners.remove(&listener.owner);
        }
        drop(listeners);
        hide_core::diagnostic!(json!({
            "component": "screen_net", "kind": "screen_net.closed",
            "device": listener.owner.device, "reason": reason,
        }));
        self.retire(listener.owner.key);
        lock(&listener.task).take();
    }

    async fn accept(self: Arc<Self>, listener: Arc<Listener>, socket: TcpListener) {
        let mut carried = JoinSet::new();
        let mut check = tokio::time::interval(CHECK_EVERY);
        check.tick().await;
        loop {
            tokio::select! {
                accepted = next_connection(&listener, &socket) => {
                    let local = match accepted {
                        Ok((local, _)) => local,
                        // A failed accept (a full file table, an aborted
                        // connection) fails that one connection: the port
                        // pages point at stays, and it is told once per kind.
                        Err(error) => {
                            let kind = format!("accept_{:?}", error.kind()).to_ascii_lowercase();
                            if lock(&listener.refused).insert(kind.clone()) {
                                hide_core::diagnostic!(json!({
                                    "component": "screen_net", "kind": "screen_net.accept_failed",
                                    "device": listener.owner.device, "error": kind,
                                }));
                            }
                            tokio::time::sleep(ACCEPT_AGAIN).await;
                            continue;
                        }
                    };
                    if !self.holds(&listener) {
                        break;
                    }
                    let net = Arc::clone(&self);
                    let held = Arc::clone(&listener);
                    carried.spawn(async move { net.carry(&held, local).await });
                }
                _ = check.tick() => {
                    if !self.holds(&listener) {
                        break;
                    }
                }
                Some(_) = carried.join_next(), if !carried.is_empty() => {}
            }
        }
        self.close(&listener, "owner_or_pairing_gone");
    }

    /// One connection of a page: its head and secret, then its place, the
    /// grant judged now, then one `page` stream carrying its bytes both ways,
    /// the head first, all of them reserved on the relay.
    async fn carry(&self, listener: &Listener, mut local: TcpStream) {
        let head = {
            let Ok(_unproven) = Arc::clone(&listener.unproven).try_acquire_owned() else {
                listener.refused("busy");
                return;
            };
            match tokio::time::timeout(HEAD_WITHIN, read_head(&mut local)).await {
                Ok(Ok(head)) => head,
                _ => return,
            }
        };
        if !authorized(&head, &listener.secret) {
            listener.refused("proxy_auth");
            answer(&mut local, "407 Proxy Authentication Required", None).await;
            return;
        }
        let Some(place) = self.places_of(listener.owner.key).take().await else {
            listener.refused("busy");
            answer(&mut local, "503 Service Unavailable", Some("busy")).await;
            return;
        };
        let link = match self.reach.link(&listener.owner.device, &listener.owner.key) {
            Ok(link) => link,
            Err(reason) => {
                listener.refused(reason);
                answer(&mut local, "503 Service Unavailable", Some(reason)).await;
                return;
            }
        };
        let budget = self.budget_of(listener.owner.key);
        if link.path() == PathKind::Relay && budget.spent() {
            listener.refused("relay_budget");
            answer(&mut local, "503 Service Unavailable", Some("relay_budget")).await;
            return;
        }
        let stream = match open_page(&link).await {
            Ok(stream) => stream,
            Err(reason) => {
                listener.refused(reason);
                answer(&mut local, "503 Service Unavailable", Some(reason)).await;
                return;
            }
        };
        let link_id = link.id();
        let relay = move || link.path() == PathKind::Relay;
        let (piped, back) = pipe(
            &mut local,
            &head,
            stream.recv,
            stream.send,
            &budget,
            &relay,
            &place.slot,
            &|| *lock(&listener.unsupported) = None,
        )
        .await;
        match piped {
            Piped::Unanswered => {
                *lock(&listener.unsupported) = Some(link_id);
                listener.refused("unsupported_there");
                answer(
                    &mut local,
                    "503 Service Unavailable",
                    Some("unsupported_there"),
                )
                .await;
            }
            Piped::Budget => {
                listener.refused("relay_budget");
                if back == 0 {
                    answer(&mut local, "503 Service Unavailable", Some("relay_budget")).await;
                }
            }
            Piped::Done | Piped::Reclaimed => {}
        }
    }

    /// The host's half: every `page` stream a granted guest opens is one
    /// connection to this machine's forward proxy. Called once, before the
    /// endpoint opens.
    pub fn serve(self: &Arc<Self>, peers: &Arc<PeerHost>) {
        let net = Arc::clone(self);
        let host = Arc::clone(peers);
        peers.handle(StreamKind::Page, move |incoming| {
            let net = Arc::clone(&net);
            let host = Arc::clone(&host);
            async move { net.proxy(&host, incoming).await }
        });
    }

    async fn proxy(self: Arc<Self>, host: &PeerHost, incoming: Incoming) {
        let key = incoming.link.key();
        let granted = host
            .peers()
            .and_then(|peers| peers.grants(|grants, _| grants.current(&key).cloned()))
            .is_some();
        if !granted {
            return;
        }
        let tunnel = self.tunnels.fetch_add(1, Ordering::SeqCst);
        // The stream's row is written when the stream itself goes: after a
        // `CONNECT`, the tunnel it was upgraded to outlives the HTTP session.
        let io = Counted {
            inner: tokio::io::join(incoming.stream.recv, incoming.stream.send),
            counts: Counts::default(),
            ended: Some((tunnel, incoming.link.path())),
        };
        serve_proxy(io, tunnel, Arc::new(self.hosts.clone())).await;
    }
}

/// A `page` stream to the host under the grant's generation. The host
/// counts its streams on its own side and frees a tunnel's place only once
/// that tunnel ended there, so a stream refused for the count is asked for
/// again until `DIAL_WITHIN`; the refusal says which of the reasons the
/// page's words tell apart.
async fn open_page(link: &Link) -> Result<crate::peer_host::Stream, &'static str> {
    use hide_peer::peers::OpenError;
    use hide_peer::protocol::hello::GrantState;
    use hide_peer::protocol::stream::Status;
    let deadline = Instant::now() + DIAL_WITHIN;
    loop {
        let opened = tokio::time::timeout_at(deadline, link.open(StreamKind::Page)).await;
        match opened {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(OpenError::Refused(Status::Limit)))
                if Instant::now() + OPEN_AGAIN < deadline =>
            {
                tokio::time::sleep(OPEN_AGAIN).await;
            }
            Ok(Err(OpenError::Refused(Status::Limit))) => return Err("busy"),
            _ => {
                return Err(match link.granted() {
                    GrantState::Paused => "paused_there",
                    GrantState::Active => "not_connected",
                    _ => "no_grant",
                });
            }
        }
    }
}

/// The listener's next connection; a test makes it report failures first.
async fn next_connection(
    listener: &Listener,
    socket: &TcpListener,
) -> io::Result<(TcpStream, SocketAddr)> {
    #[cfg(test)]
    if listener
        .fail_accepts
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok()
    {
        return Err(io::ErrorKind::ConnectionAborted.into());
    }
    #[cfg(not(test))]
    let _ = listener;
    socket.accept().await
}

/// One stream of the host's forward proxy, over any byte stream.
async fn serve_proxy<IO>(io: IO, tunnel: u64, hosts: Arc<HashMap<String, IpAddr>>)
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request: Request<Body>| {
        let hosts = Arc::clone(&hosts);
        async move { Ok::<_, std::convert::Infallible>(forward(request, tunnel, &hosts).await) }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(io), service)
        .with_upgrades()
        .await;
}

type ProxyBody = BoxBody<Bytes, hyper::Error>;

fn status(code: StatusCode) -> Response<ProxyBody> {
    let mut response = Response::new(
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = code;
    response
}

/// One request the guest's page sent: `CONNECT host:port` dials and joins
/// the two, an absolute-form `http` request goes to its origin, anything
/// else is refused. No address is refused: a grant is the host account's
/// whole authority (security spec 0), and the page is the guest's view of it.
async fn forward(
    request: Request<Body>,
    tunnel: u64,
    hosts: &HashMap<String, IpAddr>,
) -> Response<ProxyBody> {
    let Some(authority) = request.uri().authority().cloned() else {
        return status(StatusCode::BAD_REQUEST);
    };
    if request.method() == Method::CONNECT {
        let Some(port) = authority.port_u16() else {
            return status(StatusCode::BAD_REQUEST);
        };
        let Ok(far) = dial(authority.host(), port, tunnel, hosts).await else {
            return status(StatusCode::BAD_GATEWAY);
        };
        tokio::spawn(async move {
            if let Ok(upgraded) = hyper::upgrade::on(request).await {
                let mut near = hyper_util::rt::TokioIo::new(upgraded);
                let mut far = far;
                let _ = tokio::io::copy_bidirectional_with_sizes(&mut near, &mut far, CHUNK, CHUNK)
                    .await;
            }
        });
        return status(StatusCode::OK);
    }
    if request.uri().scheme_str() != Some("http") {
        return status(StatusCode::BAD_REQUEST);
    }
    let port = authority.port_u16().unwrap_or(80);
    let Ok(far) = dial(authority.host(), port, tunnel, hosts).await else {
        return status(StatusCode::BAD_GATEWAY);
    };
    let Ok((mut sender, connection)) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(far)).await
    else {
        return status(StatusCode::BAD_GATEWAY);
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let (mut parts, body) = request.into_parts();
    let origin_form = parts
        .uri
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .parse()
        .unwrap_or_else(|_| hyper::Uri::from_static("/"));
    parts.uri = origin_form;
    strip_hop_by_hop(&mut parts.headers);
    if !parts.headers.contains_key(header::HOST)
        && let Ok(value) = authority.as_str().parse()
    {
        parts.headers.insert(header::HOST, value);
    }
    match sender.send_request(Request::from_parts(parts, body)).await {
        Ok(response) => {
            let (mut parts, body) = response.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            Response::from_parts(parts, body.boxed())
        }
        Err(_) => status(StatusCode::BAD_GATEWAY),
    }
}

/// The headers of one hop: the proxy's own credentials (which a guest's
/// kept-alive connection carries to every later request) and the
/// connection's, never sent on.
fn strip_hop_by_hop(headers: &mut header::HeaderMap) {
    let named: Vec<header::HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| name.trim().parse().ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        "proxy-authorization",
        "proxy-authenticate",
        "proxy-connection",
        "connection",
        "keep-alive",
        "te",
        "trailer",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

/// Dials `host` on this machine's network, as its own resolver answers.
async fn dial(
    host: &str,
    port: u16,
    tunnel: u64,
    hosts: &HashMap<String, IpAddr>,
) -> io::Result<TcpStream> {
    let name = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let addresses: Vec<SocketAddr> = match hosts.get(&name) {
        Some(ip) => vec![SocketAddr::new(*ip, port)],
        None => tokio::time::timeout(DIAL_WITHIN, tokio::net::lookup_host((name.as_str(), port)))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??
            .collect(),
    };
    let deadline = Instant::now() + DIAL_WITHIN;
    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for address in addresses {
        // Each address gets a bounded share, so a dead family leaves the next one time.
        let until = deadline.min(Instant::now() + DIAL_EACH);
        match tokio::time::timeout_at(until, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                hide_core::diagnostic!(json!({
                    "component": "screen_net", "kind": "screen_net.dialed", "tunnel": tunnel,
                    "local_port": stream.local_addr().map_or(0, |local| local.port()),
                }));
                return Ok(stream);
            }
            Ok(Err(error)) => last = error,
            Err(_) if Instant::now() < deadline => last = io::Error::from(io::ErrorKind::TimedOut),
            Err(_) => return Err(io::Error::from(io::ErrorKind::TimedOut)),
        }
    }
    Err(last)
}

#[derive(Clone, Default)]
struct Counts {
    read: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
}

/// A byte stream that counts what crosses it and, with `ended`, logs the
/// tunnel's row when it is dropped.
struct Counted<IO> {
    inner: IO,
    counts: Counts,
    ended: Option<(u64, PathKind)>,
}

impl<IO> Drop for Counted<IO> {
    fn drop(&mut self) {
        if let Some((tunnel, path)) = self.ended.take() {
            hide_core::diagnostic!(json!({
                "component": "screen_net", "kind": "screen_net.ended", "tunnel": tunnel,
                "bytes_in": self.counts.read.load(Ordering::SeqCst),
                "bytes_out": self.counts.written.load(Ordering::SeqCst),
                "path": path,
            }));
        }
    }
}

impl<IO: AsyncRead + Unpin> AsyncRead for Counted<IO> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = buf.filled().len() - before;
        self.counts.read.fetch_add(read as u64, Ordering::Relaxed);
        polled
    }
}

impl<IO: AsyncWrite + Unpin> AsyncWrite for Counted<IO> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let polled = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(written)) = &polled {
            self.counts
                .written
                .fetch_add(*written as u64, Ordering::Relaxed);
        }
        polled
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// How a tunnel's two copies ended.
#[derive(Debug, Eq, PartialEq)]
enum Piped {
    /// Both sides closed, or one failed.
    Done,
    /// The host closed before any byte came back while the page still
    /// waited: a build with no handler for the kind.
    Unanswered,
    /// A chunk could not be reserved on the relay.
    Budget,
    /// A new connection took its quiet place.
    Reclaimed,
}

enum Copied {
    Closed,
    Budget,
    /// Reading its source failed.
    ReadFailed,
    /// Writing to its far end failed.
    WriteFailed,
}

/// Copies `from` into `to` in chunks of at most `CHUNK`, each reserved
/// against `budget` before it is written while `relay` says the
/// connection takes the relay.
#[allow(clippy::too_many_arguments)]
async fn copy_reserved<R, W>(
    mut from: R,
    mut to: W,
    first: &[u8],
    budget: &RelayBudget,
    relay: &(dyn Fn() -> bool + Send + Sync),
    slot: &Slot,
    moved: &AtomicU64,
    answered: &(dyn Fn() + Send + Sync),
) -> Copied
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // What was read before the copy began (a connection's head) goes first,
    // reserved and counted as every later chunk is.
    for chunk in first.chunks(CHUNK) {
        if relay() && !budget.reserve(chunk.len() as u64) {
            return Copied::Budget;
        }
        if to.write_all(chunk).await.is_err() || to.flush().await.is_err() {
            return Copied::WriteFailed;
        }
        moved.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        slot.touch();
    }
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        // On the relay a read is reserved before it is made, at most what
        // the day has left: what the other side sends past the budget stays
        // in the transport's receive window, never read here, and the part
        // of the hold the read did not take goes back.
        let mut held = if relay() {
            match budget.hold(CHUNK as u64) {
                Some(held) => Some(held),
                None => return Copied::Budget,
            }
        } else {
            None
        };
        let room = held.as_ref().map_or(CHUNK, Held::room);
        let read = match from.read(&mut buffer[..room]).await {
            Ok(0) => {
                drop(held);
                let _ = to.shutdown().await;
                return Copied::Closed;
            }
            Ok(read) => read,
            Err(_) => return Copied::ReadFailed,
        };
        if let Some(held) = held.as_mut() {
            held.keep(read as u64);
        }
        drop(held);
        if to.write_all(&buffer[..read]).await.is_err() || to.flush().await.is_err() {
            return Copied::WriteFailed;
        }
        if moved.fetch_add(read as u64, Ordering::Relaxed) == 0 {
            answered();
        }
        slot.touch();
    }
}

/// A tunnel's bytes both ways, `head` first on the way up, until both sides
/// close, either fails, a chunk is refused on the relay, or its place is
/// taken; with the bytes that came back. The host closing the stream, or
/// refusing the head's write, before any byte came back while the page
/// still waited is a build with no handler for the kind.
#[allow(clippy::too_many_arguments)]
async fn pipe<L, FR, FW>(
    local: &mut L,
    head: &[u8],
    far_recv: FR,
    far_send: FW,
    budget: &RelayBudget,
    relay: &(dyn Fn() -> bool + Send + Sync),
    slot: &Slot,
    answered: &(dyn Fn() + Send + Sync),
) -> (Piped, u64)
where
    L: AsyncRead + AsyncWrite + Unpin,
    FR: AsyncRead + Unpin,
    FW: AsyncWrite + Unpin,
{
    let (local_read, local_write) = tokio::io::split(local);
    let (sent, back) = (AtomicU64::new(0), AtomicU64::new(0));
    let up = copy_reserved(
        local_read,
        far_send,
        head,
        budget,
        relay,
        slot,
        &sent,
        &|| {},
    );
    let down = copy_reserved(
        far_recv,
        local_write,
        &[],
        budget,
        relay,
        slot,
        &back,
        answered,
    );
    tokio::pin!(up, down);
    let (mut up_closed, mut down_closed) = (false, false);
    let unanswered = |up_closed: bool| back.load(Ordering::Relaxed) == 0 && !up_closed;
    let piped = loop {
        tokio::select! {
            () = slot.reclaim.notified() => break Piped::Reclaimed,
            copied = &mut up, if !up_closed => match copied {
                Copied::Closed => up_closed = true,
                Copied::Budget => break Piped::Budget,
                Copied::WriteFailed if unanswered(false) => break Piped::Unanswered,
                Copied::WriteFailed | Copied::ReadFailed => break Piped::Done,
            },
            copied = &mut down, if !down_closed => match copied {
                Copied::Closed if unanswered(up_closed) => break Piped::Unanswered,
                Copied::Closed => down_closed = true,
                Copied::Budget => break Piped::Budget,
                Copied::WriteFailed | Copied::ReadFailed => break Piped::Done,
            },
        }
        if up_closed && down_closed {
            break Piped::Done;
        }
    };
    (piped, back.load(Ordering::Relaxed))
}

/// A connection's first request head, with whatever followed it in the same
/// reads, which goes on with it.
async fn read_head<R: AsyncRead + Unpin>(local: &mut R) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = local.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        head.extend_from_slice(&buffer[..read]);
        // The head, its blank line included, is held to `MAX_HEAD`; body
        // bytes read with a head within it go on with it.
        match head.windows(4).position(|window| window == b"\r\n\r\n") {
            Some(end) if end + 4 <= MAX_HEAD => return Ok(head),
            Some(_) => return Err(io::ErrorKind::InvalidData.into()),
            None if head.len() >= MAX_HEAD => return Err(io::ErrorKind::InvalidData.into()),
            None => {}
        }
    }
}

/// Whether a head's `Proxy-Authorization` is `hide` with `secret`, compared
/// in constant time.
fn authorized(head: &[u8], secret: &str) -> bool {
    let end = head
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or(head.len());
    let Ok(text) = std::str::from_utf8(&head[..end]) else {
        return false;
    };
    let Some(value) = text.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("proxy-authorization")
            .then(|| value.trim())
    }) else {
        return false;
    };
    let Some((scheme, encoded)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let expected = format!("hide:{secret}");
    decoded.len() == expected.len() && bool::from(decoded.ct_eq(expected.as_bytes()))
}

/// Answers the page's connection itself, with nothing opened behind it.
async fn answer<W: AsyncWrite + Unpin>(local: &mut W, status: &str, reason: Option<&str>) {
    let challenge = if status.starts_with("407") {
        "Proxy-Authenticate: Basic realm=\"hide\"\r\n"
    } else {
        ""
    };
    let reason = reason.map_or_else(String::new, |reason| format!("x-hide-reason: {reason}\r\n"));
    let response = format!(
        "HTTP/1.1 {status}\r\n{challenge}{reason}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = local.write_all(response.as_bytes()).await;
    let _ = local.shutdown().await;
}

/// `HIDE_TEST_SCREEN_NET_HOSTS`: `name=ip` pairs, comma separated.
pub fn parse_hosts(value: &str) -> Option<HashMap<String, IpAddr>> {
    value
        .split(',')
        .filter(|pair| !pair.trim().is_empty())
        .map(|pair| {
            let (name, ip) = pair.split_once('=')?;
            let name = name.trim().to_ascii_lowercase();
            (!name.is_empty() && !name.contains(['/', ':', ' '])).then_some(())?;
            Some((name, ip.trim().parse().ok()?))
        })
        .collect()
}

#[cfg(test)]
#[path = "screen_net_tests.rs"]
mod tests;
