//! How two machines become a pair, and stop being one (PRD B1-B7, B25-B27,
//! B50, B51; D-05, D-30, D-51).
//!
//! [`Pairings`] sits above [`Peers`] and owns what a pairing needs besides
//! the two records: the one offer a host has open, the requests that wait
//! for this machine's operator, the pairing this machine is joining, and
//! the confirm and the removal a guest keeps sending until its host
//! answered. It decides nothing about a grant that [`Peers`] does not save
//! first.
//!
//! A pairing by link, seen from both ends:
//!
//! ```text
//! guest                                   host
//!   join(link) -- dial,
//!        pair{secret, commit} --------->    the offer's secret? it is used up
//!              <-- asking{name, words,
//!                         nonce} -------    its own value, drawn only now
//!              --- reveal{nonce} ------->   opens the commitment? asks its
//!   shows the same four words               operator (two minutes), and
//!                                           shows the same four words
//!   compare(true)                           answer(ask, true)
//!              <-- granted{grant} --------  saved as pending: opens nothing
//!   saved as confirming
//!              --- confirm{grant} ------->  saved as active
//!              <-- confirmed{active} -----
//!   saved as active
//! ```
//!
//! Either operator saying no, two minutes without an answer, or the
//! connection ending leaves no grant that opens anything: the host's
//! pending grant is told to go once and expires on its own otherwise.
//!
//! What is shown here never leaves this machine's own screens: the link and
//! the words are in [`View`], which is neither logged nor part of any
//! snapshot a peer can read.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hide_peer_protocol::frame::MAX_MESSAGE;
use hide_peer_protocol::hello::GrantState;
use hide_peer_protocol::pairing::{ConfirmState, PairRefusal, Pairing as Message, Revoke};
use hide_peer_protocol::{Candidate, CloseCode, PeerAddr, PeerKey, StreamKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::link::{self, LinkError, PairLink, Secret};
use crate::peers::{DialError, Incoming, Keeper, Link, PeerEvent, PeerStream, Peers};
use crate::rate::Rate;
use crate::store::{Host, KnownState, StoreError, Via};
use crate::transport::PeerTransport;
use crate::words::{self, Nonce, WordList};
use crate::{limits, wire};

/// This machine, as a peer is told of it.
#[derive(Clone, Debug)]
pub struct Me {
    /// Its product device id.
    pub device: String,
    pub name: String,
}

/// What this machine's own screens show of the pairings under way.
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
pub struct View {
    /// The link this machine shares now.
    pub offer: Option<OfferView>,
    /// The machines that wait for this machine's operator.
    pub asks: Vec<AskView>,
    /// The pairing this machine is joining, or how the last one ended.
    pub join: Option<JoinView>,
    /// What came of asking a guest to be let in the other way.
    pub requests: Vec<RequestView>,
}

impl fmt::Debug for View {
    /// The link and the words are an operator's to read, not a log's.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("View")
            .field("offer", &self.offer.is_some())
            .field("asks", &self.asks.len())
            .field("join", &self.join.as_ref().map(|join| join.phase))
            .field("requests", &self.requests)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct OfferView {
    pub link: String,
    pub expires_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// A machine that pasted this machine's link.
    Link,
    /// A machine this one is a guest of asks to be let in the other way.
    Request,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct AskView {
    pub id: u64,
    pub key: PeerKey,
    pub device: String,
    pub name: String,
    pub kind: AskKind,
    /// The words to compare; none for a request over a connection that
    /// already proved both machines.
    pub words: Option<[&'static str; words::COUNT]>,
    pub expires_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinPhase {
    Connecting,
    /// The words are shown and this machine's operator has not answered.
    Comparing,
    /// The operator said the words match; the host's answer is awaited.
    Settling,
    Paired,
    /// Saved here and not yet confirmed by the host: the confirm is sent
    /// again until it answers.
    Waiting,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinFailure {
    /// The link is used, expired, replaced, or not this host's.
    NoOffer,
    /// The host's operator said no.
    Declined,
    /// The host holds as many machines as it takes (B6).
    HostFull,
    /// This machine knows as many hosts as it takes.
    Full,
    /// Nobody answered in time.
    Unanswered,
    /// The host could not be reached, the connection ended midway, or a
    /// record could not be saved (B5).
    Unreachable,
    /// One of the two builds is too old to pair with the other.
    UpdateNeeded,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct JoinView {
    pub id: u64,
    pub host: PeerKey,
    pub phase: JoinPhase,
    pub failure: Option<JoinFailure>,
    /// The host's name, once it said it.
    pub name: Option<String>,
    pub words: Option<[&'static str; words::COUNT]>,
    pub expires_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Waiting,
    Declined,
    Unanswered,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequestView {
    pub key: PeerKey,
    pub state: RequestState,
}

#[derive(Debug)]
pub enum ShareError {
    /// This machine's device id or relay cannot be written into a link.
    Unspellable,
    Io(io::Error),
}

impl fmt::Display for ShareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unspellable => f.write_str("this machine cannot be written into a link"),
            Self::Io(error) => write!(f, "no link could be made: {error}"),
        }
    }
}

impl std::error::Error for ShareError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinError {
    Link(LinkError),
    /// The link is this machine's own, or names its device id.
    Own,
    /// This machine is already a guest of that host (B51).
    Already,
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link(error) => error.fmt(f),
            Self::Own => f.write_str("the link is this machine's own"),
            Self::Already => f.write_str("this machine is already connected to that one"),
        }
    }
}

impl std::error::Error for JoinError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// That machine is no guest of this one: there is no connection to ask
    /// over, and a link pairs them instead.
    NotAGuest,
    /// This machine is already a guest of it.
    Already,
    /// An earlier request still waits for its answer.
    Waiting,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAGuest => "that machine is not connected to this one",
            Self::Already => "this machine is already connected to that one",
            Self::Waiting => "an earlier request still waits",
        })
    }
}

impl std::error::Error for RequestError {}

#[derive(Debug)]
pub enum AcceptError {
    /// The request does not name a machine that can be a guest here.
    Invalid,
    /// This machine is already a guest of that host under another grant
    /// (B51).
    Already,
    Store(StoreError),
}

impl fmt::Display for AcceptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => f.write_str("the request does not name a machine to pair with"),
            Self::Already => f.write_str("this machine is already connected to that one"),
            Self::Store(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AcceptError {}

impl From<StoreError> for AcceptError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// What a host answers a pairing over an already trusted channel (SSH,
/// `wsl.exe`) with: everything its guest saves to confirm over the
/// connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    pub key: PeerKey,
    pub device: String,
    pub name: String,
    pub grant: String,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
}

struct Offer {
    id: u64,
    secret: Secret,
    list: WordList,
    link: Zeroizing<String>,
    expires_unix_ms: u64,
}

struct Ask {
    view: AskView,
    /// The connection the request came on: an answer counts only while it
    /// is still that machine's connection.
    conn: u64,
    answer: Option<oneshot::Sender<bool>>,
}

struct Join {
    view: JoinView,
    said: Option<oneshot::Sender<bool>>,
}

struct AcceptedRequest {
    request: String,
    guest: PeerKey,
    grant: String,
}

