//! The stream the two machines' cores talk on (PRD device-connect-iroh
//! D-62, D-72), and the one place this daemon handles it.
//!
//! A side that holds an active grant on the other opens a `core` stream
//! and both write: the letters one core's agents wrote to the other's, the
//! answer to each, and the reads and starts one asks of the other
//! (`hide_peer::protocol::core`). This daemon carries them and decides
//! nothing: who may write, read or start is the core's to judge from the
//! pairing it was told (`hide_core::access::core_admit`), and the letters
//! wait in the core's ledger, not here.
//!
//! What the two machines show of each other's work rides the same stream
//! (D-50, D-60): this machine's own Needs You and Done go to the machines
//! that may open it, as its list changes; this computer's seat goes to the
//! machines it may open, as it changes; a notification is handed to the
//! computer someone is at, which answers whether it showed it; and the
//! names this core shows its work on a machine it opens by go to that
//! machine, which keeps them with the panes (B10, `crate::work_names`). Which
//! way each goes is checked here by who opened a stream, which is who
//! holds a grant: the list and the place are the core's and
//! `desktop_notices`' to judge.
//!
//! What is kept here is memory of this run only: which letters were sent
//! and wait for their answer, so one is sent again after
//! [`RESEND_AFTER`] or on the next connection, and the calls that wait for
//! theirs. A letter sent twice is the same letter to the core that takes
//! it, by its sender and intent.
//!
//! A call a peer asks is this service's work, not its stream's: it holds
//! one of that peer's [`CALLS_AT_ONCE`] places until the core's work for
//! it ended and its answer was handed over, whatever became of the stream
//! it came on, and it ends with the daemon.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hide_core::agent_state::phone::Projection;
use hide_core::core_calls::{self, Agent, Asked, CoreItem, CoreLink, Letter, Outcome, Party};
use hide_core::notice_place::Seat;
use hide_peer::PeerEvent;
use hide_peer::protocol::core::{Ask, CoreMessage, Item, Work};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::stream::Status;
use hide_peer::protocol::{PeerKey, StreamKind, frame};
use hide_peer::wire::{MessageReader, write_message};
use serde_json::{Value, json};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio::task::JoinSet;

use crate::core::CoreHandle;
use crate::desktop_notices::{Computers, DesktopNotices};
use crate::ending::Ending;
use crate::peer_host::{HostPeers, Incoming, Link, PeerHost, Stream};

/// How long the core has to keep one letter or write one answer.
const LETTER_WITHIN: Duration = Duration::from_secs(5);
/// How long a letter waits for its answer before this daemon sends it again.
const RESEND_AFTER: Duration = Duration::from_secs(30);
/// How long a read or a pane send asked of another core may take, and a
/// start, which makes a worktree and waits for its agent.
const READ_WITHIN: Duration = Duration::from_secs(15);
const START_WITHIN: Duration = Duration::from_secs(180);
/// The messages one stream holds to be written.
const OUTBOUND: usize = 64;
/// The calls of one peer this daemon runs at once, over every stream that
/// peer opens and closes, and the calls of this machine's agents that wait
/// for an answer at once. A call past either is refused `capacity`.
const CALLS_AT_ONCE: usize = 4;
const CALLS_WAITING: usize = 64;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Each map is whole after any step of its own.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn record(kind: &str, key: &PeerKey, detail: Value) {
    hide_core::diagnostic!(json!({
        "component": "peer_core",
        "kind": kind,
        "peer": key.short(),
        "detail": detail,
    }));
}

fn party_out(party: &Party) -> hide_peer::protocol::core::Party {
    hide_peer::protocol::core::Party {
        item: match party.item {
            CoreItem::Agent => Item::Agent,
            CoreItem::Pane => Item::Pane,
        },
        id: party.id.clone(),
        name: party.name.clone(),
    }
}

fn party_in(party: hide_peer::protocol::core::Party) -> Party {
    Party {
        item: match party.item {
            Item::Agent => CoreItem::Agent,
            Item::Pane => CoreItem::Pane,
        },
        id: party.id,
        name: party.name,
    }
}

fn agent_out(agent: Agent) -> hide_peer::protocol::core::Agent {
    hide_peer::protocol::core::Agent {
        id: agent.id,
        name: agent.name,
    }
}

fn agent_in(agent: hide_peer::protocol::core::Agent) -> Agent {
    Agent {
        id: agent.id,
        name: agent.name,
    }
}

fn letter_out(letter: Letter) -> CoreMessage {
    CoreMessage::Letter {
        id: letter.id,
        from: agent_out(letter.from),
        to: letter.to,
        reply_to: letter.reply_to,
        kind: letter.kind,
        intent: letter.intent,
        body: letter.body,
        now: letter.now,
    }
}

/// What this daemon asks of the core for the other machines' cores. Each
/// call blocks on the core, so it runs on a blocking thread.
pub trait Cores: Send + Sync + 'static {
    /// The letters that wait to go to machine `key`, after the core wrote
    /// the end of those it cannot send.
    fn outbox(&self, key: &str) -> Vec<Letter>;
    /// Keeps a letter machine `key` sent: its outcome, or an error when
    /// this core's own store failed and the letter is to come again.
    fn take(&self, key: &str, letter: Letter) -> Result<Outcome, String>;
    /// Writes what machine `key` answered for letter `id`.
    fn answered(&self, key: &str, id: &str, outcome: Outcome) -> Result<(), String>;
    /// Runs a read or a start machine `key` asked for.
    fn call(&self, key: &str, from: Party, asked: Asked) -> Result<Value, String>;
    /// Hands the core what machine `key` told of its own work, whole, or
    /// that it tells no more: its stream ended. The core lists it only for
    /// a connected machine this one opens.
    fn work(&self, key: &str, work: Option<Value>) -> Result<(), String>;
}

impl Cores for CoreHandle {
    fn outbox(&self, key: &str) -> Vec<Letter> {
        self.core_outbox(key).settled(LETTER_WITHIN)
    }

    fn take(&self, key: &str, letter: Letter) -> Result<Outcome, String> {
        core_calls::take(self.prepare_core_letter(key, letter), LETTER_WITHIN)
    }

    fn answered(&self, key: &str, id: &str, outcome: Outcome) -> Result<(), String> {
        match self.core_letter_answered(key, id, outcome) {
            Some(settle) => settle.run(LETTER_WITHIN),
            None => Ok(()),
        }
    }

    fn call(&self, key: &str, from: Party, asked: Asked) -> Result<Value, String> {
        let within = match &asked {
            Asked::Pane(_) => READ_WITHIN,
            Asked::Delivery(_) => START_WITHIN,
        };
        self.prepare_core_call(key, from, asked)?.run(within)
    }

    fn work(&self, key: &str, work: Option<Value>) -> Result<(), String> {
        let event = json!({
            "schema_version": hide_core::SCHEMA_VERSION,
            "kind": "peer_work",
            "payload": {"key": key, "work": work},
        });
        self.dispatch(event.to_string().into_bytes())
    }
}

/// Where what a machine that may open this one told of its work here is
/// kept: that guest's panes (`Guests::name_panes`). Blocking.
pub trait WorkNamed: Send + Sync + 'static {
    fn named(&self, guest: &PeerKey, work: &HashMap<String, Work>);
    /// The guest's core tells no more: where its work stands is not known.
    fn away(&self, guest: &PeerKey);
}

impl WorkNamed for crate::guests::Guests {
    fn named(&self, guest: &PeerKey, work: &HashMap<String, Work>) {
        self.name_panes(guest, work);
    }

    fn away(&self, guest: &PeerKey) {
        self.guest_away(guest);
    }
}

/// The names of the work two machines run for each other (B10): what this
/// core shows its work on the machines it opens by, and where a guest's
/// names for its work here are kept.
pub struct Names {
    /// This core's names, each machine's by its connection key
    /// (`work_names::follow`).
    pub told: watch::Receiver<crate::work_names::Told>,
    /// Set while a stream this side opened is open: the names are read
    /// from the core only then.
    pub wanted: watch::Sender<bool>,
    /// Where a guest's names are kept; none where nothing is hosted.
    pub kept: Option<Arc<dyn WorkNamed>>,
}

/// This core's work on one machine as the `core` stream tells it: every
/// pane in `work`, and the named ones in `names` too, for a build of the
/// machine before `work` (B10).
fn names_out(names: &[hide_core::work_names::WorkName]) -> CoreMessage {
    CoreMessage::WorkNames {
        names: names
            .iter()
            .filter_map(|told| {
                told.name
                    .as_ref()
                    .map(|name| hide_peer::protocol::core::WorkName {
                        pane: told.pane.clone(),
                        name: name.clone(),
                    })
            })
            .collect(),
        work: names
            .iter()
            .map(|told| Work {
                pane: told.pane.clone(),
                name: told.name.clone(),
                branch: told.branch.clone(),
                mark: serde_json::to_value(told.mark)
                    .ok()
                    .and_then(|mark| mark.as_str().map(str::to_owned)),
                agent: told.agent.clone(),
            })
            .collect(),
    }
}

fn seat_out(seat: Seat) -> CoreMessage {
    let ms = |length: Duration| u64::try_from(length.as_millis()).unwrap_or(u64::MAX);
    CoreMessage::Seat {
        input_ms: seat.input.map(ms),
        shows: seat.shows,
        phone_ms: seat.phone.map(ms),
    }
}

