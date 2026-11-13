//! One outbound relay socket, owned by Mobile. The relay routes opaque frames;
//! phone admission and every encrypted application frame stay in phone.rs.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::{
    Error,
    admission::{
        ClientMessage, DeviceHello, DeviceKey, PushOutcome, PushRequest, Refusal, ServerMessage,
    },
    frame::{Frame, MAX_FRAME},
    noise::{NoiseKeypair, Session},
    qr::PairingQr,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use super::{Mobile, phone, store::RelayKeys};

const DEADLINE: Duration = Duration::from_secs(10);
/// One budget from peer arrival through Noise and the first credential message.
const PHONE_AUTH_DEADLINE: Duration = Duration::from_secs(10);
const MAX_PEERS: usize = 8;
const INBOUND_QUEUE: usize = 8;
const OUTBOUND_QUEUE: usize = 32;
/// The phone application contract, independent of relay wire version. 2 is
/// the list a parent's children hang under, with devices in one recency
/// order (PRD mobile-companion-ui D-18). 3 adds a page View's screenshot, a
/// `look_head` of mode `page`. 4 adds the pull request tab's `prs` frame and
/// its requests. 5 adds the `html` kind of a View, which an older list has no
/// icon or word for, and its `look_head` of mode `html`. An addition (a frame type, an optional field, a request sent
/// only from what those draw) keeps the number; it is raised only for a change
/// an older side would misread (`docs/ARCHITECTURE.md`, the phone app), and
/// `web/src/mobile/phoneVersion.ts` carries it too.
pub const PHONE_PROTOCOL: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Connecting,
    Connected,
    Retrying,
    Refused(Refusal),
    KeyUnavailable,
}
impl State {
    pub fn value(&self) -> serde_json::Value {
        match self {
            Self::Off => json!({"state":"off"}),
            Self::Connecting => json!({"state":"connecting"}),
            Self::Connected => json!({"state":"connected"}),
            Self::Retrying => json!({"state":"retrying"}),
            Self::Refused(reason) => json!({"state":"refused", "reason":reason}),
            Self::KeyUnavailable => json!({"state":"key_unavailable"}),
        }
    }
}

pub struct Keys {
    device: DeviceKey,
    noise: NoiseKeypair,
}
impl Keys {
    pub fn generate() -> Result<(Self, RelayKeys), Error> {
        let device = DeviceKey::generate()?;
        let noise = NoiseKeypair::generate()?;
        let stored = RelayKeys {
            device_seed: URL_SAFE_NO_PAD.encode(device.seed().as_ref()),
            noise_secret: URL_SAFE_NO_PAD.encode(noise.secret().as_ref()),
            noise_public: URL_SAFE_NO_PAD.encode(noise.public_key()),
        };
        Ok((Self { device, noise }, stored))
    }
    pub fn load(stored: &RelayKeys) -> Result<Self, Error> {
        fn key(text: &str) -> Result<[u8; 32], Error> {
            URL_SAFE_NO_PAD
                .decode(text)
                .map_err(|_| Error::InvalidKey)?
                .try_into()
                .map_err(|_| Error::InvalidKey)
        }
        let noise = NoiseKeypair::from_secret(&key(&stored.noise_secret)?)?;
        if noise.public_key() != key(&stored.noise_public)? {
            return Err(Error::InvalidKey);
        }
        Ok(Self {
            device: DeviceKey::from_seed(&key(&stored.device_seed)?),
            noise,
        })
    }
    pub fn room(&self) -> String {
        hide_relay_protocol::admission::room_id(&self.device.public_key())
    }
    pub fn qr(&self, code: &str, name: &str) -> Result<String, Error> {
        let code: [u8; 16] = URL_SAFE_NO_PAD
            .decode(code)
            .map_err(|_| Error::InvalidQr)?
            .try_into()
            .map_err(|_| Error::InvalidQr)?;
        // A host's display name is outside the QR contract's 128-byte bound.
        // Keep complete Unicode characters and exclude controls from its label.
        let mut label = String::new();
        for ch in name.chars().filter(|ch| !ch.is_control()) {
            if label.len() + ch.len_utf8() > 128 {
                break;
            }
            label.push(ch);
        }
        let label = label.trim();
        let name = if label.is_empty() { "Hide" } else { label };
        PairingQr::new(
            &self.device.public_key(),
            &self.noise.public_key(),
            &code,
            name,
        )?
        .encode()
    }
}