#[derive(Default)]
struct State {
    next: u64,
    offer: Option<Offer>,
    asks: Vec<Ask>,
    join: Option<Join>,
    requests: Vec<RequestView>,
    /// The hosts a task is settling with ([`Inner::settle`]).
    settling: HashSet<PeerKey>,
    accepted: VecDeque<AcceptedRequest>,
}

impl State {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn view(&self) -> View {
        View {
            offer: self.offer.as_ref().map(|offer| OfferView {
                link: offer.link.to_string(),
                expires_unix_ms: offer.expires_unix_ms,
            }),
            asks: self.asks.iter().map(|ask| ask.view.clone()).collect(),
            join: self.join.as_ref().map(|join| join.view.clone()),
            requests: self.requests.clone(),
        }
    }
}

/// What a guest's `pair` says.
struct Asked {
    secret: String,
    device: String,
    name: String,
    /// The guest's commitment to its value for the words.
    commit: String,
}

/// How an operator's answer to a request came out.
enum Said {
    Allow,
    Deny,
    /// Two minutes passed.
    Nobody,
    /// The offer the request came under was replaced or ended.
    Ended,
    /// The machine that asked took it back, or left.
    Left,
}

enum JoinEnd {
    Paired,
    Waiting,
    Failed(JoinFailure),
    /// This machine's operator said the words differ, or closed the window.
    Dismissed,
}

enum Settled {
    Active,
    /// The host holds no such grant: pair again.
    Gone,
    /// The host confirmed and this machine could not save it: sent again.
    NotSaved,
}

struct Inner<T: PeerTransport> {
    peers: Peers<T>,
    me: Me,
    state: Mutex<State>,
    view: watch::Sender<View>,
    /// Counts every change of a record made here.
    saved: watch::Sender<u64>,
    confirms: Rate,
    revokes: Rate,
    requests: Rate,
    keepers: Mutex<HashMap<PeerKey, Keeper>>,
    /// One `accept-ssh` at a time, so the same request run twice at once
    /// makes one grant.
    accepting: Mutex<()>,
    stop: watch::Sender<bool>,
}

/// The pairings of one machine.
pub struct Pairings<T: PeerTransport> {
    inner: Arc<Inner<T>>,
}

impl<T: PeerTransport> Clone for Pairings<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // What these locks guard is memory a panic leaves whole: the records
    // are in `Peers`, saved before anything here acted on them.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn refused(reason: PairRefusal) -> Option<Message> {
    Some(Message::Refused { reason })
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

/// The messages of a pairing stream, read by a task of its own so whoever
/// waits for the next one can wait for something else too. A message of a
/// later build is skipped; the channel ends with the stream.
fn messages<R>(mut recv: R) -> mpsc::Receiver<Message>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (heard, messages) = mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            let body = tokio::select! {
                () = heard.closed() => return,
                body = wire::read_message(&mut recv, MAX_MESSAGE) => body,
            };
            let Ok(Some(body)) = body else {
                return;
            };
            match Message::parse(&body) {
                Ok(Some(message)) => {
                    if heard.send(message).await.is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    });
    messages
}

/// Resolves at the next connection to `key`. Skipped events may have held
/// one, so they count as one.
async fn connected(events: &mut broadcast::Receiver<PeerEvent>, key: &PeerKey) {
    loop {
        match events.recv().await {
            Ok(PeerEvent::Connected { key: connected, .. }) if connected == *key => return,
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => return,
            Err(broadcast::error::RecvError::Closed) => std::future::pending::<()>().await,
        }
    }
}