/// One open stream to a peer: what is written to it.
struct Open {
    id: u64,
    out: mpsc::Sender<CoreMessage>,
    /// What this stream is to tell whole, the latest only.
    standing: Arc<Standing>,
    /// This side opened it.
    mine: bool,
}

/// What one stream tells whole and has not told yet: this machine's own
/// work, to a machine that may open it, and this computer's seat, to a
/// machine it may open (D-50, D-60). Each replaces what was told before, so
/// one mark each is all a stream holds: its writer reads the value as it
/// stands when the stream can send, and neither waits in the queue behind
/// letters nor is lost when the queue is full.
#[derive(Default)]
struct Standing {
    work: std::sync::atomic::AtomicBool,
    seat: std::sync::atomic::AtomicBool,
    /// This core's names of its work on the machine this side opens.
    names: std::sync::atomic::AtomicBool,
    moved: Notify,
}

impl Standing {
    fn tell(&self, due: &std::sync::atomic::AtomicBool) {
        due.store(true, Ordering::SeqCst);
        self.moved.notify_one();
    }
}

/// One place among a peer's calls at work, held until the call ended.
struct Place {
    service: Arc<PeerCore>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for Place {
    fn drop(&mut self) {
        // The place is free before the rooms are read again.
        self.permit = None;
        self.service.forget_idle_rooms();
    }
}

/// A call of this machine's agent that waits for the other core's answer.
struct Waiting {
    key: PeerKey,
    answer: std::sync::mpsc::SyncSender<Result<Value, String>>,
}

#[derive(Default)]
struct Heard {
    work: HashMap<PeerKey, Value>,
    unhanded: std::collections::BTreeSet<PeerKey>,
}

pub struct PeerCore {
    core: Arc<dyn Cores>,
    /// This computer's seat, the seats the others tell, and the notices
    /// handed between them.
    desktop: Arc<DesktopNotices>,
    /// What this machine last told of its own work, for a machine that
    /// connects later.
    told: Mutex<Option<Value>>,
    /// What each machine this one opens last told of its own work, and the
    /// machines whose latest the core has not been handed yet. Changed only
    /// under `open`, so a list and the end of the stream it came on cannot
    /// pass each other; it holds one list per machine with a stream this
    /// side opened and loses it with that stream.
    heard: Mutex<Heard>,
    /// A machine's told work changed, or it tells no more.
    heard_moved: Notify,
    shutdown: Ending,
    /// How long a letter waits for its answer before it is sent again.
    resend_after: Duration,
    /// The streams open to each peer; the first is written to.
    open: Mutex<HashMap<PeerKey, Vec<Open>>>,
    /// The peers this side is opening a stream to, or has one open to.
    opening: Mutex<Vec<PeerKey>>,
    /// The letters sent that wait for their answer, with when.
    sent: Mutex<HashMap<(PeerKey, String), Instant>>,
    calls: Mutex<HashMap<u64, Waiting>>,
    /// The places each peer's calls work in, kept past its streams: a peer
    /// that closes a stream and opens another has no more room than it had.
    /// Only a peer with a stream open or a call at work has an entry.
    rooms: Mutex<HashMap<PeerKey, Arc<Semaphore>>>,
    /// The calls at work for the peers, at most [`CALLS_AT_ONCE`] of each,
    /// ended with the daemon.
    working: Mutex<JoinSet<()>>,
    next: AtomicU64,
    /// The letters that wait to go changed, or a stream opened.
    moved: Notify,
    names: Names,
}

impl PeerCore {
    /// Takes the `core` streams peers open and follows every connection to
    /// open its own, until the daemon ends, and hands the core this
    /// connection to the others. Called once, before the endpoint opens.
    ///
    /// `desktop` is where this computer's seat and notices live, and
    /// `list` this machine's own list as the follower last read it.
    pub fn serve(
        core: &Arc<CoreHandle>,
        host: &Arc<PeerHost>,
        desktop: &Arc<DesktopNotices>,
        list: watch::Receiver<Arc<Projection>>,
        guests: &Arc<crate::guests::Guests>,
        shutdown: Ending,
    ) -> Arc<Self> {
        let (wanted, reading) = watch::channel(false);
        let names = Names {
            told: crate::work_names::follow(Arc::clone(core), reading, shutdown.clone()),
            wanted,
            kept: Some(Arc::clone(guests) as Arc<dyn WorkNamed>),
        };
        let service = Self::over(
            Arc::clone(core) as Arc<dyn Cores>,
            host,
            desktop,
            list,
            names,
            shutdown,
            RESEND_AFTER,
        );
        core.set_core_link(Arc::clone(&service) as Arc<dyn CoreLink>);
        service
    }