fn diagnostic(connection: Option<u32>, reason: &str) {
    hide_core::diagnostic!(json!({
        "component":"mobile_relay", "kind":"relay.closed", "connection":connection, "reason":reason,
    }));
}

/// Equal jitter, exponential to a 30-second ceiling. No retained retry queue.
fn backoff(attempt: u32) -> Result<Duration, Error> {
    let ceiling = (1000_u64 << attempt.min(5)).min(30_000);
    let mut random = [0; 8];
    getrandom::getrandom(&mut random).map_err(|_| Error::RandomUnavailable)?;
    Ok(Duration::from_millis(
        ceiling / 2 + u64::from_le_bytes(random) % (ceiling / 2 + 1),
    ))
}

pub async fn supervise(mobile: Arc<Mobile>) {
    let mut changes = mobile.relay_change.subscribe();
    let mut stopping = mobile.stopping.subscribe();
    let mut open = mobile.config.open.clone();
    let mut attempt = 0;
    let mut peers = JoinSet::new();
    loop {
        changes.borrow_and_update();
        if *stopping.borrow() {
            break;
        }
        if !mobile.relay_desired() {
            tokio::select! {
                _ = changes.changed() => {},
                _ = open.changed(), if open.has_changed().is_ok() => {},
                _ = stopping.changed() => break,
            }
            attempt = 0;
            continue;
        }
        let keys = match mobile.relay_keys() {
            Ok(keys) => keys,
            Err(()) => {
                mobile.relay_state(State::KeyUnavailable);
                tokio::select! {
                    _ = changes.changed() => {},
                    _ = stopping.changed() => break,
                }
                continue;
            }
        };
        if attempt == 0 {
            mobile.relay_state(State::Connecting);
        }
        let result = tokio::select! {
            biased;
            _ = stopping.changed() => None,
            _ = changes.changed() => None,
            _ = open.changed(), if open.has_changed().is_ok() => None,
            result = connect(&mobile, keys, &mut peers) => Some(result),
        };
        // Cancellation drops the WebSocket before aborting all its phone owners.
        // Their registration guards release viewing and start demand on every path.
        peers.abort_all();
        while peers.join_next().await.is_some() {}
        let Some(result) = result else {
            attempt = 0;
            continue;
        };
        diagnostic(
            None,
            match &result {
                Err(LinkError::Refused(_)) => "admission_refused",
                _ => "link_lost",
            },
        );
        match result {
            Err(LinkError::Refused(reason)) => mobile.relay_state(State::Refused(reason)),
            _ => mobile.relay_state(State::Retrying),
        }
        let delay = match backoff(attempt) {
            Ok(delay) => delay,
            Err(_) => {
                diagnostic(None, "random_unavailable");
                mobile.relay_state(State::KeyUnavailable);
                tokio::select! {
                    _ = changes.changed() => { attempt = 0; },
                    _ = stopping.changed() => break,
                }
                continue;
            }
        };
        attempt = attempt.saturating_add(1);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {},
            _ = changes.changed() => { attempt = 0; },
            _ = open.changed(), if open.has_changed().is_ok() => { attempt = 0; },
            _ = stopping.changed() => break,
        }
    }
}

enum LinkError {
    Closed,
    Refused(Refusal),
}
impl From<Error> for LinkError {
    fn from(_: Error) -> Self {
        Self::Closed
    }
}
impl From<tokio_tungstenite::tungstenite::Error> for LinkError {
    fn from(_: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Closed
    }
}
pub(super) enum Output {
    Frame { connection: u32, bytes: Vec<u8> },
    Closed { connection: u32 },
}