impl<T: PeerTransport> Pairings<T> {
    /// Starts keeping every host this machine knows connected, and goes on
    /// with what was not settled when it last stopped. Call it right after
    /// [`Peers::start`], on the same runtime.
    pub fn start(peers: Peers<T>, me: Me) -> io::Result<Self> {
        if !link::valid_device(&me.device) || !link::valid_name(&me.name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "this machine's device id or name cannot be shown to a peer",
            ));
        }
        let events = peers.events();
        let inner = Arc::new(Inner {
            peers,
            me,
            state: Mutex::new(State::default()),
            view: watch::channel(View::default()).0,
            saved: watch::channel(0).0,
            confirms: Rate::new(),
            revokes: Rate::new(),
            requests: Rate::new(),
            keepers: Mutex::new(HashMap::new()),
            accepting: Mutex::new(()),
            stop: watch::channel(false).0,
        });
        for host in inner.peers.hosts() {
            if host.state.dialed() {
                inner.keep(host.key);
            }
            if matches!(
                host.state,
                KnownState::Confirming | KnownState::RevokePending
            ) {
                inner.settle(host.key);
            }
        }
        tokio::spawn(Arc::clone(&inner).watch_grants(events));
        Ok(Self { inner })
    }

    /// What this machine's screens show. It holds the link and the words:
    /// hand it to this machine's own windows and terminal, nowhere else.
    pub fn view(&self) -> watch::Receiver<View> {
        self.inner.view.subscribe()
    }

    /// Moves whenever a record changed here: a grant saved, confirmed or
    /// removed, a host adopted, paused or forgotten.
    pub fn saved(&self) -> watch::Receiver<u64> {
        self.inner.saved.subscribe()
    }

    /// Takes a pairing or a revoke stream a peer opened. Any other stream
    /// is handed back.
    pub fn serve(&self, incoming: Incoming<T::Conn>) -> Option<Incoming<T::Conn>> {
        let inner = Arc::clone(&self.inner);
        match incoming.stream.kind {
            StreamKind::Pairing => {
                tokio::spawn(inner.serve_pairing(incoming.link, incoming.stream));
            }
            StreamKind::Revoke => {
                tokio::spawn(inner.serve_revoke(incoming.link, incoming.stream));
            }
            _ => return Some(incoming),
        }
        None
    }

    /// Opens this machine to one guest for nine minutes and returns the
    /// link to hand it. An offer open before is over, with whoever was
    /// being asked under it.
    pub fn share(&self, list: WordList) -> Result<OfferView, ShareError> {
        let inner = &self.inner;
        let secret = Secret::random().map_err(ShareError::Io)?;
        let candidates = inner.peers.transport().candidates();
        let link = PairLink {
            host: inner.peers.key(),
            device: inner.me.device.clone(),
            relay: candidates.iter().find_map(|candidate| match candidate {
                Candidate::Relay(url) => Some(url.clone()),
                Candidate::Ip(_) | Candidate::Wsl(_) | Candidate::Ssh(_) => None,
            }),
            addrs: candidates
                .iter()
                .filter_map(|candidate| match candidate {
                    Candidate::Ip(addr) => Some(*addr),
                    Candidate::Relay(_) | Candidate::Wsl(_) | Candidate::Ssh(_) => None,
                })
                .collect(),
            secret: secret.clone(),
        }
        .text()
        .ok_or(ShareError::Unspellable)?;
        let expires_unix_ms = inner.peers.now() + millis(limits::OFFER_FOR);
        let shown = OfferView {
            link: link.to_string(),
            expires_unix_ms,
        };
        let id = {
            let mut state = lock(&inner.state);
            let id = state.id();
            state.asks.retain(|ask| ask.view.kind != AskKind::Link);
            state.offer = Some(Offer {
                id,
                secret,
                list,
                link,
                expires_unix_ms,
            });
            id
        };
        inner.publish();
        let inner = Arc::clone(inner);
        tokio::spawn(async move {
            let mut stop = inner.stop.subscribe();
            tokio::select! {
                () = tokio::time::sleep(limits::OFFER_FOR) => {}
                _ = stop.wait_for(|stop| *stop) => return,
            }
            let over = {
                let mut state = lock(&inner.state);
                let over = state.offer.as_ref().is_some_and(|offer| offer.id == id);
                if over {
                    state.offer = None;
                }
                over
            };
            if over {
                inner.publish();
            }
        });
        Ok(shown)
    }

    /// Ends the open offer, and whoever was being asked under it: the
    /// operator closed the window or the terminal that shared it.
    pub fn end_offer(&self) {
        {
            let mut state = lock(&self.inner.state);
            state.offer = None;
            state.asks.retain(|ask| ask.view.kind != AskKind::Link);
        }
        self.inner.publish();
    }

    /// The operator's answer to request `ask`. `false` when nothing was
    /// answered: the request is gone, its time is over, or the connection
    /// it came on is no longer that machine's.
    pub fn answer(&self, ask: u64, allow: bool) -> bool {
        let inner = &self.inner;
        let answered = {
            let mut state = lock(&inner.state);
            let now = inner.peers.now();
            let Some(found) = state.asks.iter_mut().find(|found| found.view.id == ask) else {
                return false;
            };
            let current = found.view.expires_unix_ms > now
                && inner
                    .peers
                    .link(&found.view.key)
                    .is_some_and(|link| link.id() == found.conn);
            match found.answer.take() {
                Some(answer) if current => answer.send(allow).is_ok(),
                // Dropped unanswered: whoever waits reads the request as
                // ended.
                _ => false,
            }
        };
        if !answered {
            lock(&inner.state).asks.retain(|found| found.view.id != ask);
            inner.publish();
        }
        answered
    }

    /// Stops what guest `key` runs here: its streams end and none opens
    /// until [`Pairings::resume_guest`] (B25).
    pub fn pause_guest(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.inner.peers.pause(key)?;
        self.inner.saved();
        Ok(())
    }

    pub fn resume_guest(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.inner.peers.resume(key)?;
        self.inner.saved();
        Ok(())
    }

    /// Removes guest `key`: it is cut off at once and its key opens nothing
    /// here again until it is paired again (B26).
    pub fn remove_guest(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.inner.peers.revoke(key, None)?;
        self.inner.saved();
        Ok(())
    }

    /// Saves a pending grant for a guest that reached this machine over a
    /// channel its account already trusts (SSH, `wsl.exe`), and answers
    /// what that guest saves to confirm over the connection. The same
    /// request run again answers the same while its grant is there.
    pub fn accept_ssh(
        &self,
        request: &str,
        guest: PeerKey,
        device: &str,
        name: &str,
        via: Via,
    ) -> Result<Accepted, AcceptError> {
        let inner = &self.inner;
        let spelled = !request.is_empty()
            && request.len() <= 64
            && request
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !spelled
            || guest == inner.peers.key()
            || device == inner.me.device
            || !link::valid_device(device)
            || !link::valid_name(name)
        {
            return Err(AcceptError::Invalid);
        }
        let _one = lock(&inner.accepting);
        let remembered = lock(&inner.state)
            .accepted
            .iter()
            .find(|accepted| accepted.request == request && accepted.guest == guest)
            .map(|accepted| accepted.grant.clone())
            .filter(|grant| {
                inner.peers.grants(|grants, now| {
                    grants
                        .list(now)
                        .any(|held| held.key == guest && held.id == *grant)
                })
            });
        let grant = match remembered {
            Some(grant) => grant,
            None => {
                let grant = inner.peers.grant_pending(guest, device, name, via)?.id;
                let mut state = lock(&inner.state);
                if state.accepted.len() >= limits::MAX_ACCEPTED {
                    state.accepted.pop_front();
                }
                state.accepted.push_back(AcceptedRequest {
                    request: request.to_owned(),
                    guest,
                    grant: grant.clone(),
                });
                drop(state);
                inner.saved();
                grant
            }
        };
        Ok(Accepted {
            key: inner.peers.key(),
            device: inner.me.device.clone(),
            name: inner.me.name.clone(),
            grant,
            candidates: inner.peers.transport().candidates(),
        })
    }

    /// Starts pairing with the host a link names. How it goes is in
    /// [`View::join`]; a pairing already being joined is given up for it.
    pub fn join(&self, text: &str) -> Result<(), JoinError> {
        let inner = &self.inner;
        let link = PairLink::parse(text).map_err(JoinError::Link)?;
        if link.host == inner.peers.key() || link.device == inner.me.device {
            return Err(JoinError::Own);
        }
        if inner.hosted_by(&link.host) {
            return Err(JoinError::Already);
        }
        let (said, heard) = oneshot::channel();
        let id = {
            let mut state = lock(&inner.state);
            let id = state.id();
            state.join = Some(Join {
                view: JoinView {
                    id,
                    host: link.host,
                    phase: JoinPhase::Connecting,
                    failure: None,
                    name: None,
                    words: None,
                    expires_unix_ms: None,
                },
                said: Some(said),
            });
            id
        };
        inner.publish();
        let inner = Arc::clone(inner);
        tokio::spawn(async move {
            let end = inner.run_join(id, link, heard).await;
            {
                let mut state = lock(&inner.state);
                let Some(join) = state.join.as_mut().filter(|join| join.view.id == id) else {
                    return;
                };
                join.view.words = None;
                join.view.expires_unix_ms = None;
                match end {
                    JoinEnd::Paired => join.view.phase = JoinPhase::Paired,
                    JoinEnd::Waiting => join.view.phase = JoinPhase::Waiting,
                    JoinEnd::Failed(failure) => {
                        join.view.phase = JoinPhase::Failed;
                        join.view.failure = Some(failure);
                    }
                    JoinEnd::Dismissed => state.join = None,
                }
            }
            inner.publish();
        });
        Ok(())
    }

    /// The operator's answer to "are these the words on the host's
    /// screen?". `false` when no words are being compared.
    pub fn compare(&self, same: bool) -> bool {
        let said = {
            let mut state = lock(&self.inner.state);
            state
                .join
                .as_mut()
                .filter(|join| join.view.phase == JoinPhase::Comparing)
                .and_then(|join| join.said.take())
        };
        said.is_some_and(|said| said.send(same).is_ok())
    }

    /// Clears the pairing being joined from the view: the operator closed
    /// its window. One whose words the operator already confirmed goes on.
    pub fn dismiss_join(&self) {
        lock(&self.inner.state).join = None;
        self.inner.publish();
    }

    /// Makes this machine a guest of a host that answered a pairing over
    /// SSH or `wsl.exe`: saved as confirming, then confirmed over the
    /// connection. `first` are addresses to try before the host's own (the
    /// address the SSH login reached, D-25), and `ssh` the alias the
    /// pairing signed in through, kept as the host's SSH route
    ///: the host the alias reached answered with the key
    /// that will be confirmed over the connection.
    pub fn adopt(
        &self,
        accepted: Accepted,
        first: Vec<Candidate>,
        ssh: Option<String>,
    ) -> Result<(), AcceptError> {
        let inner = &self.inner;
        if accepted.key == inner.peers.key()
            || accepted.device == inner.me.device
            || !link::valid_device(&accepted.device)
            || !link::valid_name(&accepted.name)
        {
            return Err(AcceptError::Invalid);
        }
        // The same answer adopted again is the same pairing going on.
        let again = inner
            .peers
            .host(&accepted.key)
            .is_some_and(|host| host.grant == accepted.grant);
        if !again && inner.hosted_by(&accepted.key) {
            return Err(AcceptError::Already);
        }
        if !again {
            let mut candidates = first;
            for candidate in accepted.candidates {
                // Which distribution a machine is, and the alias it is
                // signed in to by, only this machine says: a host's answer
                // never picks a program to run.
                if matches!(candidate, Candidate::Wsl(_) | Candidate::Ssh(_)) {
                    continue;
                }
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
            inner.adopt_host(Host {
                key: accepted.key,
                device: accepted.device,
                name: accepted.name,
                chosen_name: None,
                grant: accepted.grant,
                state: KnownState::Confirming,
                candidates,
                started: None,
                ssh: ssh
                    .filter(|alias| crate::store::valid_alias(alias))
                    .map(crate::store::SshRoute::Alias),
            })?;
        }
        inner.settle(accepted.key);
        Ok(())
    }

    /// Stops this machine's use of host `key`: the streams it opened there
    /// end, none opens, and the host is not dialed until
    /// [`Pairings::resume_host`] (B25).
    pub fn pause_host(&self, key: &PeerKey) -> Result<(), StoreError> {
        let inner = &self.inner;
        match inner.peers.host(key).map(|host| host.state) {
            Some(KnownState::Active | KnownState::Confirming) => {}
            Some(KnownState::PausedByMe) => return Ok(()),
            // A host being removed, or one that dropped the pairing, has
            // nothing left to pause.
            Some(KnownState::RevokePending | KnownState::Unpaired) | None => {
                return Err(StoreError::Unknown);
            }
        }
        inner.peers.set_host(key, KnownState::PausedByMe)?;
        inner.unkeep(key);
        inner.leave_unless_hosting(key);
        inner.saved();
        Ok(())
    }

    pub fn resume_host(&self, key: &PeerKey) -> Result<(), StoreError> {
        let inner = &self.inner;
        match inner.peers.host(key).map(|host| host.state) {
            Some(KnownState::PausedByMe) => {}
            Some(KnownState::Active | KnownState::Confirming) => return Ok(()),
            Some(KnownState::RevokePending | KnownState::Unpaired) | None => {
                return Err(StoreError::Unknown);
            }
        }
        inner.peers.set_host(key, KnownState::Active)?;
        inner.keep(*key);
        inner.saved();
        Ok(())
    }

    /// Stops being a guest of host `key`. Its streams end at once, and the
    /// host is told until it answers (B27). Asked again while the host has
    /// not answered, or of a host that dropped the pairing itself, the
    /// record is forgotten here: what the host still holds is then its
    /// operator's to remove.
    pub fn remove_host(&self, key: &PeerKey) -> Result<(), StoreError> {
        let inner = &self.inner;
        let Some(host) = inner.peers.host(key) else {
            return Ok(());
        };
        match host.state {
            KnownState::RevokePending | KnownState::Unpaired => inner.forget_host(key),
            KnownState::Active | KnownState::Confirming | KnownState::PausedByMe => {
                inner.peers.set_host(key, KnownState::RevokePending)?;
                // A paused host was not being dialed; the removal has to
                // reach it.
                inner.keep(*key);
                inner.saved();
                inner.settle(*key);
                Ok(())
            }
        }
    }

    /// Asks guest `key`, over the connection the two share, to let this
    /// machine in the other way (B50). What comes of it is in
    /// [`View::requests`] until it succeeded.
    pub fn request(&self, key: PeerKey) -> Result<(), RequestError> {
        let inner = &self.inner;
        if inner
            .peers
            .grants(|grants, _| grants.current(&key).is_none())
        {
            return Err(RequestError::NotAGuest);
        }
        if inner.hosted_by(&key) {
            return Err(RequestError::Already);
        }
        {
            let mut state = lock(&inner.state);
            if state
                .requests
                .iter()
                .any(|request| request.key == key && request.state == RequestState::Waiting)
            {
                return Err(RequestError::Waiting);
            }
            state.requests.retain(|request| request.key != key);
            state.requests.push(RequestView {
                key,
                state: RequestState::Waiting,
            });
        }
        inner.publish();
        let inner = Arc::clone(inner);
        tokio::spawn(async move {
            let failed = inner.run_request(&key).await;
            {
                let mut state = lock(&inner.state);
                state.requests.retain(|request| request.key != key);
                if let Some(state_now) = failed {
                    state.requests.push(RequestView {
                        key,
                        state: state_now,
                    });
                }
            }
            inner.publish();
        });
        Ok(())
    }

    /// Clears what came of a request to `key` from the view.
    pub fn dismiss_request(&self, key: &PeerKey) {
        lock(&self.inner.state)
            .requests
            .retain(|request| request.key != *key || request.state == RequestState::Waiting);
        self.inner.publish();
    }

    /// Stops every task started here. The connections are [`Peers`]'s to
    /// close.
    pub fn close(&self) {
        self.inner.stop.send_replace(true);
        lock(&self.inner.keepers).clear();
        {
            let mut state = lock(&self.inner.state);
            state.offer = None;
            state.asks.clear();
            state.join = None;
        }
        self.inner.publish();
    }
}