    fn over(
        core: Arc<dyn Cores>,
        host: &Arc<PeerHost>,
        desktop: &Arc<DesktopNotices>,
        list: watch::Receiver<Arc<Projection>>,
        names: Names,
        shutdown: Ending,
        resend_after: Duration,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            core,
            desktop: Arc::clone(desktop),
            told: Mutex::new(None),
            heard: Mutex::new(Heard::default()),
            heard_moved: Notify::new(),
            shutdown,
            resend_after,
            open: Mutex::new(HashMap::new()),
            opening: Mutex::new(Vec::new()),
            sent: Mutex::new(HashMap::new()),
            calls: Mutex::new(HashMap::new()),
            rooms: Mutex::new(HashMap::new()),
            working: Mutex::new(JoinSet::new()),
            next: AtomicU64::new(1),
            moved: Notify::new(),
            names,
        });
        let taking = Arc::clone(&service);
        host.handle(StreamKind::Core, move |incoming: Incoming| {
            let service = Arc::clone(&taking);
            async move {
                let key = incoming.link.key();
                service.session(key, incoming.stream, false).await;
            }
        });
        desktop.set_computers(Arc::clone(&service) as Arc<dyn Computers>);
        tokio::spawn(Arc::clone(&service).follow(Arc::clone(host)));
        tokio::spawn(Arc::clone(&service).pump());
        tokio::spawn(Arc::clone(&service).tell_work(list));
        tokio::spawn(Arc::clone(&service).tell_seat());
        tokio::spawn(Arc::clone(&service).hand_work());
        tokio::spawn(Arc::clone(&service).tell_names());
        service
    }

    /// Tells each machine this one opens the names of this core's work
    /// there, whole, whenever its own list reads differently (B10).
    async fn tell_names(self: Arc<Self>) {
        let mut told = self.names.told.clone();
        let mut last = crate::work_names::Told::default();
        loop {
            tokio::select! {
                () = self.shutdown.ended() => return,
                moved = told.changed() => if moved.is_err() { return },
            }
            let now = Arc::clone(&told.borrow_and_update());
            let open = lock(&self.open);
            for (key, streams) in open.iter() {
                let hex = key.to_hex();
                if now.get(&hex) == last.get(&hex) {
                    continue;
                }
                if let Some(stream) = streams.iter().find(|stream| stream.mine) {
                    stream.standing.tell(&stream.standing.names);
                }
            }
            drop(open);
            last = now;
        }
    }

    /// Whether a stream this side opened is open anywhere: the names are
    /// read from the core only while one is. Called under `open`.
    fn want_names(&self, open: &HashMap<PeerKey, Vec<Open>>) {
        let wanted = open.values().flatten().any(|stream| stream.mine);
        self.names.wanted.send_if_modified(|was| {
            let moved = *was != wanted;
            *was = wanted;
            moved
        });
    }

    /// Hands the core what each machine this one opens told of its own
    /// work, or that it tells no more: the latest only and one machine at
    /// a time, so the core hears a list and its stream's end in the order
    /// they happened, and a machine that tells faster than the core takes
    /// costs one list (D-50).
    async fn hand_work(self: Arc<Self>) {
        loop {
            let next = {
                let mut heard = lock(&self.heard);
                heard
                    .unhanded
                    .pop_first()
                    .map(|key| (key, heard.work.get(&key).cloned()))
            };
            let Some((key, work)) = next else {
                tokio::select! {
                    () = self.shutdown.ended() => return,
                    () = self.heard_moved.notified() => {}
                }
                continue;
            };
            let (core, hex) = (Arc::clone(&self.core), key.to_hex());
            let kept = tokio::task::spawn_blocking(move || core.work(&hex, work))
                .await
                .map_err(|error| error.to_string())
                .and_then(|kept| kept);
            if let Err(reason) = kept {
                record("peer_core.work_not_kept", &key, json!({"reason": reason}));
            }
        }
    }

    /// Tells the machines that may open this one its own Needs You and
    /// Done, whole, whenever the list reads differently (D-50).
    async fn tell_work(self: Arc<Self>, mut list: watch::Receiver<Arc<Projection>>) {
        loop {
            let told = hide_core::peer_work::told(&list.borrow_and_update());
            match serde_json::to_value(&told) {
                Ok(work) => {
                    let moved = {
                        let mut held = lock(&self.told);
                        let moved = held.as_ref() != Some(&work);
                        if moved {
                            *held = Some(work);
                        }
                        moved
                    };
                    // Marked after the list is held, so a stream that
                    // writes at its mark never reads an older list.
                    if moved {
                        self.tell_all(false, |standing| &standing.work);
                    }
                }
                Err(error) => hide_core::diagnostic!(json!({
                    "component": "peer_core", "kind": "peer_core.work_not_told",
                    "reason": error.to_string(),
                })),
            }
            tokio::select! {
                () = self.shutdown.ended() => return,
                moved = list.changed() => if moved.is_err() { return },
            }
        }
    }

    /// Tells the machines this computer may open its seat, as it changes
    /// (D-60): never per input, which `DesktopNotices` sees to.
    async fn tell_seat(self: Arc<Self>) {
        let mut moves = self.desktop.seat_moves();
        loop {
            tokio::select! {
                () = self.shutdown.ended() => return,
                moved = moves.changed() => if moved.is_err() { return },
            }
            self.tell_all(true, |standing| &standing.seat);
        }
    }

    /// Marks what `due` picks as to be told on every peer's stream `mine`
    /// says who opened: the machines this one may open, or the ones that
    /// may open it. Each stream tells it when it can send.
    fn tell_all(&self, mine: bool, due: impl Fn(&Standing) -> &std::sync::atomic::AtomicBool) {
        for streams in lock(&self.open).values() {
            if let Some(stream) = streams.iter().find(|stream| stream.mine == mine) {
                stream.standing.tell(due(&stream.standing));
            }
        }
    }

    /// Whether a stream to `key` is open that this side opened (`mine`),
    /// so this machine may open that one, or that the peer opened, so it
    /// may open this one.
    fn has(&self, key: &PeerKey, mine: bool) -> bool {
        lock(&self.open)
            .get(key)
            .is_some_and(|streams| streams.iter().any(|stream| stream.mine == mine))
    }

    /// Opens this side's stream on each connection under a grant the peer
    /// gave, as connections come and grants change.
    async fn follow(self: Arc<Self>, host: Arc<PeerHost>) {
        let Some(peers) = host.when_open().await else {
            return;
        };
        let mut events = peers.events();
        self.reach_all(&peers);
        loop {
            let event = tokio::select! {
                () = self.shutdown.ended() => return,
                event = events.recv() => event,
            };
            match event {
                Ok(PeerEvent::Connected { key, .. })
                | Ok(PeerEvent::Granted {
                    key,
                    state: GrantState::Active,
                }) => {
                    if let Some(link) = peers.link(&key) {
                        self.reach(link);
                    }
                }
                Ok(_) => {}
                // What was missed is read from the connections as they are.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => self.reach_all(&peers),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    fn reach_all(self: &Arc<Self>, peers: &HostPeers) {
        for host in peers.hosts() {
            if let Some(link) = peers.link(&host.key) {
                self.reach(link);
            }
        }
    }

    /// Opens this side's stream to the peer of `link`, unless it holds no
    /// active grant there or already has one.
    fn reach(self: &Arc<Self>, link: Link) {
        let key = link.key();
        if link.granted() != GrantState::Active {
            return;
        }
        {
            let mut opening = lock(&self.opening);
            if opening.contains(&key) {
                return;
            }
            opening.push(key);
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            match link.open(StreamKind::Core).await {
                Ok(stream) => Arc::clone(&service).session(key, stream, true).await,
                // A build from before this stream: its agents are reached
                // once it updates, and the next connection asks again.
                Err(hide_peer::peers::OpenError::Refused(Status::UnknownKind)) => {
                    record("peer_core.unknown_to_peer", &key, Value::Null);
                }
                Err(error) => record(
                    "peer_core.not_opened",
                    &key,
                    json!({"reason": error.to_string()}),
                ),
            }
            lock(&service.opening).retain(|held| *held != key);
        });
    }

    /// Reads and writes one stream until it ends.
    async fn session(self: Arc<Self>, key: PeerKey, stream: Stream, mine: bool) {
        let Stream { mut send, recv, .. } = stream;
        let mut reader = MessageReader::new(recv, frame::MAX_MESSAGE);
        let (out, mut outbound) = mpsc::channel(OUTBOUND);
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let standing = Arc::new(Standing::default());
        {
            let mut open = lock(&self.open);
            open.entry(key).or_default().push(Open {
                id,
                out: out.clone(),
                standing: Arc::clone(&standing),
                mine,
            });
            self.want_names(&open);
        }
        record("peer_core.opened", &key, json!({"mine": mine}));
        // What waited for this machine goes now, and it is told where this
        // one stands: this computer's seat for a machine it may open, this
        // machine's own work for one that may open it.
        self.moved.notify_one();
        standing.tell(if mine { &standing.seat } else { &standing.work });
        if mine {
            standing.tell(&standing.names);
        }
        let ended = 'stream: loop {
            tokio::select! {
                () = self.shutdown.ended() => break None,
                () = standing.moved.notified() => {
                    // Each as it stands now, read after its mark is taken:
                    // a change that lands meanwhile marks it again.
                    let work = standing.work.swap(false, Ordering::SeqCst);
                    let seat = standing.seat.swap(false, Ordering::SeqCst);
                    let names = standing.names.swap(false, Ordering::SeqCst);
                    let told = [
                        work.then(|| lock(&self.told).clone())
                            .flatten()
                            .map(|work| CoreMessage::Work { work }),
                        seat.then(|| seat_out(self.desktop.seat())),
                        // A machine whose session this core does not hold
                        // now keeps what it was told.
                        names
                            .then(|| self.names.told.borrow().get(&key.to_hex()).map(|names| names_out(names)))
                            .flatten(),
                    ];
                    for message in told.into_iter().flatten() {
                        if let Err(error) = write_message(&mut send, &message).await {
                            break 'stream Some(error.to_string());
                        }
                    }
                }
                message = reader.next() => match message {
                    Ok(Some(body)) => match CoreMessage::parse(&body) {
                        Ok(Some(message)) => {
                            let Some(answer) = self.take(key, message, &out).await else {
                                continue;
                            };
                            if let Err(error) = write_message(&mut send, &answer).await {
                                break Some(error.to_string());
                            }
                        }
                        // A message of a later build.
                        Ok(None) => {}
                        Err(error) => break Some(error.to_string()),
                    },
                    Ok(None) => break None,
                    Err(error) => break Some(error.to_string()),
                },
                Some(message) = outbound.recv() => {
                    if let Err(error) = write_message(&mut send, &message).await {
                        break Some(error.to_string());
                    }
                }
            }
        };
        self.closed(&key, id);
        record("peer_core.closed", &key, json!({"reason": ended}));
    }

    /// Forgets stream `id` of `key`. With its last stream gone, the
    /// letters sent to it are sent again on the next one, and the calls
    /// that waited on it are answered that it is away.
    fn closed(&self, key: &PeerKey, id: u64) {
        let (gone, opens_here, tells_no_more) = {
            let mut open = lock(&self.open);
            let Some(streams) = open.get_mut(key) else {
                return;
            };
            streams.retain(|stream| stream.id != id);
            let (gone, opens_here, opened) = (
                streams.is_empty(),
                streams.iter().any(|stream| !stream.mine),
                streams.iter().any(|stream| stream.mine),
            );
            if gone {
                open.remove(key);
            }
            self.want_names(&open);
            // A machine this one opens no longer tells nothing of its work.
            let tells_no_more = !opened && {
                let mut heard = lock(&self.heard);
                let told = heard.work.remove(key).is_some();
                if told {
                    heard.unhanded.insert(*key);
                }
                told
            };
            (gone, opens_here, tells_no_more)
        };
        // A machine that may open this one no longer has no seat here, and
        // no longer tells where its work here stands.
        if !opens_here {
            self.desktop.peer_gone(&key.to_hex());
            if let Some(kept) = &self.names.kept {
                let (kept, key) = (Arc::clone(kept), *key);
                tokio::spawn(async move {
                    if let Err(error) = tokio::task::spawn_blocking(move || kept.away(&key)).await {
                        record(
                            "peer_core.work_marks_not_dropped",
                            &key,
                            json!({"reason": error.to_string()}),
                        );
                    }
                });
            }
        }
        if tells_no_more {
            self.heard_moved.notify_one();
        }
        if gone {
            lock(&self.sent).retain(|(peer, _), _| peer != key);
            // Dropping the answer's sender ends the caller's wait.
            lock(&self.calls).retain(|_, waiting| waiting.key != *key);
            // The calls it asked keep their places until they end.
            self.forget_idle_rooms();
        }
    }

    /// A place among the calls of `key` at work, or none while
    /// [`CALLS_AT_ONCE`] of them still are, on whichever streams they came.
    fn place(self: &Arc<Self>, key: &PeerKey) -> Option<Place> {
        // Taken under the rooms' lock: a room found is held, or full,
        // before the rooms are read again, so none is forgotten between
        // being found and being held and counted anew beside its old self.
        let permit = {
            let mut rooms = lock(&self.rooms);
            let room = rooms
                .entry(*key)
                .or_insert_with(|| Arc::new(Semaphore::new(CALLS_AT_ONCE)));
            Arc::clone(room).try_acquire_owned().ok()
        }?;
        Some(Place {
            service: Arc::clone(self),
            permit: Some(permit),
        })
    }

    /// Drops the room of every peer that has no stream open and no call at
    /// work.
    fn forget_idle_rooms(&self) {
        let open: Vec<PeerKey> = lock(&self.open).keys().copied().collect();
        lock(&self.rooms)
            .retain(|peer, room| open.contains(peer) || room.available_permits() < CALLS_AT_ONCE);
    }

    /// Where a message to `key` is written: the stream this side opened,
    /// else the one the peer opened.
    fn out(&self, key: &PeerKey) -> Option<mpsc::Sender<CoreMessage>> {
        let open = lock(&self.open);
        let streams = open.get(key)?;
        streams
            .iter()
            .find(|stream| stream.mine)
            .or_else(|| streams.first())
            .map(|stream| stream.out.clone())
    }

    /// What one message of `key` asks of the core: the answer to write on
    /// the stream it came on now, when it has one.
    async fn take(
        self: &Arc<Self>,
        key: PeerKey,
        message: CoreMessage,
        out: &mpsc::Sender<CoreMessage>,
    ) -> Option<CoreMessage> {
        let hex = key.to_hex();
        match message {
            CoreMessage::Letter {
                id,
                from,
                to,
                reply_to,
                kind,
                intent,
                body,
                now,
            } => {
                let letter = Letter {
                    id: id.clone(),
                    from: agent_in(from),
                    to,
                    reply_to,
                    kind,
                    intent,
                    body,
                    now,
                };
                let core = Arc::clone(&self.core);
                let outcome = tokio::task::spawn_blocking(move || core.take(&hex, letter))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|outcome| outcome);
                match outcome {
                    Ok(Outcome::Taken { recipient }) => Some(CoreMessage::LetterTaken {
                        id,
                        recipient: agent_out(recipient),
                    }),
                    Ok(Outcome::Refused { reason }) => {
                        record(
                            "peer_core.letter_refused",
                            &key,
                            json!({"letter": id, "reason": reason}),
                        );
                        Some(CoreMessage::LetterRefused { id, reason })
                    }
                    // This core's own store failed: nothing is answered,
                    // and the letter comes again.
                    Err(code) => {
                        record(
                            "peer_core.letter_not_kept",
                            &key,
                            json!({"letter": id, "code": code}),
                        );
                        None
                    }
                }
            }
            CoreMessage::LetterTaken { id, recipient } => {
                self.answered(
                    key,
                    id,
                    Outcome::Taken {
                        recipient: agent_in(recipient),
                    },
                )
                .await;
                None
            }
            CoreMessage::LetterRefused { id, reason } => {
                self.answered(key, id, Outcome::Refused { reason }).await;
                None
            }
            CoreMessage::Call {
                call,
                from,
                ask,
                command,
            } => {
                let Some(place) = self.place(&key) else {
                    record("peer_core.call_refused", &key, json!({"code": "capacity"}));
                    return Some(CoreMessage::CallAnswer {
                        call,
                        answer: None,
                        refused: Some("capacity".to_owned()),
                    });
                };
                let core = Arc::clone(&self.core);
                let out = out.clone();
                let from = party_in(from);
                let mut working = lock(&self.working);
                // Those that ended are let go; the rest are at most the
                // places their peers hold.
                while working.try_join_next().is_some() {}
                working.spawn(async move {
                    // Held until the core's work ended and its answer was
                    // handed over, or found no stream to go on.
                    let _place = place;
                    let result = tokio::task::spawn_blocking(move || {
                        // A command this build cannot read is one of a
                        // later build's.
                        let asked = match ask {
                            Ask::Delivery => serde_json::from_value(command)
                                .map(|command| Asked::Delivery(Box::new(command))),
                            Ask::Pane => serde_json::from_value(command).map(Asked::Pane),
                        }
                        .map_err(|_| "update_needed".to_owned())?;
                        core.call(&hex, from, asked)
                    })
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|result| result);
                    let answer = match result {
                        Ok(answer) => CoreMessage::CallAnswer {
                            call,
                            answer: Some(answer),
                            refused: None,
                        },
                        Err(code) => CoreMessage::CallAnswer {
                            call,
                            answer: None,
                            refused: Some(code),
                        },
                    };
                    // An answer no frame holds is refused, not sent broken.
                    let answer = match frame::encode(&answer) {
                        Ok(_) => answer,
                        Err(_) => CoreMessage::CallAnswer {
                            call,
                            answer: None,
                            refused: Some("answer_too_large".to_owned()),
                        },
                    };
                    let _ = out.send(answer).await;
                });
                None
            }
            CoreMessage::CallAnswer {
                call,
                answer,
                refused,
            } => {
                let waiting = {
                    let mut calls = lock(&self.calls);
                    match calls.get(&call) {
                        // Only the machine that was asked answers.
                        Some(waiting) if waiting.key == key => calls.remove(&call),
                        _ => None,
                    }
                };
                if let Some(waiting) = waiting {
                    let result = match (answer, refused) {
                        (_, Some(code)) => Err(code),
                        (Some(answer), None) => Ok(answer),
                        (None, None) => Err("invalid_response".to_owned()),
                    };
                    let _ = waiting.answer.try_send(result);
                }
                None
            }
            // Its own work, from a machine this one may open.
            CoreMessage::Work { work } => {
                let kept = {
                    let open = lock(&self.open);
                    let opened = open
                        .get(&key)
                        .is_some_and(|streams| streams.iter().any(|stream| stream.mine));
                    if opened {
                        let mut heard = lock(&self.heard);
                        heard.work.insert(key, work);
                        heard.unhanded.insert(key);
                    }
                    opened
                };
                if kept {
                    self.heard_moved.notify_one();
                } else {
                    record("peer_core.work_refused", &key, Value::Null);
                }
                None
            }
            // Its seat, from a machine that may open this one.
            CoreMessage::Seat {
                input_ms,
                shows,
                phone_ms,
            } => {
                if self.has(&key, false) {
                    self.desktop.peer_seat(
                        &hex,
                        Seat {
                            input: input_ms.map(Duration::from_millis),
                            shows,
                            phone: phone_ms.map(Duration::from_millis),
                        },
                    );
                }
                None
            }
            // A notice of its work, from a machine this one may open.
            CoreMessage::Notice { id, notice } => Some(CoreMessage::NoticeAnswer {
                id,
                taken: self.has(&key, true) && self.desktop.show(&hex, &notice),
            }),
            CoreMessage::NoticeAnswer { id, taken } => {
                self.desktop.answered(&hex, id, taken);
                None
            }
            CoreMessage::NoticeClear { tags } => {
                if self.has(&key, true) {
                    self.desktop.withdraw(&hex, &tags);
                }
                None
            }
            // A phone paired with a machine this one may open (D-70); from a
            // machine that only opens this one it is nobody's to show here.
            CoreMessage::PhonePaired { phone } => {
                if self.has(&key, true) {
                    self.desktop.show_phone_paired(&hex, &phone);
                } else {
                    record("peer_core.phone_paired_refused", &key, Value::Null);
                }
                None
            }
            // The names of a machine's work here, from a machine that may
            // open this one (B10): only its own panes take them.
            // A build before `work` tells `names` alone, and its panes
            // are named without a branch or a mark.
            CoreMessage::WorkNames { names, work } => {
                let refused = if !self.has(&key, false) {
                    Some("not_opened_by_it")
                } else if !hide_peer::protocol::core::valid_work_names(&names)
                    || !hide_peer::protocol::core::valid_work(&work)
                {
                    Some("out_of_bounds")
                } else {
                    None
                };
                if let Some(reason) = refused {
                    record(
                        "peer_core.work_names_refused",
                        &key,
                        json!({"reason": reason, "names": names.len(), "work": work.len()}),
                    );
                    return None;
                }
                if let Some(kept) = &self.names.kept {
                    let kept = Arc::clone(kept);
                    let work: HashMap<String, Work> = if work.is_empty() {
                        names
                            .into_iter()
                            .map(|named| {
                                let told = Work {
                                    pane: named.pane.clone(),
                                    name: Some(named.name),
                                    branch: None,
                                    mark: None,
                                    agent: None,
                                };
                                (named.pane, told)
                            })
                            .collect()
                    } else {
                        work.into_iter()
                            .map(|told| (told.pane.clone(), told))
                            .collect()
                    };
                    if let Err(error) =
                        tokio::task::spawn_blocking(move || kept.named(&key, &work)).await
                    {
                        record(
                            "peer_core.work_names_not_kept",
                            &key,
                            json!({"reason": error.to_string()}),
                        );
                    }
                }
                None
            }
        }
    }

    /// Hands the core what `key` answered for letter `id`.
    async fn answered(&self, key: PeerKey, id: String, outcome: Outcome) {
        let core = Arc::clone(&self.core);
        let hex = key.to_hex();
        let letter = id.clone();
        let written = tokio::task::spawn_blocking(move || core.answered(&hex, &letter, outcome))
            .await
            .map_err(|error| error.to_string())
            .and_then(|written| written);
        if let Err(code) = written {
            record(
                "peer_core.answer_not_written",
                &key,
                json!({"letter": id, "code": code}),
            );
        }
        // Answered, or not written and so still waiting: either way the
        // next pass reads the ledger.
        lock(&self.sent).remove(&(key, id));
        self.moved.notify_one();
    }

    /// Sends what waits in the core's ledger for each connected peer,
    /// whenever that may have changed, and again what got no answer.
    async fn pump(self: Arc<Self>) {
        loop {
            let again = !lock(&self.sent).is_empty();
            tokio::select! {
                () = self.shutdown.ended() => {
                    // The calls at work end with the daemon.
                    lock(&self.working).abort_all();
                    return;
                }
                () = self.moved.notified() => {}
                () = tokio::time::sleep(self.resend_after), if again => {}
            }
            let peers: Vec<PeerKey> = lock(&self.open).keys().copied().collect();
            for key in peers {
                self.send_waiting(key).await;
            }
        }
    }

    async fn send_waiting(&self, key: PeerKey) {
        let core = Arc::clone(&self.core);
        let hex = key.to_hex();
        let Ok(letters) = tokio::task::spawn_blocking(move || core.outbox(&hex)).await else {
            return;
        };
        let Some(out) = self.out(&key) else {
            return;
        };
        let mut sent = lock(&self.sent);
        // A letter no longer waiting was answered or ended.
        sent.retain(|(peer, id), _| *peer != key || letters.iter().any(|letter| letter.id == *id));
        for letter in letters {
            let name = (key, letter.id.clone());
            if sent
                .get(&name)
                .is_some_and(|at| at.elapsed() < self.resend_after)
            {
                continue;
            }
            // A full stream sends it at the next pass.
            if out.try_send(letter_out(letter)).is_ok() {
                sent.insert(name, Instant::now());
            } else {
                // Due at once, and the wait for it keeps the pump turning.
                sent.entry(name)
                    .or_insert_with(|| Instant::now() - self.resend_after);
            }
        }
    }
}