/// Push requests waiting for the socket loop to send them. A blocking
/// caller that finds it full is answered at once rather than queued.
const PUSH_QUEUE: usize = 8;
/// Requests sent and not yet answered. The relay answers each within its own
/// deadline and one retry, so a full table means it stopped answering.
const MAX_PENDING_PUSH: usize = 32;
/// How long a caller waits for the relay's answer: two of the relay's
/// ten-second Apple deadlines (the one retry) and a margin.
pub const PUSH_ANSWER_WAIT: Duration = Duration::from_secs(30);

/// One request the link carries to the relay, and where its answer goes.
struct PushJob {
    request: PushRequest,
    answer: std::sync::mpsc::SyncSender<PushAnswer>,
}

/// What a running link offers the rest of Mobile: a push request to the
/// relay, when the relay said it forwards them. A relay that does not know
/// the `push` field of `registered` sends `{"type":"registered"}`, which reads
/// as false, so hided never sends a message an older relay would close the
/// link on (`docs/RELAY_PROTOCOL.md`, Push relay).
#[derive(Clone)]
pub struct Link {
    jobs: mpsc::Sender<PushJob>,
    next_id: Arc<AtomicU32>,
    push: bool,
}

impl Link {
    /// Sends one push request and waits for the relay's result. Blocking:
    /// every caller is a blocking thread, where Web Push sends already run.
    /// `Err` is why no result came, as the diagnostic names it.
    pub fn push(&self, mut request: PushRequest) -> Result<PushOutcome, &'static str> {
        if !self.push {
            return Err("relay_without_push");
        }
        request.id = self.next_id();
        let (answer, result) = std::sync::mpsc::sync_channel(1);
        self.jobs
            .try_send(PushJob { request, answer })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "push_queue_full",
                mpsc::error::TrySendError::Closed(_) => "relay_link_ended",
            })?;
        result
            .recv_timeout(PUSH_ANSWER_WAIT)
            .map_err(|error| match error {
                std::sync::mpsc::RecvTimeoutError::Timeout => "relay_no_answer",
                std::sync::mpsc::RecvTimeoutError::Disconnected => "relay_link_ended",
            })?
    }

    /// A request id this link has not used lately; 0 is not a request id
    /// (`admission::PushRequest`).
    fn next_id(&self) -> u32 {
        loop {
            let id = self.next_id.fetch_add(1, AtomicOrdering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }
}

/// The push requests sent and not yet answered, owned by the socket loop.
#[derive(Default)]
struct PendingPushes {
    sent: HashMap<u32, (std::time::Instant, std::sync::mpsc::SyncSender<PushAnswer>)>,
}

type PushAnswer = Result<PushOutcome, &'static str>;

impl PendingPushes {
    /// The text to send for `job`, or `None` when the job was answered here
    /// instead: a full table (the relay stopped answering) or a request that
    /// cannot be encoded. Either is that request's failure alone; the link
    /// and its phones are untouched.
    fn send(&mut self, job: PushJob, now: std::time::Instant) -> Option<String> {
        // A caller stops waiting after PUSH_ANSWER_WAIT; its entry goes then,
        // so the table holds only answers still awaited.
        self.sent
            .retain(|_, (sent, _)| now.saturating_duration_since(*sent) < PUSH_ANSWER_WAIT);
        if self.sent.len() >= MAX_PENDING_PUSH {
            let _ = job.answer.send(Err("relay_not_answering"));
            return None;
        }
        let id = job.request.id;
        let Ok(text) = ClientMessage::Push(job.request).encode() else {
            let _ = job.answer.send(Err("push_request_invalid"));
            return None;
        };
        self.sent.insert(id, (now, job.answer));
        Some(text)
    }

    /// Hands the relay's answer to its caller; an id this link never sent,
    /// or one already answered, is not this relay's answer.
    fn answer(&mut self, id: u32, outcome: PushOutcome) {
        if let Some((_, answer)) = self.sent.remove(&id) {
            let _ = answer.send(Ok(outcome));
        }
    }
}

/// Holds the link Mobile hands out while its socket loop runs, and takes it
/// back on every way out of the loop: an end, an error or a cancellation.
struct Published<'a> {
    mobile: &'a Mobile,
    link: Link,
}