impl<T: PeerTransport> Inner<T> {
    fn publish(&self) {
        let view = lock(&self.state).view();
        self.view.send_if_modified(|shown| {
            if *shown == view {
                return false;
            }
            *shown = view;
            true
        });
    }

    fn saved(&self) {
        self.saved.send_modify(|count| *count += 1);
    }

    /// Whether this machine is a guest of `key` in a way a new pairing
    /// would double: anything but a removal under way or a pairing the
    /// host dropped.
    fn hosted_by(&self, key: &PeerKey) -> bool {
        self.peers.host(key).is_some_and(|host| {
            !matches!(host.state, KnownState::RevokePending | KnownState::Unpaired)
        })
    }

    /// Whether this machine holds as many guests as it takes, `key` not
    /// counted: what the store refuses at the save, read before the
    /// operator is asked for nothing (B6).
    fn full_for(&self, key: &PeerKey) -> bool {
        self.peers.grants(|grants, now| {
            grants.list(now).filter(|grant| grant.key != *key).count() >= limits::MAX_GRANTS
        })
    }

    fn keep(&self, key: PeerKey) {
        // In place of the one before: that one may have stopped at a pause.
        lock(&self.keepers).insert(key, self.peers.keep(key));
    }

    fn unkeep(&self, key: &PeerKey) {
        lock(&self.keepers).remove(key);
    }

    /// Closes the connection to `key` when this machine gives it no grant
    /// either: nothing is left to run over it in either direction.
    fn leave_unless_hosting(&self, key: &PeerKey) {
        let hosting = matches!(
            self.peers.granting(key).state,
            GrantState::Pending | GrantState::Active | GrantState::Paused
        );
        if !hosting && let Some(link) = self.peers.link(key) {
            link.close(CloseCode::Shutdown);
        }
    }