impl Computers for PeerCore {
    fn tell(&self, key: &str, message: CoreMessage) -> bool {
        PeerKey::from_hex(key)
            .ok()
            .and_then(|key| self.out(&key))
            .is_some_and(|out| out.try_send(message).is_ok())
    }
}

impl CoreLink for PeerCore {
    fn call(
        &self,
        key: &str,
        from: &Party,
        asked: &Asked,
        within: Duration,
    ) -> Result<Value, String> {
        let key = PeerKey::from_hex(key).map_err(|_| "machine_unavailable".to_owned())?;
        let out = self.out(&key).ok_or("machine_unavailable")?;
        let (ask, command, within) = match asked {
            Asked::Delivery(command) => (
                Ask::Delivery,
                serde_json::to_value(command),
                // A start outlasts the wait a letter's caller gives, and a
                // read crosses two machines.
                match **command {
                    hide_core::delivery::Command::Agents {
                        command: hide_core::coordination::Command::Spawn { .. },
                    } => START_WITHIN,
                    _ => within.max(READ_WITHIN),
                },
            ),
            Asked::Pane(command) => (
                Ask::Pane,
                serde_json::to_value(command),
                within.max(READ_WITHIN),
            ),
        };
        let command = command.map_err(|_| "invalid_request".to_owned())?;
        let call = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = std::sync::mpsc::sync_channel(1);
        {
            let mut calls = lock(&self.calls);
            if calls.len() >= CALLS_WAITING {
                return Err("capacity".into());
            }
            calls.insert(call, Waiting { key, answer });
        }
        let message = CoreMessage::Call {
            call,
            from: party_out(from),
            ask,
            command,
        };
        if out.try_send(message).is_err() {
            lock(&self.calls).remove(&call);
            return Err("machine_unavailable".into());
        }
        match answered.recv_timeout(within) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.calls).remove(&call);
                Err("delivery_timeout".into())
            }
            // The connection ended before the answer came.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("machine_unavailable".into())
            }
        }
    }

    fn outbox_moved(&self) {
        self.moved.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use hide_core::coordination::Command as Agents;
    use hide_core::delivery::Command as Delivery;
    use tokio::sync::watch;

    use super::*;
    use crate::peer_host::fixture::{Machine, paired, within};

    /// A core as this daemon sees one: what waits to go, what it was
    /// handed, and the answers the test chose for it.
    #[derive(Default)]
    struct Core {
        waiting: Mutex<Vec<(String, Letter)>>,
        taken: Mutex<Vec<Letter>>,
        /// The answers to the next letters; a letter past them is taken.
        takes: Mutex<VecDeque<Result<Outcome, String>>>,
        answers: Mutex<Vec<(String, Outcome)>>,
        asked: Mutex<Vec<(Party, Value)>>,
        /// The answers to the next calls; a call past them waits for the
        /// test to let it go, by dropping the other end of `held`.
        calls: Mutex<VecDeque<Result<Value, String>>>,
        held: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        /// What each machine told of its own work, as it arrived; none
        /// once its stream ended.
        works: Mutex<Vec<(String, Option<Value>)>>,
        /// Holds the next letter in this core until the test lets it go.
        keeping: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        seen: watch::Sender<u64>,
    }

    impl Core {
        fn moved(&self) {
            self.seen.send_modify(|count| *count += 1);
        }

        /// Waits until `met` holds of what this core was handed.
        async fn until(&self, what: &str, met: impl Fn(&Self) -> bool) {
            let mut seen = self.seen.subscribe();
            within(what, seen.wait_for(|_| met(self))).await.unwrap();
        }
    }

    impl Cores for Core {
        fn outbox(&self, key: &str) -> Vec<Letter> {
            lock(&self.waiting)
                .iter()
                .filter(|(to, _)| to == key)
                .map(|(_, letter)| letter.clone())
                .collect()
        }

        fn take(&self, _key: &str, letter: Letter) -> Result<Outcome, String> {
            let answer = lock(&self.takes).pop_front().unwrap_or_else(|| {
                Ok(Outcome::Taken {
                    recipient: agent("agent-9", "reviewer"),
                })
            });
            lock(&self.taken).push(letter);
            self.moved();
            let keeping = lock(&self.keeping).take();
            if let Some(keeping) = keeping {
                let _ = keeping.recv();
            }
            answer
        }

        fn answered(&self, key: &str, id: &str, outcome: Outcome) -> Result<(), String> {
            // An answered letter waits no longer, as in the ledger.
            lock(&self.waiting).retain(|(to, letter)| to != key || letter.id != id);
            lock(&self.answers).push((id.to_owned(), outcome));
            self.moved();
            Ok(())
        }

        fn call(&self, _key: &str, from: Party, asked: Asked) -> Result<Value, String> {
            let command = match asked {
                Asked::Delivery(command) => serde_json::to_value(command).unwrap(),
                Asked::Pane(command) => serde_json::to_value(command).unwrap(),
            };
            lock(&self.asked).push((from, command));
            self.moved();
            let answer = lock(&self.calls).pop_front();
            answer.unwrap_or_else(|| {
                // Every held call waits here, one behind the other.
                let held = lock(&self.held);
                let _ = held.as_ref().expect("a held call").recv();
                Err("held".into())
            })
        }

        fn work(&self, key: &str, work: Option<Value>) -> Result<(), String> {
            lock(&self.works).push((key.to_owned(), work));
            self.moved();
            Ok(())
        }
    }

    fn party(id: &str, name: &str) -> Party {
        Party {
            item: CoreItem::Agent,
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    fn agent(id: &str, name: &str) -> Agent {
        Agent {
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }

    fn letter(id: &str) -> Letter {
        Letter {
            id: id.to_owned(),
            from: agent("agent-1", "worker"),
            to: "reviewer".to_owned(),
            reply_to: None,
            kind: "request".to_owned(),
            intent: format!("intent-{id}"),
            body: "Read the diff.".to_owned(),
            now: false,
        }
    }

    /// Two machines, each a core behind its own daemon's service: `mac`
    /// holds a grant on `pc`, so it is the one that opens the stream.
    struct Pair {
        mac: Machine,
        pc: Machine,
        mac_core: Arc<Core>,
        pc_core: Arc<Core>,
        mac_service: Arc<PeerCore>,
        pc_service: Arc<PeerCore>,
        mac_desktop: Arc<DesktopNotices>,
        pc_desktop: Arc<DesktopNotices>,
        /// The PC's own list, as its follower would read it.
        pc_list: watch::Sender<Arc<Projection>>,
        /// The names the Mac's core shows its work on other machines by.
        mac_names: watch::Sender<crate::work_names::Told>,
        /// Whether each side reads its names, and what each side kept of
        /// the names another machine told it.
        mac_wants: watch::Receiver<bool>,
        pc_wants: watch::Receiver<bool>,
        mac_kept: Arc<Kept>,
        pc_kept: Arc<Kept>,
        mac_key: String,
        pc_key: String,
    }

    /// What one side kept of the work a guest told it, in order, how many
    /// lists it has kept, for a test to wait on, and which guests went
    /// away.
    struct Kept(
        Mutex<Vec<(String, HashMap<String, Work>)>>,
        watch::Sender<usize>,
        watch::Sender<Vec<String>>,
    );

    impl Default for Kept {
        fn default() -> Self {
            Self(
                Mutex::default(),
                watch::channel(0).0,
                watch::channel(Vec::new()).0,
            )
        }
    }

    impl WorkNamed for Kept {
        fn named(&self, guest: &PeerKey, work: &HashMap<String, Work>) {
            lock(&self.0).push((guest.to_hex(), work.clone()));
            self.1.send_modify(|count| *count += 1);
        }

        fn away(&self, guest: &PeerKey) {
            self.2.send_modify(|away| away.push(guest.to_hex()));
        }
    }

    impl Pair {
        async fn new(resend_after: Duration) -> Self {
            Self::with(resend_after, Arc::new(DesktopNotices::new())).await
        }

        /// The pair with the Mac's seat and windows as `mac_desktop` has
        /// them before the two connect.
        async fn with(resend_after: Duration, mac_desktop: Arc<DesktopNotices>) -> Self {
            let (mac, pc) = (Machine::new("macbook"), Machine::new("windows-pc"));
            let (mac_core, pc_core) = (Arc::new(Core::default()), Arc::new(Core::default()));
            let pc_desktop = Arc::new(DesktopNotices::new());
            let (pc_list, _) = watch::channel(Arc::<Projection>::default());
            let (mac_names, _) = watch::channel(crate::work_names::Told::default());
            let (mac_kept, pc_kept) = (Arc::new(Kept::default()), Arc::new(Kept::default()));
            let names = |told: watch::Receiver<crate::work_names::Told>, kept: &Arc<Kept>| {
                let (wanted, wants) = watch::channel(false);
                let names = Names {
                    told,
                    wanted,
                    kept: Some(Arc::clone(kept) as Arc<dyn WorkNamed>),
                };
                (names, wants)
            };
            let (mac_naming, mac_wants) = names(mac_names.subscribe(), &mac_kept);
            let (pc_naming, pc_wants) = names(watch::channel(Arc::default()).1, &pc_kept);
            let serve = |machine: &Machine,
                         core: &Arc<Core>,
                         desktop: &Arc<DesktopNotices>,
                         list: watch::Receiver<Arc<Projection>>,
                         names: Names| {
                PeerCore::over(
                    Arc::clone(core) as Arc<dyn Cores>,
                    &machine.host,
                    desktop,
                    list,
                    names,
                    machine.shutdown.clone(),
                    resend_after,
                )
            };
            let mac_service = serve(
                &mac,
                &mac_core,
                &mac_desktop,
                watch::channel(Arc::default()).1,
                mac_naming,
            );
            let pc_service = serve(&pc, &pc_core, &pc_desktop, pc_list.subscribe(), pc_naming);
            let link = paired(&pc, &mac).await;
            let pc_key = link.key().to_hex();
            let mac_key = mac.host.peers().unwrap().key().to_hex();
            Self {
                mac,
                pc,
                mac_core,
                pc_core,
                mac_service,
                pc_service,
                mac_desktop,
                pc_desktop,
                pc_list,
                mac_names,
                mac_wants,
                pc_wants,
                mac_kept,
                pc_kept,
                mac_key,
                pc_key,
            }
        }
    }

    fn list(group: &str, demand: &str) -> Arc<Projection> {
        Arc::new(hide_core::agent_state::phone::project(
            &json!({
                "navigator": {
                    "agents": [{
                        "pane_id": "w1:p1", "group": group, "mark": "going",
                        "identity_label": "Fix login", "agent_kind": "claude",
                        "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle",
                        "demand": demand,
                    }],
                    "workspaces": [{"label": "shop-api", "is_git": true, "checkouts": [
                        {"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "w1:p1"}]}]},
                    ]}],
                    "devices": [{"id": "node", "label": null}],
                },
            }),
            "node",
        ))
    }

    /// D-50 at the stream's cap: a list that changed while the stream held
    /// all it may hold to be written is told when the stream can send
    /// again, with no further change to bring it.
    #[tokio::test]
    async fn a_list_that_changed_while_the_stream_was_full_is_told_when_it_can_send() {
        let pair = Pair::new(Duration::from_secs(30)).await;
        pair.mac_core
            .until("the PC's first list on the Mac", |core| {
                !lock(&core.works).is_empty()
            })
            .await;
        // The PC's core holds a letter, so its stream writes nothing, and
        // the stream's queue fills.
        let (release, keeping) = std::sync::mpsc::channel();
        *lock(&pair.pc_core.keeping) = Some(keeping);
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-1")));
        pair.mac_service.outbox_moved();
        pair.pc_core
            .until("the letter in the PC's core", |core| {
                !lock(&core.taken).is_empty()
            })
            .await;
        // An answer to a call nobody waits for is read and dropped.
        let filler = || CoreMessage::CallAnswer {
            call: u64::MAX,
            answer: None,
            refused: Some("capacity".to_owned()),
        };
        let mut queued = 0;
        while Computers::tell(&*pair.pc_service, &pair.mac_key, filler()) {
            queued += 1;
            assert!(queued <= OUTBOUND, "the queue is bounded");
        }
        assert_eq!(queued, OUTBOUND);

        // The PC's list changes once while nothing more can be queued.
        let latest = list("needs_you", "question");
        let told = serde_json::to_value(hide_core::peer_work::told(&latest)).unwrap();
        pair.pc_list.send_replace(latest);
        within("the PC's changed list held to be told", async {
            while lock(&pair.pc_service.told).as_ref() != Some(&told) {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            lock(&pair.mac_core.works)
                .iter()
                .all(|(_, work)| work.as_ref() != Some(&told))
        );

        release.send(()).unwrap();
        pair.mac_core
            .until("the PC's latest list on the Mac", |core| {
                lock(&core.works).last() == Some(&(pair.pc_key.clone(), Some(told.clone())))
            })
            .await;
    }

    /// PRD device-connect-iroh D-50, D-60, B71: the machine that may open
    /// another is told that machine's own Needs You, tells it its seat, and
    /// while someone is at it shows that machine's notice and closes it;
    /// nothing of the kind goes the other way, where no grant is held.
    #[tokio::test]
    async fn the_machine_that_opens_another_gets_its_work_and_shows_its_notice_while_in_use() {
        // Someone is at the Mac, whose window shows notifications.
        let mac_desktop = Arc::new(DesktopNotices::new());
        mac_desktop.set(1, true);
        mac_desktop.set_waiting(1, true);
        mac_desktop.activity(1, true);
        let mut windows = mac_desktop.subscribe();
        let pair = Pair::with(Duration::from_secs(30), mac_desktop).await;
        within("the Mac's seat on the PC", pair.pc_desktop.judge_wanted()).await;
        assert!(pair.pc_desktop.judging() && pair.pc_desktop.rests_phone());

        // The PC's follower reads a list in which nothing waits, then one
        // whose agent asks.
        let at = Instant::now();
        pair.pc_list.send_replace(list("working", "none"));
        pair.pc_desktop.observe(&list("working", "none"), at);
        pair.pc_list.send_replace(list("needs_you", "question"));
        pair.mac_core
            .until("the PC's Needs You on the Mac", |core| {
                lock(&core.works).last().is_some_and(|(_, work)| {
                    work.as_ref().is_some_and(|work| work["needs_you"] == 1)
                })
            })
            .await;
        let (from, work) = lock(&pair.mac_core.works).last().cloned().unwrap();
        assert_eq!(from, pair.pc_key);
        assert_eq!(work.unwrap()["rows"][0]["title"], "Fix login");

        assert_eq!(
            pair.pc_desktop.observe(&list("needs_you", "question"), at),
            [],
            "the Mac is the place, not the phones"
        );
        let frame: Value = serde_json::from_str(
            &within("the notice on the Mac", windows.recv())
                .await
                .unwrap(),
        )
        .unwrap();
        let tag = format!("peer:{}|node|w1:p1", pair.pc_key);
        assert_eq!(frame["payload"]["notices"][0]["tag"], tag);
        assert_eq!(frame["payload"]["notices"][0]["peer"], pair.pc_key);
        assert_eq!(frame["payload"]["notices"][0]["place"], "shop-api");

        pair.pc_desktop.observe(&list("seen", "none"), at);
        let frame: Value = serde_json::from_str(
            &within("the close on the Mac", windows.recv())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(frame["payload"]["clear"], json!([tag]));

        // The PC may not open the Mac: its seat is not kept there, and
        // the Mac's work is not kept on the PC. Each is followed on the
        // same stream by a message that is taken, so it was read.
        assert!(pair.pc_service.tell(
            &pair.mac_key,
            seat_out(Seat {
                input: Some(Duration::ZERO),
                shows: true,
                phone: None,
            })
        ));
        lock(&pair.pc_core.waiting).push((pair.mac_key.clone(), letter("letter-1")));
        pair.pc_service.outbox_moved();
        pair.mac_core
            .until("the PC's letter", |core| !lock(&core.taken).is_empty())
            .await;
        assert!(!pair.mac_desktop.has_computers());

        assert!(pair.mac_service.tell(
            &pair.pc_key,
            CoreMessage::Work {
                work: json!({"needs_you": 3, "done": 0}),
            }
        ));
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-2")));
        pair.mac_service.outbox_moved();
        pair.pc_core
            .until("the Mac's letter", |core| !lock(&core.taken).is_empty())
            .await;
        assert_eq!(*lock(&pair.pc_core.works), []);

        // The connection ends: the Mac's core is told the PC tells no more.
        pair.pc.shutdown.end();
        pair.mac_core
            .until("the PC's work forgotten on the Mac", |core| {
                lock(&core.works).last() == Some(&(pair.pc_key.clone(), None))
            })
            .await;
    }

    /// B10: the Mac, which opens the PC, reads its names while it is
    /// connected there and tells the PC the names of its work on the PC,
    /// whole, on connecting and whenever they read differently, never
    /// another machine's; the PC keeps them with its panes. The same message
    /// from the PC, which the Mac does not open, is not kept on the Mac.
    #[tokio::test]
    async fn the_names_of_a_machines_work_go_to_the_machine_it_runs_on_and_not_the_other_way() {
        let named = |pairs: &[(&str, &str)]| -> Vec<hide_core::work_names::WorkName> {
            pairs
                .iter()
                .map(|(pane, name)| hide_core::work_names::WorkName {
                    pane: (*pane).to_owned(),
                    name: Some((*name).to_owned()),
                    branch: None,
                    mark: hide_core::agent_state::RowMark::Quiet,
                    agent: Some("claude".to_owned()),
                })
                .collect()
        };
        let mut pair = Pair::new(Duration::from_secs(30)).await;
        within(
            "the Mac reads its names",
            pair.mac_wants.wait_for(|wants| *wants),
        )
        .await
        .unwrap();
        assert!(!*pair.pc_wants.borrow(), "the PC opens nothing");

        let kept = |pair: &Pair| lock(&pair.pc_kept.0).clone();
        let until = |pair: &Pair, count: usize| {
            let mut kept = pair.pc_kept.1.subscribe();
            async move {
                within("the names on the PC", kept.wait_for(|told| *told >= count))
                    .await
                    .unwrap();
            }
        };
        let mut told = std::collections::BTreeMap::new();
        told.insert(pair.pc_key.clone(), named(&[("w1:p1", "로그인 고치기")]));
        told.insert("ef".repeat(32), named(&[("w9:p9", "another machine's")]));
        pair.mac_names.send_replace(Arc::new(told.clone()));
        until(&pair, 1).await;
        let (from, names) = kept(&pair).remove(0);
        assert_eq!(from, pair.mac_key);
        assert_eq!(
            names,
            HashMap::from([(
                "w1:p1".to_owned(),
                Work {
                    pane: "w1:p1".to_owned(),
                    name: Some("로그인 고치기".to_owned()),
                    branch: None,
                    mark: Some("quiet".to_owned()),
                    agent: Some("claude".to_owned()),
                }
            )])
        );

        // Another machine's names moving tells the PC nothing; its own do.
        told.insert("ef".repeat(32), named(&[]));
        pair.mac_names.send_replace(Arc::new(told.clone()));
        told.insert(
            pair.pc_key.clone(),
            named(&[("w1:p1", "로그인 고치기"), ("w1:p2", "배포 확인")]),
        );
        pair.mac_names.send_replace(Arc::new(told));
        until(&pair, 2).await;
        assert_eq!(kept(&pair).len(), 2);
        assert_eq!(kept(&pair)[1].1.len(), 2);

        // The PC may not open the Mac: names it tells are not kept there.
        // A letter follows on the same stream, so the names were read.
        assert!(
            pair.pc_service
                .tell(&pair.mac_key, names_out(&named(&[("w1:p1", "theirs")])))
        );
        lock(&pair.pc_core.waiting).push((pair.mac_key.clone(), letter("letter-1")));
        pair.pc_service.outbox_moved();
        pair.mac_core
            .until("the PC's letter", |core| !lock(&core.taken).is_empty())
            .await;
        assert!(lock(&pair.mac_kept.0).is_empty());

        // The connection ends: the Mac reads its names no more.
        pair.pc.shutdown.end();
        within(
            "the Mac stops reading",
            pair.mac_wants.wait_for(|wants| !*wants),
        )
        .await
        .unwrap();
    }

    /// B10: every mark a row is drawn with crosses as a word both the peer
    /// link and the receiving core carry, so a new mark never has the whole
    /// list refused there. The match names every mark: a new one does not
    /// build until it is listed here too.
    #[test]
    fn every_row_mark_crosses_as_a_word_both_sides_carry() {
        use hide_core::agent_state::RowMark;
        fn listed(mark: RowMark) -> RowMark {
            match mark {
                RowMark::Turn
                | RowMark::Going
                | RowMark::Waiting
                | RowMark::Done
                | RowMark::Unread
                | RowMark::Quiet
                | RowMark::Unknown
                | RowMark::Asleep
                | RowMark::Look => mark,
            }
        }
        let every = [
            RowMark::Turn,
            RowMark::Going,
            RowMark::Waiting,
            RowMark::Done,
            RowMark::Unread,
            RowMark::Quiet,
            RowMark::Unknown,
            RowMark::Asleep,
            RowMark::Look,
        ];
        let told: Vec<_> = every
            .into_iter()
            .enumerate()
            .map(|(n, mark)| hide_core::work_names::WorkName {
                pane: format!("w1:p{n}"),
                name: None,
                branch: None,
                mark: listed(mark),
                agent: None,
            })
            .collect();
        let CoreMessage::WorkNames { work, .. } = names_out(&told) else {
            panic!("work names");
        };
        assert_eq!(work.len(), every.len());
        assert!(hide_peer::protocol::core::valid_work(&work), "{work:?}");
        for told in &work {
            let mark = told.mark.as_deref().expect("every pane is told a mark");
            assert!(hide_core::guest_panes::mark_is_carried(mark), "{mark}");
        }
    }

    /// B10: the kind the sender keeps (an agent definition's id, the rule
    /// the core filters by) is the kind the receiver's `valid_work` takes;
    /// the day the two rules drift, a told agent would make the receiving
    /// machine refuse the sender's whole list. The protocol crate cannot
    /// link the adapter, so the two are held equal here.
    #[test]
    fn the_agent_kind_a_core_tells_is_the_kind_the_link_takes() {
        use hide_agent_adapter::definition::{builtins, id_is_valid};
        use hide_peer::protocol::core::work_agent_is_carried;
        let long = "a".repeat(hide_agent_adapter::definition::MAX_ID_BYTES);
        let longer = format!("{long}a");
        let samples = [
            "",
            "claude",
            "codex",
            "cursor-agent",
            "qwen3",
            "a",
            "-claude",
            "claude-",
            "my_agent",
            "Claude",
            "claude code",
            "a\nb",
            "é",
            &long,
            &longer,
        ];
        for kind in samples
            .iter()
            .copied()
            .chain(builtins().iter().map(|d| d.id.as_str()))
        {
            assert_eq!(id_is_valid(kind), work_agent_is_carried(kind), "{kind:?}");
        }
        let told: Vec<_> = builtins()
            .iter()
            .enumerate()
            .map(|(n, definition)| hide_core::work_names::WorkName {
                pane: format!("w1:p{n}"),
                name: None,
                branch: None,
                mark: hide_core::agent_state::RowMark::Going,
                agent: Some(definition.id.clone()),
            })
            .collect();
        let CoreMessage::WorkNames { work, .. } = names_out(&told) else {
            panic!("work names");
        };
        assert!(hide_peer::protocol::core::valid_work(&work), "{work:?}");
        assert!(work.iter().all(|told| told.agent.is_some()), "{work:?}");
    }

    /// B10: an earlier build tells `names` alone; the machine its work runs
    /// on keeps each name, with no branch or mark.
    #[tokio::test]
    async fn an_earlier_builds_names_are_kept_without_a_branch_or_mark() {
        let mut pair = Pair::new(Duration::from_secs(30)).await;
        within(
            "the Mac's stream to the PC",
            pair.mac_wants.wait_for(|wants| *wants),
        )
        .await
        .unwrap();
        let mut kept = pair.pc_kept.1.subscribe();
        let earlier = CoreMessage::WorkNames {
            names: vec![hide_peer::protocol::core::WorkName {
                pane: "w1:p1".to_owned(),
                name: "로그인 고치기".to_owned(),
            }],
            work: Vec::new(),
        };
        assert!(pair.mac_service.tell(&pair.pc_key, earlier));
        within("the names on the PC", kept.wait_for(|told| *told >= 1))
            .await
            .unwrap();
        let (from, work) = lock(&pair.pc_kept.0).last().cloned().expect("kept");
        assert_eq!(from, pair.mac_key);
        assert_eq!(
            work,
            HashMap::from([(
                "w1:p1".to_owned(),
                Work {
                    pane: "w1:p1".to_owned(),
                    name: Some("로그인 고치기".to_owned()),
                    branch: None,
                    mark: None,
                    agent: None,
                }
            )])
        );
    }

    /// B10: a plain terminal crosses by its branch and mark, with no name
    /// for a build before `work` to show; when the machine that started the
    /// work is gone, the machine it runs on drops where it stands.
    #[tokio::test]
    async fn a_terminal_crosses_by_its_branch_and_mark_and_its_mark_goes_with_its_machine() {
        let pair = Pair::new(Duration::from_secs(30)).await;
        let terminal = hide_core::work_names::WorkName {
            pane: "w1:p2".to_owned(),
            name: None,
            branch: Some("main".to_owned()),
            mark: hide_core::agent_state::RowMark::Going,
            agent: None,
        };
        let out = names_out(std::slice::from_ref(&terminal));
        let CoreMessage::WorkNames { names, work } = &out else {
            panic!("{out:?}");
        };
        assert!(names.is_empty(), "a build before `work` is told no name");
        assert_eq!(
            work,
            &[Work {
                pane: "w1:p2".to_owned(),
                name: None,
                branch: Some("main".to_owned()),
                mark: Some("going".to_owned()),
                agent: None,
            }]
        );
        let mut told = std::collections::BTreeMap::new();
        told.insert(pair.pc_key.clone(), vec![terminal]);
        pair.mac_names.send_replace(Arc::new(told));
        let mut count = pair.pc_kept.1.subscribe();
        within("the work on the PC", count.wait_for(|told| *told >= 1))
            .await
            .unwrap();
        assert_eq!(lock(&pair.pc_kept.0)[0].1["w1:p2"], work[0]);

        let mut away = pair.pc_kept.2.subscribe();
        pair.mac.shutdown.end();
        within(
            "the Mac away on the PC",
            away.wait_for(|away| away.contains(&pair.mac_key)),
        )
        .await
        .unwrap();
    }

    /// D-70 (security pre-review O2): a phone paired with the PC reaches
    /// the Mac, which opens it, as a notification its window shows though no
    /// one is at the Mac; the same message from the Mac, which the PC does
    /// not open, shows nothing on the PC.
    #[tokio::test]
    async fn a_new_phone_reaches_the_computer_that_opens_its_machine_and_not_the_other_way() {
        // A window at the Mac takes notifications; no one uses the Mac.
        let mac_desktop = Arc::new(DesktopNotices::new());
        mac_desktop.set(1, true);
        let mut mac_windows = mac_desktop.subscribe();
        let pair = Pair::with(Duration::from_secs(30), mac_desktop).await;
        within("the Mac's seat on the PC", pair.pc_desktop.judge_wanted()).await;

        assert_eq!(pair.pc_desktop.phone_paired("Pixel"), 1);
        let frame: Value = serde_json::from_str(
            &within("the new phone on the Mac", mac_windows.recv())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            frame["payload"]["phones"],
            json!([{
                "tag": format!("peer:{}|phone|Pixel", pair.pc_key),
                "peer": pair.pc_key,
                "phone": "Pixel",
            }])
        );

        // The PC may not open the Mac. The message is followed on the same
        // stream by a letter that is taken, so it was read.
        let mut pc_windows = pair.pc_desktop.subscribe();
        assert!(pair.mac_service.tell(
            &pair.pc_key,
            CoreMessage::PhonePaired {
                phone: "iPhone".to_owned(),
            }
        ));
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-1")));
        pair.mac_service.outbox_moved();
        pair.pc_core
            .until("the Mac's letter", |core| !lock(&core.taken).is_empty())
            .await;
        assert!(pc_windows.try_recv().is_err(), "nothing shows on the PC");
    }

    #[tokio::test]
    async fn a_letter_reaches_the_other_core_once_and_each_side_writes_on_the_one_stream() {
        let pair = Pair::new(RESEND_AFTER).await;
        // Written before the stream is open: it goes when the stream opens.
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-1")));
        pair.mac_service.outbox_moved();
        pair.mac_core
            .until("the answer to the Mac's letter", |core| {
                !lock(&core.answers).is_empty()
            })
            .await;
        assert_eq!(*lock(&pair.pc_core.taken), [letter("letter-1")]);
        assert_eq!(
            *lock(&pair.mac_core.answers),
            [(
                "letter-1".to_owned(),
                Outcome::Taken {
                    recipient: agent("agent-9", "reviewer"),
                }
            )]
        );

        // The PC holds no grant on the Mac and opened nothing: its letter
        // goes on the stream the Mac opened.
        lock(&pair.pc_core.waiting).push((pair.mac_key.clone(), letter("letter-7")));
        pair.pc_service.outbox_moved();
        pair.pc_core
            .until("the answer to the PC's letter", |core| {
                !lock(&core.answers).is_empty()
            })
            .await;
        assert_eq!(*lock(&pair.mac_core.taken), [letter("letter-7")]);
        // Nothing went twice while the other letter crossed.
        assert_eq!(lock(&pair.pc_core.taken).len(), 1);
    }

    #[tokio::test]
    async fn a_letter_the_other_core_could_not_keep_comes_again_and_a_refused_one_ends_with_its_reason()
     {
        // The resend is the subject: short, and waited for as an event.
        let pair = Pair::new(Duration::from_millis(50)).await;
        lock(&pair.pc_core.takes).extend([
            Err("ledger_unavailable".to_owned()),
            Ok(Outcome::Taken {
                recipient: agent("agent-9", "reviewer"),
            }),
            Ok(Outcome::Refused {
                reason: "target_unavailable".to_owned(),
            }),
        ]);
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-1")));
        pair.mac_service.outbox_moved();
        pair.mac_core
            .until("the answer to the letter sent again", |core| {
                !lock(&core.answers).is_empty()
            })
            .await;
        // Unanswered the first time, so the same letter came once more.
        assert_eq!(
            *lock(&pair.pc_core.taken),
            [letter("letter-1"), letter("letter-1")]
        );

        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-2")));
        pair.mac_service.outbox_moved();
        pair.mac_core
            .until("the refusal", |core| lock(&core.answers).len() == 2)
            .await;
        assert_eq!(
            lock(&pair.mac_core.answers)[1],
            (
                "letter-2".to_owned(),
                Outcome::Refused {
                    reason: "target_unavailable".to_owned(),
                }
            )
        );
        assert_eq!(lock(&pair.pc_core.taken).len(), 3);
    }

    /// A peer's calls are counted over its streams, not by each: the work
    /// of a stream that closed keeps its places until it ends, so opening
    /// another stream gets the peer no more of this daemon's work.
    #[tokio::test]
    async fn a_peers_calls_keep_their_places_past_the_stream_they_came_on_until_their_work_ends() {
        let pair = Pair::new(RESEND_AFTER).await;
        let mac = PeerKey::from_hex(&pair.mac_key).unwrap();
        let (release, held) = std::sync::mpsc::channel();
        *lock(&pair.pc_core.held) = Some(held);
        let call = |call: u64| CoreMessage::Call {
            call,
            from: party_out(&party("agent-1", "worker")),
            ask: Ask::Delivery,
            command: json!({"op": "agents", "command": {"action": "list", "live": true}}),
        };
        let refused = |call: u64| {
            Some(CoreMessage::CallAnswer {
                call,
                answer: None,
                refused: Some("capacity".to_owned()),
            })
        };
        let at_once = CALLS_AT_ONCE as u64;

        // One stream asks as many calls as the peer has places, then one
        // more.
        let (first, answers) = mpsc::channel(OUTBOUND);
        for n in 0..at_once {
            assert_eq!(pair.pc_service.take(mac, call(n), &first).await, None);
        }
        assert_eq!(
            pair.pc_service.take(mac, call(at_once), &first).await,
            refused(at_once)
        );
        pair.pc_core
            .until("the calls the PC works on", |core| {
                lock(&core.asked).len() == CALLS_AT_ONCE
            })
            .await;

        // The stream closes while that work goes on, and the peer opens
        // one after another: none of them is given a call's place.
        drop((first, answers));
        for stream in 1..=3 {
            let (again, _answers) = mpsc::channel(OUTBOUND);
            let n = at_once + stream;
            assert_eq!(pair.pc_service.take(mac, call(n), &again).await, refused(n));
        }
        assert_eq!(lock(&pair.pc_core.asked).len(), CALLS_AT_ONCE);
        let room = Arc::clone(&lock(&pair.pc_service.rooms)[&mac]);
        assert_eq!(room.available_permits(), 0);

        // The work ends: every place is free again, and the next call is
        // taken.
        drop(release);
        let free = within(
            "the places of the calls that ended",
            Arc::clone(&room).acquire_many_owned(CALLS_AT_ONCE as u32),
        )
        .await
        .unwrap();
        drop(free);
        lock(&pair.pc_core.calls).push_back(Ok(json!({"agents": []})));
        let (last, mut answers) = mpsc::channel(OUTBOUND);
        assert_eq!(pair.pc_service.take(mac, call(99), &last).await, None);
        assert_eq!(
            within("the answer to the call taken", answers.recv()).await,
            Some(CoreMessage::CallAnswer {
                call: 99,
                answer: Some(json!({"agents": []})),
                refused: None,
            })
        );
        assert!(lock(&pair.pc_service.working).len() <= CALLS_AT_ONCE + 1);
    }

    #[tokio::test]
    async fn a_read_asked_of_another_core_answers_with_what_that_core_said_and_says_when_it_is_away()
     {
        let pair = Pair::new(RESEND_AFTER).await;
        // A letter's answer says the stream is open.
        lock(&pair.mac_core.waiting).push((pair.pc_key.clone(), letter("letter-1")));
        pair.mac_service.outbox_moved();
        pair.mac_core
            .until("the stream between the cores", |core| {
                !lock(&core.answers).is_empty()
            })
            .await;
        lock(&pair.pc_core.calls).extend([
            Ok(json!({"agents": [{"id": "agent-9"}]})),
            Err("machine_not_permitted".to_owned()),
        ]);
        let list = || {
            Asked::Delivery(Box::new(Delivery::Agents {
                command: Agents::List {
                    live: true,
                    machine: None,
                },
            }))
        };
        let ask = |within: Duration| {
            let (service, to, asked) = (Arc::clone(&pair.mac_service), pair.pc_key.clone(), list());
            tokio::task::spawn_blocking(move || {
                service.call(&to, &party("agent-1", "worker"), &asked, within)
            })
        };
        let wait = Duration::from_secs(30);
        assert_eq!(
            ask(wait).await.unwrap(),
            Ok(json!({"agents": [{"id": "agent-9"}]}))
        );
        assert_eq!(
            lock(&pair.pc_core.asked)[0],
            (
                party("agent-1", "worker"),
                json!({"op": "agents", "command": {"action": "list", "live": true}})
            )
        );
        assert_eq!(
            ask(wait).await.unwrap(),
            Err("machine_not_permitted".into())
        );

        // A machine with no stream is away, and so is one whose connection
        // ends before it answered.
        assert_eq!(
            pair.mac_service
                .call(&pair.mac_key, &party("agent-1", "worker"), &list(), wait),
            Err("machine_unavailable".into())
        );
        let (release, held) = std::sync::mpsc::channel();
        *lock(&pair.pc_core.held) = Some(held);
        let pending = ask(wait);
        pair.pc_core
            .until("the call the PC holds", |core| lock(&core.asked).len() == 3)
            .await;
        pair.pc.shutdown.end();
        assert_eq!(
            within("the held call's end", pending).await.unwrap(),
            Err("machine_unavailable".into())
        );
        drop(release);
        drop(pair.mac);
    }
}