impl<'a> Published<'a> {
    fn new(mobile: &'a Mobile, link: Link) -> Self {
        mobile.lock().relay_link = Some(link.clone());
        Self { mobile, link }
    }
}

impl Drop for Published<'_> {
    fn drop(&mut self) {
        let mut inner = self.mobile.lock();
        // A newer link may already stand in its place; only this one goes.
        if inner
            .relay_link
            .as_ref()
            .is_some_and(|held| held.jobs.same_channel(&self.link.jobs))
        {
            inner.relay_link = None;
        }
    }
}

struct Peer {
    inbound: mpsc::Sender<Vec<u8>>,
    task: AbortHandle,
}

async fn connect(
    mobile: &Arc<Mobile>,
    keys: Arc<Keys>,
    tasks: &mut JoinSet<u32>,
) -> Result<(), LinkError> {
    // Wait out an in-progress Tailscale exposure before checking the saved entry.
    // A method change cancels this future, including the wait.
    {
        let _reconciling = mobile.reconcile_lock.lock().await;
        if !mobile.relay_desired() {
            return Err(LinkError::Closed);
        }
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (mut socket, _) = tokio::time::timeout(
        DEADLINE,
        connect_async_with_config(&mobile.config.relay_url, Some(config), true),
    )
    .await
    .map_err(|_| LinkError::Closed)??;
    let hello = DeviceHello::new(&keys.device.public_key(), PHONE_PROTOCOL)?;
    let push = tokio::time::timeout(DEADLINE, async {
        socket
            .send(Message::Text(
                ClientMessage::Device {
                    hello: hello.clone(),
                }
                .encode()?
                .into(),
            ))
            .await?;
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            return Err(LinkError::Closed);
        };
        let challenge = match ServerMessage::decode(&text)? {
            ServerMessage::Challenge { challenge } => challenge,
            ServerMessage::Refused { reason } => return Err(LinkError::Refused(reason)),
            _ => return Err(LinkError::Closed),
        };
        let signature = keys.device.sign(&hello, &challenge)?;
        socket
            .send(Message::Text(
                ClientMessage::Proof { signature }.encode()?.into(),
            ))
            .await?;
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            return Err(LinkError::Closed);
        };
        match ServerMessage::decode(&text)? {
            // `push` reads as false from a relay that does not know the
            // field, so an older relay is served every phone frame and no
            // push request (`docs/RELAY_PROTOCOL.md`, Push relay, deploying).
            ServerMessage::Registered { push, .. } => Ok(push),
            ServerMessage::Refused { reason } => Err(LinkError::Refused(reason)),
            _ => Err(LinkError::Closed),
        }
    })
    .await
    .map_err(|_| LinkError::Closed)??;
    mobile.relay_state(State::Connected);
    let (outgoing, mut output) = mpsc::channel::<Output>(OUTBOUND_QUEUE);
    let (jobs, mut pushed) = mpsc::channel::<PushJob>(PUSH_QUEUE);
    let link = Link {
        jobs,
        next_id: Arc::new(AtomicU32::new(1)),
        push,
    };
    // The link is handed out only while this loop runs.
    let _published = Published::new(mobile, link);
    // What waited for the link goes out now that it is published.
    mobile.flush_held();
    let mut pending = PendingPushes::default();
    let mut peers: HashMap<u32, Peer> = HashMap::new();
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heard = tokio::time::Instant::now();
    loop {
        let send = tokio::select! {
            incoming = socket.next() => {
                heard = tokio::time::Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text))) => match ServerMessage::decode(&text)? {
                        ServerMessage::PeerJoined { connection } => {
                            if peers.contains_key(&connection) { return Err(LinkError::Closed); }
                            if peers.len() >= MAX_PEERS || tasks.len() >= MAX_PEERS {
                                diagnostic(Some(connection), "connection_limit");
                                Some(Message::Text(ClientMessage::Close { connection }.encode()?.into()))
                            } else {
                                let (tx, rx) = mpsc::channel(INBOUND_QUEUE);
                                let task = tasks.spawn(serve(Arc::clone(mobile), Arc::clone(&keys), connection, rx, outgoing.clone()));
                                peers.insert(connection, Peer { inbound: tx, task });
                                None
                            }
                        }
                        ServerMessage::PeerLeft { connection } => {
                            if let Some(peer) = peers.remove(&connection) { peer.task.abort(); }
                            None
                        }
                        // Answers an id this link sent; one it never sent, or
                        // an id already answered, is not this relay's answer.
                        ServerMessage::PushResult(result) => {
                            pending.answer(result.id, result.result);
                            None
                        }
                        ServerMessage::Refused { reason } => return Err(LinkError::Refused(reason)),
                        _ => return Err(LinkError::Closed),
                    },
                    Some(Ok(Message::Binary(bytes))) => {
                        let connection = Frame::decode(&bytes)?.connection;
                        if let Some(peer) = peers.get(&connection)
                            && peer.inbound.try_send(bytes.to_vec()).is_err() {
                            diagnostic(Some(connection), "receive_limit");
                            if let Some(peer) = peers.remove(&connection) { peer.task.abort(); }
                            Some(Message::Text(ClientMessage::Close { connection }.encode()?.into()))
                        } else { None }
                    }
                    Some(Ok(Message::Ping(bytes))) => Some(Message::Pong(bytes)),
                    Some(Ok(Message::Pong(_))) => None,
                    _ => return Err(LinkError::Closed),
                }
            }
            Some(output) = output.recv() => match output {
                Output::Frame { connection, bytes } if peers.contains_key(&connection) => Some(Message::Binary(bytes.into())),
                Output::Closed { connection } if peers.remove(&connection).is_some() => Some(Message::Text(ClientMessage::Close { connection }.encode()?.into())),
                _ => None,
            },
            Some(job) = pushed.recv() => {
                pending.send(job, std::time::Instant::now()).map(|text| Message::Text(text.into()))
            },
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                let _ = result;
                None
            }
            _ = ping.tick() => {
                if heard.elapsed() >= Duration::from_secs(60) { return Err(LinkError::Closed); }
                Some(Message::Ping(Default::default()))
            }
        };
        if let Some(message) = send {
            tokio::time::timeout(DEADLINE, socket.send(message))
                .await
                .map_err(|_| LinkError::Closed)??;
        }
    }
}

