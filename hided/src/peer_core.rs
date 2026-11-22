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

use hide_core::core_calls::{self, Agent, Asked, CoreItem, CoreLink, Letter, Outcome, Party};
use hide_peer::PeerEvent;
use hide_peer::protocol::core::{Ask, CoreMessage, Item};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::stream::Status;
use hide_peer::protocol::{PeerKey, StreamKind, frame};
use hide_peer::wire::{MessageReader, write_message};
use serde_json::{Value, json};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::peer_host::{HostPeers, Incoming, Link, PeerHost, Stream};

/// How long the core has to keep one letter or write one answer.
const LETTER_WITHIN: Duration = Duration::from_secs(5);
/// How long a letter waits for its answer before this daemon sends it again.
const RESEND_AFTER: Duration = Duration::from_secs(30);
/// How long a read asked of another core may take, and a start, which
/// makes a worktree and waits for its agent.
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
}

/// One open stream to a peer: what is written to it.
struct Open {
    id: u64,
    out: mpsc::Sender<CoreMessage>,
    /// This side opened it.
    mine: bool,
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

pub struct PeerCore {
    core: Arc<dyn Cores>,
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
}

impl PeerCore {
    /// Takes the `core` streams peers open and follows every connection to
    /// open its own, until the daemon ends, and hands the core this
    /// connection to the others. Called once, before the endpoint opens.
    pub fn serve(core: &Arc<CoreHandle>, host: &Arc<PeerHost>, shutdown: Ending) -> Arc<Self> {
        let service = Self::over(
            Arc::clone(core) as Arc<dyn Cores>,
            host,
            shutdown,
            RESEND_AFTER,
        );
        core.set_core_link(Arc::clone(&service) as Arc<dyn CoreLink>);
        service
    }

    fn over(
        core: Arc<dyn Cores>,
        host: &Arc<PeerHost>,
        shutdown: Ending,
        resend_after: Duration,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            core,
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
        });
        let taking = Arc::clone(&service);
        host.handle(StreamKind::Core, move |incoming: Incoming| {
            let service = Arc::clone(&taking);
            async move {
                let key = incoming.link.key();
                service.session(key, incoming.stream, false).await;
            }
        });
        tokio::spawn(Arc::clone(&service).follow(Arc::clone(host)));
        tokio::spawn(Arc::clone(&service).pump());
        service
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
        lock(&self.open).entry(key).or_default().push(Open {
            id,
            out: out.clone(),
            mine,
        });
        record("peer_core.opened", &key, json!({"mine": mine}));
        // What waited for this machine goes now.
        self.moved.notify_one();
        let ended = loop {
            tokio::select! {
                () = self.shutdown.ended() => break None,
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
        let gone = {
            let mut open = lock(&self.open);
            let Some(streams) = open.get_mut(key) else {
                return;
            };
            streams.retain(|stream| stream.id != id);
            let gone = streams.is_empty();
            if gone {
                open.remove(key);
            }
            gone
        };
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
        mac_key: String,
        pc_key: String,
    }

    impl Pair {
        async fn new(resend_after: Duration) -> Self {
            let (mac, pc) = (Machine::new("macbook"), Machine::new("windows-pc"));
            let (mac_core, pc_core) = (Arc::new(Core::default()), Arc::new(Core::default()));
            let serve = |machine: &Machine, core: &Arc<Core>| {
                PeerCore::over(
                    Arc::clone(core) as Arc<dyn Cores>,
                    &machine.host,
                    machine.shutdown.clone(),
                    resend_after,
                )
            };
            let (mac_service, pc_service) = (serve(&mac, &mac_core), serve(&pc, &pc_core));
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
                mac_key,
                pc_key,
            }
        }
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