    fn forget_host(&self, key: &PeerKey) -> Result<(), StoreError> {
        self.unkeep(key);
        self.peers.forget(key)?;
        self.saved();
        Ok(())
    }

    /// Saves `host` and starts keeping it connected. A host known under
    /// another key with the same device id is the same machine after it
    /// lost its key: its record goes, so one device is one host (B51).
    fn adopt_host(&self, host: Host) -> Result<(), StoreError> {
        let (key, device) = (host.key, host.device.clone());
        self.peers.known(|known| known.put(host))?;
        for earlier in self.peers.hosts() {
            if earlier.device == device
                && earlier.key != key
                && let Err(error) = self.forget_host(&earlier.key)
            {
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "pairing.earlier_host_kept",
                    "peer": earlier.key.short(),
                    "reason": error.to_string(),
                }));
            }
        }
        self.keep(key);
        self.saved();
        Ok(())
    }

    /// Follows what hosts say of the grants they give this machine: one
    /// that says it holds none any more is no longer dialed, and shows as
    /// a host to pair with again (B4, B51).
    async fn watch_grants(self: Arc<Self>, mut events: broadcast::Receiver<PeerEvent>) {
        let mut stop = self.stop.subscribe();
        loop {
            let event = tokio::select! {
                event = events.recv() => event,
                _ = stop.wait_for(|stop| *stop) => return,
            };
            match event {
                Ok(PeerEvent::Granted { key, state }) => self.heard_grant(&key, state),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // What was skipped is read from the connections. A
                    // connection that heard nothing yet says `None` too, so
                    // only a removal is taken from here.
                    for host in self.peers.hosts() {
                        let state = self.peers.link(&host.key).map(|link| link.granted());
                        if state == Some(GrantState::Revoked) {
                            self.heard_grant(&host.key, GrantState::Revoked);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    fn heard_grant(&self, key: &PeerKey, state: GrantState) {
        if !matches!(state, GrantState::Revoked | GrantState::None) {
            return;
        }
        let settled = self
            .peers
            .host(key)
            .is_some_and(|host| matches!(host.state, KnownState::Active | KnownState::PausedByMe));
        if !settled {
            return;
        }
        if let Err(error) = self.peers.set_host(key, KnownState::Unpaired) {
            crate::diagnostic!(json!({
                "component": "peer",
                "event": "pairing.unpaired_not_saved",
                "peer": key.short(),
                "reason": error.to_string(),
            }));
            return;
        }
        self.unkeep(key);
        self.leave_unless_hosting(key);
        self.saved();
    }

    // --- the host's side ------------------------------------------------

    async fn serve_pairing(self: Arc<Self>, link: Link<T::Conn>, stream: PeerStream<T::Conn>) {
        let key = link.key();
        let PeerStream { mut send, recv, .. } = stream;
        let mut heard = messages(recv);
        // The first message has ten seconds; a confirm may follow a grant
        // for as long as the grant waits.
        let mut within = limits::HELLO_WITHIN;
        loop {
            let Ok(Some(message)) = tokio::time::timeout(within, heard.recv()).await else {
                return;
            };
            within = limits::PENDING_FOR;
            let answer = match message {
                Message::Pair {
                    secret,
                    device,
                    name,
                    commit,
                } => {
                    let asked = Asked {
                        secret,
                        device,
                        name,
                        commit,
                    };
                    self.asked_by_link(&link, &mut send, &mut heard, asked)
                        .await
                }
                Message::Request { device, name } => {
                    self.asked_over_the_connection(&link, &mut heard, device, name)
                        .await
                }
                Message::Confirm { grant } => self.confirm_asked(&key, grant),
                // An answer where a question belongs.
                Message::Asking { .. }
                | Message::Reveal { .. }
                | Message::Granted { .. }
                | Message::Refused { .. }
                | Message::Confirmed { .. } => continue,
            };
            let Some(answer) = answer else {
                return;
            };
            if wire::write_message(&mut send, &answer).await.is_err() {
                return;
            }
        }
    }

    /// A guest shows a link's secret. `None` ends the stream unanswered.
    async fn asked_by_link<W>(
        &self,
        link: &Link<T::Conn>,
        send: &mut W,
        heard: &mut mpsc::Receiver<Message>,
        asked: Asked,
    ) -> Option<Message>
    where
        W: AsyncWrite + Unpin,
    {
        let Asked {
            secret,
            device,
            name,
            commit,
        } = asked;
        let key = link.key();
        let now = self.peers.now();
        // Whatever comes of it, the offer is over: a link is used once,
        // and one wrong secret ends it.
        let offer = lock(&self.state).offer.take();
        self.publish();
        let offer = offer.filter(|offer| {
            offer.expires_unix_ms > now
                && Secret::from_hex(&secret).is_some_and(|shown| offer.secret.matches(&shown))
        });
        // A guest that commits to no value cannot be held to one: its link
        // is spent like one with a wrong secret.
        let Some(offer) = offer.filter(|_| words::valid_commitment(&commit)) else {
            return refused(PairRefusal::NoOffer);
        };
        if device == self.me.device || !link::valid_device(&device) || !link::valid_name(&name) {
            return refused(PairRefusal::Other);
        }
        if self.full_for(&key) {
            return refused(PairRefusal::Full);
        }
        // This machine's value for the words is drawn now, with the guest
        // already bound to its own by `commit`, and the guest opens that
        // commitment before anyone here is asked: neither side chose its
        // value knowing the other's (`words`).
        let Ok(mine) = Nonce::random() else {
            return refused(PairRefusal::Other);
        };
        let asking = Message::Asking {
            device: self.me.device.clone(),
            name: self.me.name.clone(),
            words: offer.list.name().to_owned(),
            nonce: mine.to_hex().to_string(),
        };
        if wire::write_message(send, &asking).await.is_err() {
            return None;
        }
        // The one message that may follow, from a machine and not from a
        // person, so in the time a machine answers in.
        let theirs = match tokio::time::timeout(limits::HELLO_WITHIN, heard.recv()).await {
            Ok(Some(Message::Reveal { nonce })) => Nonce::from_hex(&nonce),
            // The guest left, or gave the pairing up before the words.
            Ok(None | Some(Message::Refused { .. })) => return None,
            _ => None,
        };
        // A value that is not the committed one is a guest choosing its
        // value after it saw this machine's. The link is spent either way.
        let host = self.peers.key();
        let Some(theirs) = theirs.filter(|theirs| words::opens(&commit, &host, &key, theirs))
        else {
            return refused(PairRefusal::NoOffer);
        };
        let words = words::of(offer.list, &host, &key, &theirs, &mine);
        let now = self.peers.now();
        let expires_unix_ms = (now + millis(limits::ANSWER_WITHIN)).min(offer.expires_unix_ms);
        let Some((id, said)) = self.ask(
            link,
            &device,
            &name,
            AskKind::Link,
            Some(words),
            expires_unix_ms,
        ) else {
            return refused(PairRefusal::Other);
        };
        let within = Duration::from_millis(expires_unix_ms.saturating_sub(now));
        match self.answered(id, said, heard, within).await {
            Said::Allow => self.grant(&key, &device, &name),
            Said::Deny => refused(PairRefusal::Declined),
            Said::Nobody => refused(PairRefusal::Unanswered),
            Said::Ended => refused(PairRefusal::NoOffer),
            Said::Left => None,
        }
    }

    /// A machine this one is a guest of asks to be let in the other way.
    async fn asked_over_the_connection(
        &self,
        link: &Link<T::Conn>,
        heard: &mut mpsc::Receiver<Message>,
        device: String,
        name: String,
    ) -> Option<Message> {
        let key = link.key();
        if !self.requests.admit(&key) {
            return None;
        }
        // Only a host this machine paired with, under the device id it
        // paired as: the connection proved its key, and the record says
        // whose key that is.
        let host = self.peers.host(&key).filter(|host| {
            matches!(host.state, KnownState::Active | KnownState::PausedByMe)
                && host.device == device
        });
        if host.is_none() || !link::valid_name(&name) {
            return refused(PairRefusal::Other);
        }
        if self.full_for(&key) {
            return refused(PairRefusal::Full);
        }
        let expires_unix_ms = self.peers.now() + millis(limits::ANSWER_WITHIN);
        let Some((id, said)) = self.ask(
            link,
            &device,
            &name,
            AskKind::Request,
            None,
            expires_unix_ms,
        ) else {
            return refused(PairRefusal::Other);
        };
        match self.answered(id, said, heard, limits::ANSWER_WITHIN).await {
            Said::Allow => self.grant(&key, &device, &name),
            Said::Deny => refused(PairRefusal::Declined),
            Said::Nobody => refused(PairRefusal::Unanswered),
            Said::Ended => refused(PairRefusal::Other),
            Said::Left => None,
        }
    }

    /// Puts a request before the operator. `None` when as many wait as may.
    fn ask(
        &self,
        link: &Link<T::Conn>,
        device: &str,
        name: &str,
        kind: AskKind,
        words: Option<[&'static str; words::COUNT]>,
        expires_unix_ms: u64,
    ) -> Option<(u64, oneshot::Receiver<bool>)> {
        let (answer, said) = oneshot::channel();
        let id = {
            let mut state = lock(&self.state);
            let key = link.key();
            // One request of a machine at a time: its newer one is the one
            // it means.
            state.asks.retain(|ask| ask.view.key != key);
            if state.asks.len() >= limits::MAX_ASKS {
                return None;
            }
            let id = state.id();
            state.asks.push(Ask {
                view: AskView {
                    id,
                    key,
                    device: device.to_owned(),
                    name: name.to_owned(),
                    kind,
                    words,
                    expires_unix_ms,
                },
                conn: link.id(),
                answer: Some(answer),
            });
            id
        };
        self.publish();
        Some((id, said))
    }

    fn drop_ask(&self, id: u64) {
        lock(&self.state).asks.retain(|ask| ask.view.id != id);
        self.publish();
    }

    /// Waits for the operator's answer to request `id`, for at most
    /// `within`, or for the machine that asked to leave.
    async fn answered(
        &self,
        id: u64,
        mut said: oneshot::Receiver<bool>,
        heard: &mut mpsc::Receiver<Message>,
        within: Duration,
    ) -> Said {
        let over = tokio::time::sleep(within);
        tokio::pin!(over);
        let said = loop {
            tokio::select! {
                said = &mut said => break match said {
                    Ok(true) => Said::Allow,
                    Ok(false) => Said::Deny,
                    Err(_) => Said::Ended,
                },
                () = &mut over => break Said::Nobody,
                message = heard.recv() => match message {
                    None | Some(Message::Refused { .. }) => break Said::Left,
                    Some(_) => {}
                },
            }
        };
        self.drop_ask(id);
        said
    }

    /// Saves the pending grant the operator allowed and answers it.
    fn grant(&self, key: &PeerKey, device: &str, name: &str) -> Option<Message> {
        match self.peers.grant_pending(*key, device, name, Via::Link) {
            Ok(grant) => {
                self.saved();
                Some(Message::Granted {
                    grant: grant.id,
                    device: self.me.device.clone(),
                    name: self.me.name.clone(),
                })
            }
            Err(StoreError::Full) => refused(PairRefusal::Full),
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "pairing.grant_not_saved",
                    "peer": key.short(),
                    "reason": error.to_string(),
                }));
                refused(PairRefusal::Other)
            }
        }
    }

    /// A guest's confirm of a grant, with or without an offer open.
    fn confirm_asked(&self, key: &PeerKey, grant: String) -> Option<Message> {
        if !self.confirms.admit(key) {
            return None;
        }
        match self.peers.confirm(key, &grant) {
            Ok(state) => {
                self.saved();
                Some(Message::Confirmed { grant, state })
            }
            Err(error) => {
                // Unanswered: the grant stays pending and the guest sends
                // its confirm again.
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "pairing.confirm_not_saved",
                    "peer": key.short(),
                    "reason": error.to_string(),
                }));
                None
            }
        }
    }

    /// The revoke stream: any machine may say "remove the grant you gave
    /// me". Only the grant that is that machine's and is the one named
    /// goes, and the answer is the same whatever was there.
    ///
    /// Removing a machine's last grant closes its connection, which may
    /// take the answer with it. The guest asks again on its next
    /// connection, and that time nothing changes and the answer arrives.
    async fn serve_revoke(self: Arc<Self>, link: Link<T::Conn>, stream: PeerStream<T::Conn>) {
        let key = link.key();
        let PeerStream {
            mut send, mut recv, ..
        } = stream;
        loop {
            let read = wire::read_message(&mut recv, MAX_MESSAGE);
            let Ok(Ok(Some(body))) = tokio::time::timeout(limits::HELLO_WITHIN, read).await else {
                return;
            };
            let grant = match Revoke::parse(&body) {
                Ok(Some(Revoke::Revoke { grant })) => grant,
                Ok(_) => continue,
                Err(_) => return,
            };
            if !self.revokes.admit(&key) {
                return;
            }
            if let Err(error) = self.peers.revoke(&key, Some(&grant)) {
                crate::diagnostic!(json!({
                    "component": "peer",
                    "event": "pairing.revoke_not_saved",
                    "peer": key.short(),
                    "reason": error.to_string(),
                }));
                return;
            }
            self.saved();
            if wire::write_message(&mut send, &Revoke::Revoked { grant })
                .await
                .is_err()
            {
                return;
            }
        }
    }

    // --- the guest's side -----------------------------------------------

    /// Changes the view of join `id`. `false` when that join is no longer
    /// the one shown: it was dismissed or replaced.
    fn join_shows(&self, id: u64, change: impl FnOnce(&mut JoinView)) -> bool {
        let shown = {
            let mut state = lock(&self.state);
            match state.join.as_mut().filter(|join| join.view.id == id) {
                Some(join) => {
                    change(&mut join.view);
                    true
                }
                None => false,
            }
        };
        self.publish();
        shown
    }

    async fn run_join(
        self: &Arc<Self>,
        id: u64,
        link: PairLink,
        mut said: oneshot::Receiver<bool>,
    ) -> JoinEnd {
        use JoinEnd::Failed;
        let host = link.host;
        let conn = match self
            .peers
            .dial(&PeerAddr::new(host, link.candidates()))
            .await
        {
            Ok(conn) => conn,
            Err(DialError::UpdateNeeded(_)) => return Failed(JoinFailure::UpdateNeeded),
            Err(_) => return Failed(JoinFailure::Unreachable),
        };
        let Ok(stream) = conn.open(StreamKind::Pairing).await else {
            return Failed(JoinFailure::Unreachable);
        };
        let PeerStream { mut send, recv, .. } = stream;
        let mut heard = messages(recv);
        // This machine's value for the words: the host gets only its
        // commitment until the host's own value is here (`words`).
        let Ok(mine) = Nonce::random() else {
            return Failed(JoinFailure::Unreachable);
        };
        let pair = Message::Pair {
            secret: link.secret.to_hex().to_string(),
            device: self.me.device.clone(),
            name: self.me.name.clone(),
            commit: words::commitment(&host, &self.peers.key(), &mine),
        };
        if wire::write_message(&mut send, &pair).await.is_err() {
            return Failed(JoinFailure::Unreachable);
        }
        let asking = tokio::time::timeout(limits::HELLO_WITHIN, heard.recv()).await;
        let (name, list, theirs) = match asking {
            Ok(Some(Message::Asking {
                device,
                name,
                words,
                nonce,
            })) => {
                // The host names the device its link named, or it is not
                // the host the link was for.
                if device != link.device || !link::valid_name(&name) {
                    return Failed(JoinFailure::Unreachable);
                }
                // A list this build does not carry is a newer host's.
                let Some(list) = WordList::from_name(&words) else {
                    return Failed(JoinFailure::UpdateNeeded);
                };
                let Some(theirs) = Nonce::from_hex(&nonce) else {
                    return Failed(JoinFailure::Unreachable);
                };
                (name, list, theirs)
            }
            Ok(Some(Message::Refused { reason })) => return Failed(reason.into()),
            _ => return Failed(JoinFailure::Unreachable),
        };
        let reveal = Message::Reveal {
            nonce: mine.to_hex().to_string(),
        };
        if wire::write_message(&mut send, &reveal).await.is_err() {
            return Failed(JoinFailure::Unreachable);
        }
        let words = words::of(list, &host, &self.peers.key(), &mine, &theirs);
        let expires = self.peers.now() + millis(limits::ANSWER_WITHIN);
        let shown = self.join_shows(id, |view| {
            view.phase = JoinPhase::Comparing;
            view.name = Some(name.clone());
            view.words = Some(words);
            view.expires_unix_ms = Some(expires);
        });

        // Both have to happen, in either order: this operator says the
        // words match, and the host's operator allows.
        let mut granted: Option<String> = None;
        let mut same = false;
        let over = tokio::time::sleep(limits::ANSWER_WITHIN);
        tokio::pin!(over);
        let failed = loop {
            if !shown {
                break Some(None);
            }
            if same && granted.is_some() {
                break None;
            }
            tokio::select! {
                answer = &mut said, if !same => match answer {
                    Ok(true) => {
                        same = true;
                        // The host's operator has its own two minutes.
                        over.as_mut().reset(
                            Instant::now() + limits::ANSWER_WITHIN + limits::HELLO_WITHIN,
                        );
                        self.join_shows(id, |view| {
                            view.phase = JoinPhase::Settling;
                            view.expires_unix_ms = None;
                        });
                    }
                    _ => break Some(None),
                },
                message = heard.recv() => match message {
                    Some(Message::Granted { grant, device, .. }) => {
                        if device != link.device {
                            break Some(Some(JoinFailure::Unreachable));
                        }
                        granted = Some(grant);
                    }
                    Some(Message::Refused { reason }) => break Some(Some(reason.into())),
                    // The words stay those of the first `asking`: a value
                    // the host sends once this machine's is revealed would
                    // be one it could have chosen knowing it.
                    Some(_) => {}
                    None => break Some(Some(JoinFailure::Unreachable)),
                },
                () = &mut over => break Some(Some(JoinFailure::Unanswered)),
            }
        };
        if let Some(failure) = failed {
            // Nothing is kept here. The host is told once; unheard, its
            // pending grant expires on its own.
            let declined = Message::Refused {
                reason: PairRefusal::Declined,
            };
            let _ = wire::write_message(&mut send, &declined).await;
            if let Some(grant) = granted {
                self.revoke_once(conn, grant);
            }
            return failure.map_or(JoinEnd::Dismissed, Failed);
        }
        let grant = granted.expect("the loop ended with a grant");
        let record = Host {
            key: host,
            device: link.device.clone(),
            name,
            chosen_name: None,
            grant: grant.clone(),
            state: KnownState::Confirming,
            candidates: link.candidates(),
            started: None,
            ssh: None,
        };
        if let Err(error) = self.adopt_host(record) {
            self.revoke_once(conn, grant);
            return Failed(match error {
                StoreError::Full => JoinFailure::Full,
                _ => JoinFailure::Unreachable,
            });
        }
        let confirm = Message::Confirm {
            grant: grant.clone(),
        };
        let confirmed = match wire::write_message(&mut send, &confirm).await {
            Ok(()) => tokio::time::timeout(limits::HELLO_WITHIN, heard.recv())
                .await
                .ok()
                .flatten(),
            Err(_) => None,
        };
        match confirmed {
            Some(Message::Confirmed {
                grant: theirs,
                state,
            }) if theirs == grant => match self.confirmed(&host, &grant, state) {
                Settled::Active => JoinEnd::Paired,
                Settled::Gone => Failed(JoinFailure::NoOffer),
                Settled::NotSaved => {
                    self.settle(host);
                    JoinEnd::Waiting
                }
            },
            _ => {
                self.settle(host);
                JoinEnd::Waiting
            }
        }
    }

    /// Tells a host once that the pending grant it just saved is not
    /// wanted.
    fn revoke_once(self: &Arc<Self>, conn: Link<T::Conn>, grant: String) {
        tokio::spawn(async move {
            if let Ok(stream) = conn.open(StreamKind::Revoke).await {
                let PeerStream {
                    mut send, mut recv, ..
                } = stream;
                if wire::write_message(&mut send, &Revoke::Revoke { grant })
                    .await
                    .is_ok()
                {
                    let read = wire::read_message(&mut recv, MAX_MESSAGE);
                    let _ = tokio::time::timeout(limits::HELLO_WITHIN, read).await;
                }
            }
        });
    }

    /// The host's answer to this machine's confirm of `grant`.
    fn confirmed(&self, host: &PeerKey, grant: &str, state: ConfirmState) -> Settled {
        let current = self
            .peers
            .host(host)
            .is_some_and(|known| known.grant == grant && known.state == KnownState::Confirming);
        if !current {
            // Paused or removed here meanwhile: that stands.
            return Settled::Active;
        }
        let to = match state {
            // A grant the host paused is settled all the same; the host's
            // hello says it is paused.
            ConfirmState::Active | ConfirmState::Paused => KnownState::Active,
            ConfirmState::Unknown => KnownState::Unpaired,
        };
        if let Err(error) = self.peers.set_host(host, to) {
            crate::diagnostic!(json!({
                "component": "peer",
                "event": "pairing.confirmed_not_saved",
                "peer": host.short(),
                "reason": error.to_string(),
            }));
            return Settled::NotSaved;
        }
        self.saved();
        if to == KnownState::Unpaired {
            self.unkeep(host);
            self.leave_unless_hosting(host);
            return Settled::Gone;
        }
        Settled::Active
    }

    /// Starts the task that sends host `key` this machine's confirm, or
    /// its removal, until the host answered. One task a host.
    fn settle(self: &Arc<Self>, key: PeerKey) {
        if lock(&self.state).settling.insert(key) {
            tokio::spawn(Arc::clone(self).settling(key));
        }
    }

    async fn settling(self: Arc<Self>, key: PeerKey) {
        let mut events = self.peers.events();
        let mut stop = self.stop.subscribe();
        let mut wait = limits::SETTLE_RETRY_FIRST;
        loop {
            let unsettled = |host: &Host| {
                matches!(
                    host.state,
                    KnownState::Confirming | KnownState::RevokePending
                )
            };
            let Some(host) = self.peers.host(&key).filter(unsettled) else {
                // Read again under the lock `settle` takes: a change made
                // meanwhile either finds this task or starts the next.
                let mut state = lock(&self.state);
                if self.peers.host(&key).is_some_and(|host| unsettled(&host)) {
                    continue;
                }
                state.settling.remove(&key);
                return;
            };
            let done = match host.state {
                KnownState::Confirming => self.send_confirm(&key, &host.grant).await,
                _ => self.send_revoke(&key, &host.grant).await,
            };
            if done {
                continue;
            }
            tokio::select! {
                () = tokio::time::sleep(wait) => {
                    wait = (wait * 2).min(limits::SETTLE_RETRY_MOST);
                }
                () = connected(&mut events, &key) => {}
                _ = stop.wait_for(|stop| *stop) => return,
            }
        }
    }

    /// Whether the host answered the confirm.
    async fn send_confirm(&self, key: &PeerKey, grant: &str) -> bool {
        let Some(link) = self.peers.link(key) else {
            return false;
        };
        let Ok(stream) = link.open(StreamKind::Pairing).await else {
            return false;
        };
        let PeerStream { mut send, recv, .. } = stream;
        let mut heard = messages(recv);
        let confirm = Message::Confirm {
            grant: grant.to_owned(),
        };
        if wire::write_message(&mut send, &confirm).await.is_err() {
            return false;
        }
        match tokio::time::timeout(limits::HELLO_WITHIN, heard.recv()).await {
            Ok(Some(Message::Confirmed {
                grant: theirs,
                state,
            })) if theirs == grant => {
                !matches!(self.confirmed(key, grant, state), Settled::NotSaved)
            }
            _ => false,
        }
    }

    /// Whether the host answered the removal, which forgets it here.
    async fn send_revoke(&self, key: &PeerKey, grant: &str) -> bool {
        let Some(link) = self.peers.link(key) else {
            return false;
        };
        let Ok(stream) = link.open(StreamKind::Revoke).await else {
            return false;
        };
        let PeerStream {
            mut send, mut recv, ..
        } = stream;
        let revoke = Revoke::Revoke {
            grant: grant.to_owned(),
        };
        if wire::write_message(&mut send, &revoke).await.is_err() {
            return false;
        }
        let read = wire::read_message(&mut recv, MAX_MESSAGE);
        let Ok(Ok(Some(body))) = tokio::time::timeout(limits::HELLO_WITHIN, read).await else {
            return false;
        };
        let answered = matches!(
            Revoke::parse(&body),
            Ok(Some(Revoke::Revoked { grant: theirs })) if theirs == grant
        );
        if !answered {
            return false;
        }
        // Only while it is still being removed: the operator may have
        // paired with the host again meanwhile.
        let removing = self
            .peers
            .host(key)
            .is_some_and(|host| host.grant == grant && host.state == KnownState::RevokePending);
        if removing && let Err(error) = self.forget_host(key) {
            crate::diagnostic!(json!({
                "component": "peer",
                "event": "pairing.host_not_forgotten",
                "peer": key.short(),
                "reason": error.to_string(),
            }));
            return false;
        }
        true
    }

    /// Asks guest `key` to let this machine in. `None` when it did.
    async fn run_request(self: &Arc<Self>, key: &PeerKey) -> Option<RequestState> {
        // The guest dials; this machine has no address to dial it at.
        let (Some(link), Some(expected)) = (self.peers.link(key), self.peers.identity(key)) else {
            return Some(RequestState::Unreachable);
        };
        let Ok(stream) = link.open(StreamKind::Pairing).await else {
            return Some(RequestState::Unreachable);
        };
        let PeerStream { mut send, recv, .. } = stream;
        let mut heard = messages(recv);
        let request = Message::Request {
            device: self.me.device.clone(),
            name: self.me.name.clone(),
        };
        if wire::write_message(&mut send, &request).await.is_err() {
            return Some(RequestState::Unreachable);
        }
        let within = limits::ANSWER_WITHIN + limits::HELLO_WITHIN;
        let (grant, name) = match tokio::time::timeout(within, heard.recv()).await {
            Ok(Some(Message::Granted {
                grant,
                device,
                name,
            })) if device == expected.device() && link::valid_name(&name) => (grant, name),
            Ok(Some(Message::Refused {
                reason: PairRefusal::Unanswered,
            }))
            | Err(_) => return Some(RequestState::Unanswered),
            Ok(Some(Message::Refused { .. })) => return Some(RequestState::Declined),
            _ => return Some(RequestState::Unreachable),
        };
        // Both machines attach to the same relay; the guest's own
        // addresses are found through it.
        let candidates = self
            .peers
            .transport()
            .candidates()
            .into_iter()
            .filter(|candidate| matches!(candidate, Candidate::Relay(_)))
            .collect();
        let record = Host {
            key: *key,
            device: expected.device().to_owned(),
            name,
            chosen_name: None,
            grant: grant.clone(),
            state: KnownState::Confirming,
            candidates,
            started: None,
            ssh: None,
        };
        if self.adopt_host(record).is_err() {
            self.revoke_once(link, grant);
            return Some(RequestState::Unreachable);
        }
        let confirm = Message::Confirm {
            grant: grant.clone(),
        };
        let confirmed = match wire::write_message(&mut send, &confirm).await {
            Ok(()) => tokio::time::timeout(limits::HELLO_WITHIN, heard.recv())
                .await
                .ok()
                .flatten(),
            Err(_) => None,
        };
        match confirmed {
            Some(Message::Confirmed {
                grant: theirs,
                state,
            }) if theirs == grant => {
                if matches!(self.confirmed(key, &grant, state), Settled::NotSaved) {
                    self.settle(*key);
                }
            }
            _ => self.settle(*key),
        }
        None
    }
}

impl From<PairRefusal> for JoinFailure {
    fn from(reason: PairRefusal) -> Self {
        match reason {
            PairRefusal::NoOffer => Self::NoOffer,
            PairRefusal::Declined => Self::Declined,
            PairRefusal::Full => Self::HostFull,
            PairRefusal::Unanswered => Self::Unanswered,
            // The host could not save, or says something a later build
            // would understand: nothing was made (B5).
            PairRefusal::Other => Self::Unreachable,
        }
    }
}