async fn serve(
    mobile: Arc<Mobile>,
    keys: Arc<Keys>,
    connection: u32,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    outbound: mpsc::Sender<Output>,
) -> u32 {
    let authenticate_by = tokio::time::Instant::now() + PHONE_AUTH_DEADLINE;
    let result = async {
        let mut session = Session::responder(&keys.noise, connection)?;
        tokio::time::timeout_at(authenticate_by, async {
            let frame = inbound.recv().await.ok_or(Error::SessionClosed)?;
            session.read_handshake(&frame)?;
            outbound
                .send(Output::Frame {
                    connection,
                    bytes: session.write_handshake()?,
                })
                .await
                .map_err(|_| Error::SessionClosed)
        })
        .await
        .map_err(|_| Error::Timeout)??;
        let mut socket = phone::Socket::relay(session, inbound, outbound.clone(), connection);
        let first = tokio::time::timeout_at(authenticate_by, socket.recv())
            .await
            .map_err(|_| Error::Timeout)?;
        let Some(Ok(axum::extract::ws::Message::Text(text))) = first else {
            return Err(Error::InvalidMessage);
        };
        if text.len() > phone::MAX_FRAME {
            return Err(Error::SizeLimit);
        }
        let first = serde_json::from_str(&text).map_err(|_| Error::InvalidMessage)?;
        if !phone::is_phone(&first) {
            return Err(Error::InvalidMessage);
        }
        // Relay ids never collide with the loopback server's connection counter.
        phone::serve_socket(
            socket,
            first,
            mobile,
            u64::from(connection) | (1 << 63),
            String::new(),
        )
        .await;
        Ok::<_, Error>(())
    }
    .await;
    if let Err(error) = result {
        diagnostic(Some(connection), &error.to_string());
    }
    // Share the data queue so the peer receives its final encrypted refusal
    // before the routing close. Completion of a task is not a send barrier.
    let _ = outbound.send(Output::Closed { connection }).await;
    connection
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_relay_protocol::admission::PushEnvironment;

    fn request(id: u32) -> PushRequest {
        PushRequest {
            id,
            environment: PushEnvironment::Sandbox,
            token: "ab".repeat(32),
            collapse: "collapse".into(),
            payload: URL_SAFE_NO_PAD.encode([7_u8; 64]),
        }
    }

    fn job(id: u32) -> (PushJob, std::sync::mpsc::Receiver<PushAnswer>) {
        let (answer, result) = std::sync::mpsc::sync_channel(1);
        (
            PushJob {
                request: request(id),
                answer,
            },
            result,
        )
    }

    /// A caller is answered at once, never left waiting, when the link cannot
    /// carry its request: an older relay, a full queue, an ended link.
    #[test]
    fn a_push_the_link_cannot_carry_is_answered_at_once() {
        let (jobs, mut queued) = mpsc::channel(1);
        let link = |push| Link {
            jobs: jobs.clone(),
            next_id: Arc::new(AtomicU32::new(1)),
            push,
        };
        assert_eq!(link(false).push(request(0)), Err("relay_without_push"));
        let (waiting, _answer) = job(1);
        jobs.try_send(waiting).ok().unwrap();
        assert_eq!(link(true).push(request(0)), Err("push_queue_full"));
        queued.close();
        let _ = queued.try_recv();
        assert_eq!(link(true).push(request(0)), Err("relay_link_ended"));
    }

    /// The table of unanswered requests is capped; an entry goes once its
    /// caller stopped waiting; an answer reaches only its own caller; a request
    /// that cannot be encoded fails alone.
    #[test]
    fn unanswered_requests_are_bounded_and_answers_reach_their_caller() {
        let mut pending = PendingPushes::default();
        let start = std::time::Instant::now();
        let mut callers = Vec::new();
        for id in 1..=MAX_PENDING_PUSH as u32 {
            let (job, result) = job(id);
            assert!(pending.send(job, start).is_some());
            callers.push(result);
        }
        let (over, refused) = job(99);
        assert_eq!(pending.send(over, start), None);
        assert_eq!(refused.try_recv(), Ok(Err("relay_not_answering")));
        pending.answer(1, PushOutcome::Delivered);
        pending.answer(1, PushOutcome::TokenGone);
        pending.answer(500, PushOutcome::Delivered);
        assert_eq!(callers[0].try_recv(), Ok(Ok(PushOutcome::Delivered)));
        assert!(callers[0].try_recv().is_err(), "one answer per request");
        assert!(callers[1].try_recv().is_err());
        // Past the wait the old entries are gone and a new request is sent.
        let (late, _) = job(100);
        assert!(pending.send(late, start + PUSH_ANSWER_WAIT).is_some());
        assert_eq!(pending.sent.len(), 1);
        // A request the wire refuses to encode is that request's failure.
        let (broken, result) = job(0);
        assert_eq!(pending.send(broken, start + PUSH_ANSWER_WAIT), None);
        assert_eq!(result.try_recv(), Ok(Err("push_request_invalid")));
    }
}
