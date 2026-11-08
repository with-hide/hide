//! One phone's connection on `/ws` (PRD D-04, D-13, D-16, D-17).
//!
//! The first frame names it: `{client_kind: "phone", token}` for a paired
//! phone, `{client_kind: "phone", pair, name}` to pair with the live code.
//! After that the phone may only: open one agent's detail, choose its
//! conversation or its terminal, ask for older messages or more rows, close
//! it, send that pane a reply or one key, open the start sheet and start an
//! agent from it, wake a sleeping agent in its pane, wake or inspect a listed
//! sleeping conversation, answer the decision a listed mission waits on
//! (`answer_mission`), read, merge or hand to an agent a listed pull request
//! (`read_pr`, `merge_pr`, `read_pr_feedback`, `delegate_pr`), register or
//! report its push subscription, ask whether hided still answers (`ping`),
//! and look at one View an agent of its open detail opened (`look_open`,
//! `look_ack`, `look_seen`). Anything else is refused and recorded. It never
//! receives a path, a setting or the core's snapshot, and a file only as the
//! content of a View that is open now, asked for by its id (`looks.rs`); the
//! one setting is the interface language, which rides each `agents` frame so
//! the phone speaks the language the operator chose.
//!
//! Every frame to the phone goes through `encode`, the one place a relay
//! transport would wrap in end-to-end encryption (PRD D-01).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use hide_core::agent_state::phone::{DormantPhase, SleepId};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::conversation::{self, Tail, Transcript};
use super::looks;
use super::pane::{self, Input, PaneError};
use super::projection::{AgentKey, PhoneMissions, PhonePrs, Projection};
use super::prs;
use super::store::{Notifications, PushSubscription};
use super::{InputState, Mobile, PhoneMeta, Reservation};

/// How often an open detail reads its pane again.
const DETAIL_INTERVAL: Duration = Duration::from_secs(1);
/// The largest frame a phone may send.
pub(super) const MAX_FRAME: usize = 16 * 1024;
/// The close code for a refused phone: the PWA shows why from the frame before it.
const CLOSE_REFUSED: u16 = 4001;
/// How often a live phone is pinged; the browser answers on its own.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// A phone that sent nothing, not even a pong, for this long is gone: its
/// socket through tailscaled can stay half-open for minutes, holding a
/// client slot and a "viewing" that would hold back its push (B33).
const SILENT_LIMIT: Duration = Duration::from_secs(45);
/// How long one frame may take to leave: a send into a half-open socket
/// whose buffer is full would otherwise block the loop, and with it the
/// ping, the silent limit and a close for eviction or revoke.
const SEND_LIMIT: Duration = Duration::from_secs(10);
// The list frames' budget is the relay's message limit (hide-core names it
// without linking the relay crate).
const _: () =
    assert!(hide_core::agent_state::phone::FRAME_LIMIT == hide_relay_protocol::noise::MAX_MESSAGE);

/// Sends one message; false when the socket failed or stalled past
/// `SEND_LIMIT`, and the connection should end.
async fn send(socket: &mut Socket, message: Message, phone_id: &str) -> bool {
    match tokio::time::timeout(SEND_LIMIT, socket.send(message)).await {
        Ok(Ok(())) => true,
        Ok(Err(())) => {
            hide_core::diagnostic!(
                json!({"component":"mobile_phone", "kind":"phone.send_failed", "phone_id":phone_id})
            );
            false
        }
        Err(_) => {
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phone.stalled", "phone_id": phone_id,
            }));
            false
        }
    }
}

/// The one function every application frame to a phone passes through.
/// Relay messages leave this boundary only as authenticated ciphertext.
fn encode(
    frame: &Value,
    session: Option<&mut hide_relay_protocol::noise::Session>,
) -> Result<Vec<Message>, hide_relay_protocol::Error> {
    let text = frame.to_string();
    match session {
        Some(session) => Ok(session
            .send_message(text.as_bytes())?
            .into_iter()
            .map(|bytes| Message::Binary(bytes.into()))
            .collect()),
        None => Ok(vec![Message::Text(text.into())]),
    }
}

async fn send_frame(socket: &mut Socket, frame: &Value, phone_id: &str) -> bool {
    let session = match socket {
        Socket::Direct(_) => None,
        Socket::Relay(relay) => Some(&mut relay.session),
    };
    let messages = match encode(frame, session) {
        Ok(messages) => messages,
        Err(reason) => {
            hide_core::diagnostic!(
                json!({"component":"mobile_phone", "kind":"phone.encode_failed", "phone_id":phone_id, "reason":reason.to_string()})
            );
            return false;
        }
    };
    for message in messages {
        if !send(socket, message, phone_id).await {
            return false;
        }
    }
    true
}

pub(super) enum Socket {
    Direct(Box<WebSocket>),
    Relay(Box<RelaySocket>),
}
pub(super) struct RelaySocket {
    session: hide_relay_protocol::noise::Session,
    incoming: mpsc::Receiver<Vec<u8>>,
    outgoing: mpsc::Sender<super::relay::Output>,
    clock: tokio::time::Instant,
    timer: tokio::time::Interval,
    connection: u32,
}
impl Socket {
    pub(super) fn relay(
        session: hide_relay_protocol::noise::Session,
        incoming: mpsc::Receiver<Vec<u8>>,
        outgoing: mpsc::Sender<super::relay::Output>,
        connection: u32,
    ) -> Self {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self::Relay(Box::new(RelaySocket {
            session,
            incoming,
            outgoing,
            clock: tokio::time::Instant::now(),
            timer,
            connection,
        }))
    }
    async fn send(&mut self, message: Message) -> Result<(), ()> {
        match self {
            Self::Direct(socket) => socket.send(message).await.map_err(|_| ()),
            Self::Relay(relay) => match message {
                Message::Binary(bytes) => relay
                    .outgoing
                    .try_send(super::relay::Output::Frame {
                        connection: relay.connection,
                        bytes: bytes.to_vec(),
                    })
                    .map_err(|_| ()),
                // Relay liveness uses the phone protocol's JSON ping. Closing the
                // serving task tells the supervisor to send the routing close.
                Message::Ping(_) | Message::Close(_) => Ok(()),
                _ => Err(()),
            },
        }
    }
    pub(super) async fn recv(&mut self) -> Option<Result<Message, ()>> {
        match self {
            Self::Direct(socket) => socket.recv().await.map(|result| result.map_err(|_| ())),
            Self::Relay(relay) => loop {
                let result = tokio::select! {
                    incoming = relay.incoming.recv() => {
                        let bytes = incoming?;
                        relay.session.recv_message(&bytes, relay.clock.elapsed().as_millis() as u64)
                    },
                    _ = relay.timer.tick() => {
                        relay.session.check_timeout(relay.clock.elapsed().as_millis() as u64).map(|()| None)
                    },
                };
                match result {
                    Ok(Some(bytes)) => {
                        return Some(
                            String::from_utf8(bytes)
                                .map(|text| Message::Text(text.into()))
                                .map_err(|_| ()),
                        );
                    }
                    Ok(None) => {}
                    Err(reason) => {
                        hide_core::diagnostic!(
                            json!({"component":"mobile_relay", "kind":"phone.cipher_refused",
                            "connection":relay.connection, "reason":reason.to_string()})
                        );
                        return Some(Err(()));
                    }
                }
            },
        }
    }
}

fn refusal_logged(kind: &str, phone_id: Option<&str>, reason: &str) {
    hide_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": kind, "phone_id": phone_id, "reason": reason,
    }));
}

async fn refuse(socket: &mut Socket, reason: &str) {
    let refusal = json!({"type": "refused", "reason": reason});
    let _ = tokio::time::timeout(SEND_LIMIT, async {
        let _ = send_frame(socket, &refusal, "unadmitted").await;
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_REFUSED,
                reason: reason.to_owned().into(),
            })))
            .await;
    })
    .await;
}

/// Whether a first frame is a phone's.
pub fn is_phone(first: &Value) -> bool {
    first.get("client_kind").and_then(Value::as_str) == Some("phone")
}

/// The list frame and how many agents it left out to fit one relay message
/// (`Projection::wire`), which the phone shows as a line under the list.
fn agents_frame(projection: &Projection) -> (Value, usize) {
    let wire = projection.wire();
    let frame = json!({
        "type": "agents",
        "groups": wire.groups,
        "omitted": wire.omitted,
        // The Mac and the devices it reaches, for the header and the device sheet.
        "devices": projection.devices,
        // The core's language as stored, or null to follow the phone's own.
        "interface_language": projection.interface_language,
        // The operator's palette; the phone keeps its own light or dark.
        "color_theme": projection.color_theme,
    });
    (frame, wire.omitted)
}

/// Records when a connection's list first leaves agents out, not on every
/// change while it stays over (the count is in the frame).
fn note_agents_omitted(phone_id: &str, before: &mut usize, omitted: usize) {
    if omitted > 0 && *before == 0 {
        hide_core::diagnostic!(json!({
            "component": "mobile_phone", "kind": "agents.capped", "phone_id": phone_id,
            "omitted": omitted, "budget": hide_core::agent_state::phone::LIST_BUDGET,
        }));
    }
    *before = omitted;
}

/// The missions the phone lists (PRD mobile-companion-ui B28, B31-B36): the
/// record's trimmed projection, republished when it changes.
fn missions_frame(missions: &PhoneMissions) -> Value {
    json!({
        "type": "missions", "items": missions.items, "failed": missions.failed,
        "omitted": missions.omitted,
    })
}

/// The pull requests the phone lists (PRD mobile-companion-ui B37-B45): the
/// projection's trimmed list with the cards the core holds, republished when
/// it changes.
fn prs_frame(prs: &PhonePrs) -> Value {
    json!({
        "type": "prs", "projects": prs.projects, "items": prs.items,
        "feedback": prs.feedback, "omitted": prs.omitted,
    })
}

fn meta_frame(meta: &PhoneMeta) -> Value {
    json!({
        "type": "meta",
        "push_mode": meta.push_mode.as_str(),
        // Other phones on a live connection, for "폰 n대 더 연결됨".
        "other_phones": meta.live_phones.saturating_sub(1),
    })
}

/// Which half of a detail the phone shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum View {
    Conversation,
    Terminal,
}

impl View {
    fn of(message: &Value) -> Option<Self> {
        match message.get("view").and_then(Value::as_str) {
            Some("conversation") | None => Some(Self::Conversation),
            Some("terminal") => Some(Self::Terminal),
            Some(_) => None,
        }
    }
}

/// What the phone asked of its open detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Ask {
    view: View,
    lines: u32,
}

/// What a phone has open: at most one agent's detail, the start sheet, at
/// most one start waiting for the core's answer, and at most one input being
/// written.
#[derive(Default)]
struct Open {
    detail: Option<Detail>,
    /// The sheet gets the catalog as it changes.
    sheet: bool,
    /// Not aborted when the connection ends: the start already reached the
    /// core, and settling its request id lets a reconnecting phone's repeat
    /// read the answer. `start::ANSWER_LIMIT` bounds it.
    start: Option<tokio::task::JoinHandle<()>>,
    /// A reply or key being written, which can take a device pane's whole
    /// connection time. Not aborted when the connection ends, for the same
    /// reason as a start: the write may already be on its way and its request
    /// id must be settled. The pane call's own limits bound it.
    input: Option<tokio::task::JoinHandle<()>>,
    /// The content of the one View the phone is looking at, sent a window of
    /// chunks at a time. It ends with the detail or with the next request:
    /// it only reads, so nothing is owed to a request id.
    look: Option<looks::Transfer>,
}

/// Runs one slow piece of a phone's work beside the socket loop, so the loop
/// keeps answering `ping`, evictions and revokes while it waits, and sends its
/// answer as a frame. One piece per slot: while it runs, the next is answered
/// with `busy` and does not start.
fn spawn_beside_loop(
    slot: &mut Option<tokio::task::JoinHandle<()>>,
    frames: &mpsc::Sender<Value>,
    busy: Value,
    work: impl Future<Output = Value> + Send + 'static,
) -> Option<Value> {
    if slot.as_ref().is_some_and(|task| !task.is_finished()) {
        return Some(busy);
    }
    let frames = frames.clone();
    *slot = Some(tokio::spawn(async move {
        let _ = frames.send(work.await).await;
    }));
    None
}

struct Detail {
    ask: tokio::sync::watch::Sender<Ask>,
    /// The cursors of older conversation pages the phone pulled for.
    older: mpsc::Sender<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Detail {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves the open detail once a second: the conversation while the phone
/// shows it and the pane has one, the pane's rows otherwise, so a pane
/// without a transcript still shows its terminal.
fn spawn_detail(
    mobile: Arc<Mobile>,
    key: AgentKey,
    frames: mpsc::Sender<Value>,
    mut asks: tokio::sync::watch::Receiver<Ask>,
    mut older: mpsc::Receiver<u64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rows = RowsReader::default();
        let mut transcript: Option<Transcript> = None;
        // Whether the phone was last told the pane has a conversation.
        let mut told: Option<bool> = None;
        // The Views the phone was last told the agent has open.
        let mut told_looks: Option<hide_core::LookViews> = None;
        loop {
            let ask = *asks.borrow_and_update();
            let mut outgoing = Vec::new();
            outgoing.extend(looks_step(&mobile, &key, &mut told_looks).await);
            if ask.view == View::Conversation {
                let (next, frame) = conversation_step(&mobile, &key, transcript.take()).await;
                transcript = next;
                outgoing.extend(frame);
                if transcript.is_none() && told != Some(false) {
                    outgoing.push(json!({
                        "type": "conversation", "device_id": key.device_id, "pane_id": key.pane_id, "state": "none",
                    }));
                }
                told = Some(transcript.is_some());
            }
            if ask.view == View::Terminal || transcript.is_none() {
                outgoing.extend(rows.read(&mobile, &key, ask.lines).await);
            }
            for frame in outgoing {
                if frames.send(frame).await.is_err() {
                    return;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(DETAIL_INTERVAL) => {}
                changed = asks.changed() => if changed.is_err() { return },
                Some(cursor) = older.recv() => {
                    let Some(pager) = transcript.as_ref().map(Transcript::pager) else { continue };
                    match tokio::task::spawn_blocking(move || pager.before(cursor)).await {
                        Ok(Ok(page)) => {
                            let frame = json!({
                                "type": "conversation", "device_id": key.device_id, "pane_id": key.pane_id,
                                "state": "ok", "mode": "older", "messages": page.messages, "before": page.before,
                            });
                            if frames.send(frame).await.is_err() { return; }
                        }
                        Ok(Err(error)) => conversation_failed(&error.to_string()),
                        Err(error) => conversation_failed(&error.to_string()),
                    }
                }
            }
        }
    })
}

/// The Views the agent in `pane_id` has open, asked of the core on a blocking
/// thread: its owner thread answers, and no worker of the runtime waits for it.
async fn agent_looks(mobile: &Arc<Mobile>, pane_id: &str) -> Result<hide_core::LookViews, String> {
    let (core, pane_id) = (Arc::clone(mobile.core()), pane_id.to_owned());
    tokio::task::spawn_blocking(move || core.agent_looks(&pane_id))
        .await
        .map_err(|error| error.to_string())?
}

/// The list of Views the agent has open, when it differs from the last one
/// sent: a View opened, closed or looked at moves it.
async fn looks_step(
    mobile: &Arc<Mobile>,
    key: &AgentKey,
    told: &mut Option<hide_core::LookViews>,
) -> Option<Value> {
    let looks = match agent_looks(mobile, &key.pane_id).await {
        Ok(looks) => looks,
        Err(message) => {
            looks_failed(&message);
            return None;
        }
    };
    if told.as_ref() == Some(&looks) {
        return None;
    }
    let frame = looks::list_frame(key, &looks);
    *told = Some(looks);
    Some(frame)
}

fn looks_failed(message: &str) {
    hide_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": "looks.read_failed", "message": message,
    }));
}

fn conversation_failed(message: &str) {
    hide_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": "conversation.read_failed", "message": message,
    }));
}

/// One pass over the pane's conversation: open its transcript and send the
/// newest page, or send what the agent appended. The transcript comes back
/// `None` when the pane has no conversation to show. A failed pane read
/// keeps an open transcript, so a slow answer does not flip the phone to
/// the terminal.
async fn conversation_step(
    mobile: &Arc<Mobile>,
    key: &AgentKey,
    transcript: Option<Transcript>,
) -> (Option<Transcript>, Option<Value>) {
    // An SSH device's transcript is on that device.
    if key.device_id != mobile.node().as_str() {
        return (None, None);
    }
    let reader = Arc::clone(mobile);
    let pane_id = key.engine_pane_id().to_owned();
    let (transcript, page) = tokio::task::spawn_blocking(move || {
        let source = reader
            .engine(reader.node().as_str())
            .and_then(|engine| conversation::source(&engine, &pane_id));
        let source = match source {
            Ok(Some(source)) => source,
            Ok(None) | Err(PaneError::Gone) => return (None, None),
            Err(error) => {
                if let PaneError::Unavailable(message) = &error {
                    conversation_failed(message);
                }
                return (transcript, None);
            }
        };
        if let Some(mut open) = transcript.filter(|open| open.source() == &source) {
            match open.poll() {
                Ok(Tail::Messages(messages)) => {
                    let appended = (!messages.is_empty()).then_some((messages, None, "append"));
                    return (Some(open), appended);
                }
                Ok(Tail::Reset) => {}
                Err(error) => conversation_failed(&error.to_string()),
            }
        }
        match Transcript::open(reader.home(), &pane_id, source) {
            Ok((open, page)) => (Some(open), Some((page.messages, page.before, "reset"))),
            // Reported, and nothing written yet.
            Err(hide_session::SessionError::SessionFileMissing) => (None, None),
            Err(error) => {
                conversation_failed(&error.to_string());
                (None, None)
            }
        }
    })
    .await
    .unwrap_or_else(|error| {
        conversation_failed(&error.to_string());
        (None, None)
    });
    let frame = page.map(|(messages, before, mode)| {
        let mut frame = json!({
            "type": "conversation", "device_id": key.device_id, "pane_id": key.pane_id,
            "state": "ok", "mode": mode, "messages": messages,
        });
        if mode == "reset" {
            frame["before"] = json!(before);
        }
        frame
    });
    (transcript, frame)
}

/// Reads the open pane's rows and answers only when their text or the asked
/// line count changed. The engine's read `revision` counts pane state, not output,
/// so it cannot say that new rows arrived.
#[derive(Default)]
struct RowsReader {
    last: Option<(String, u32)>,
    last_error: Option<&'static str>,
}

impl RowsReader {
    async fn read(&mut self, mobile: &Arc<Mobile>, key: &AgentKey, asked: u32) -> Option<Value> {
        let pane_id = key.engine_pane_id().to_owned();
        let device_id = key.device_id.clone();
        let reader = Arc::clone(mobile);
        // Resolving a device's connection waits on the core owner thread,
        // so it runs on the blocking pool with the read itself.
        let read = tokio::task::spawn_blocking(move || {
            reader
                .engine(&device_id)
                .and_then(|engine| pane::read(&engine, &pane_id, asked))
        })
        .await
        .unwrap_or_else(|error| Err(PaneError::Unavailable(error.to_string())));
        match read {
            Ok(rows) => {
                self.last_error = None;
                if self
                    .last
                    .as_ref()
                    .is_some_and(|(text, lines)| *lines == asked && *text == rows.text)
                {
                    return None;
                }
                let frame = json!({
                    "type": "rows", "device_id": key.device_id, "pane_id": key.pane_id,
                    "state": "ok", "text": rows.text, "lines": asked,
                    // The engine says whether it cut older rows off this read.
                    "more": asked < pane::MAX_LINES && rows.truncated,
                });
                self.last = Some((rows.text, asked));
                Some(frame)
            }
            Err(error) => {
                self.last = None;
                if self.last_error == Some(error.reason()) {
                    return None;
                }
                self.last_error = Some(error.reason());
                if let PaneError::Unavailable(message) = &error {
                    hide_core::diagnostic!(json!({
                        "component": "mobile_phone", "kind": "pane.read_failed", "message": message,
                    }));
                }
                Some(json!({
                    "type": "rows", "device_id": key.device_id, "pane_id": key.pane_id,
                    "state": error.reason(),
                }))
            }
        }
    }
}

fn key_of(message: &Value) -> Option<AgentKey> {
    let device_id = message.get("device_id").and_then(Value::as_str)?;
    let pane_id = message.get("pane_id").and_then(Value::as_str)?;
    (!device_id.is_empty() && device_id.len() <= 256 && !pane_id.is_empty() && pane_id.len() <= 512)
        .then(|| AgentKey {
            device_id: device_id.to_owned(),
            pane_id: pane_id.to_owned(),
        })
}

fn request_id_of(message: &Value) -> Option<&str> {
    message
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// Serves a phone after the daemon counted it toward `MAX_CLIENTS`.
pub async fn serve(
    socket: WebSocket,
    first: Value,
    mobile: Arc<Mobile>,
    connection: u64,
    user_agent: String,
) {
    serve_socket(
        Socket::Direct(Box::new(socket)),
        first,
        mobile,
        connection,
        user_agent,
    )
    .await;
}

pub(super) async fn serve_socket(
    mut socket: Socket,
    first: Value,
    mobile: Arc<Mobile>,
    connection: u64,
    user_agent: String,
) {
    let (phone, paired) = if let Some(code) = first.get("pair").and_then(Value::as_str) {
        let hint = first
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| name.len() <= 200)
            .map(str::to_owned)
            .unwrap_or(user_agent);
        match mobile.pair(code, &hint) {
            Ok((phone, credential)) => {
                let paired = json!({
                    "type": "paired", "credential": credential, "phone_id": phone.id, "name": phone.name,
                });
                (phone, Some(paired))
            }
            Err(refusal) => {
                refuse(&mut socket, refusal.reason()).await;
                return;
            }
        }
    } else if let Some(token) = first.get("token").and_then(Value::as_str) {
        match mobile.authenticate(token) {
            Ok(phone) => (phone, None),
            Err(reason) => {
                refusal_logged("phone.refused", None, reason);
                refuse(&mut socket, reason).await;
                return;
            }
        }
    } else {
        refusal_logged("phone.refused", None, "no_credential");
        refuse(&mut socket, "revoked").await;
        return;
    };
    // Registered before anything is sent, then checked again: a revoke or a
    // switch-off that lands in between closes this socket like any other.
    let close = mobile.register(connection, &phone.id);
    struct Registration(Arc<Mobile>, u64);
    impl Drop for Registration {
        fn drop(&mut self) {
            self.0.unregister(self.1);
        }
    }
    let _registration = Registration(Arc::clone(&mobile), connection);
    if let Err(reason) = mobile.still_admitted(&phone.id) {
        refuse(&mut socket, reason).await;
        return;
    }
    if let Some(paired) = paired
        && !send_frame(&mut socket, &paired, &phone.id).await
    {
        return;
    }
    run(&mut socket, &mobile, connection, &phone.id, close).await;
}

async fn run(
    socket: &mut Socket,
    mobile: &Arc<Mobile>,
    connection: u64,
    phone_id: &str,
    close: Arc<tokio::sync::Notify>,
) {
    let mut projection = mobile.subscribe_projection();
    let mut missions = mobile.subscribe_missions();
    let mut prs = mobile.subscribe_prs();
    let mut meta = mobile.subscribe_meta();
    let (frames_tx, mut frames) = mpsc::channel::<Value>(8);
    let mut open = Open::default();
    let mut catalog = mobile.subscribe_catalog();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heard = tokio::time::Instant::now();
    let record = mobile.phone(phone_id);
    let hello = json!({
        "type": "hello",
        "mac_name": mobile.mac_name(),
        "phone_id": phone_id,
        "name": record.as_ref().map(|phone| phone.name.clone()),
        "vapid_public_key": mobile.vapid_public_key(),
        "notifications": match record.as_ref().map(|phone| (phone.notifications, phone.push.is_some())) {
            Some((_, true)) => "on",
            Some((Notifications::Off, false)) => "off",
            _ => "unasked",
        },
    });
    let mut agents_omitted = 0;
    let (agents, omitted) = agents_frame(&projection.borrow_and_update());
    note_agents_omitted(phone_id, &mut agents_omitted, omitted);
    let first = [
        hello,
        meta_frame(&meta.borrow_and_update()),
        agents,
        missions_frame(&missions.borrow_and_update()),
        prs_frame(&prs.borrow_and_update()),
    ];
    for frame in &first {
        if !send_frame(socket, frame, phone_id).await {
            return;
        }
    }
    loop {
        tokio::select! {
            _ = close.notified() => {
                let reason = if mobile.phone(phone_id).is_some() { "mobile_off" } else { "revoked" };
                refuse(socket, reason).await;
                return;
            }
            changed = projection.changed() => {
                if changed.is_err() { return; }
                let (frame, omitted) = agents_frame(&projection.borrow_and_update());
                note_agents_omitted(phone_id, &mut agents_omitted, omitted);
                if !send_frame(socket, &frame, phone_id).await { return; }
            }
            changed = missions.changed() => {
                if changed.is_err() { return; }
                let frame = missions_frame(&missions.borrow_and_update());
                if !send_frame(socket, &frame, phone_id).await { return; }
            }
            changed = prs.changed() => {
                if changed.is_err() { return; }
                let frame = prs_frame(&prs.borrow_and_update());
                if !send_frame(socket, &frame, phone_id).await { return; }
            }
            changed = meta.changed() => {
                if changed.is_err() { return; }
                let frame = meta_frame(&meta.borrow_and_update());
                if !send_frame(socket, &frame, phone_id).await { return; }
            }
            changed = catalog.changed() => {
                if changed.is_err() { return; }
                let frame = catalog.borrow_and_update().frame();
                if open.sheet && !send_frame(socket, &frame, phone_id).await { return; }
            }
            Some(frame) = frames.recv() => {
                if !send_frame(socket, &frame, phone_id).await { return; }
            }
            _ = ping.tick() => {
                if heard.elapsed() >= SILENT_LIMIT {
                    hide_core::diagnostic!(json!({
                        "component": "mobile_phone", "kind": "phone.silent", "phone_id": phone_id,
                    }));
                    return;
                }
                if !send(socket, Message::Ping(Default::default()), phone_id).await { return; }
            }
            incoming = socket.recv() => {
                heard = tokio::time::Instant::now();
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(Message::Pong(_) | Message::Ping(_))) => continue,
                    Some(Ok(_)) => {
                        refusal_logged("scope.refused", Some(phone_id), "binary");
                        continue;
                    }
                };
                if text.len() > MAX_FRAME {
                    refusal_logged("scope.refused", Some(phone_id), "too_large");
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(&text) else {
                    refusal_logged("scope.refused", Some(phone_id), "not_json");
                    continue;
                };
                // A clone of the current list: a watch borrow must not be held across an await.
                let current = Arc::clone(&projection.borrow());
                let reply = handle(mobile, connection, phone_id, &message, &mut open, &frames_tx, &current).await;
                if let Some(reply) = reply
                    && !send_frame(socket, &reply, phone_id).await
                {
                    return;
                }
            }
        }
    }
}

/// One phone message. The phone's whole vocabulary is here.
async fn handle(
    mobile: &Arc<Mobile>,
    connection: u64,
    phone_id: &str,
    message: &Value,
    open: &mut Open,
    frames: &mpsc::Sender<Value>,
    projection: &Arc<Projection>,
) -> Option<Value> {
    let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "open" => {
            let Some(key) = key_of(message).filter(|key| projection.find(key).is_some()) else {
                refusal_logged("scope.refused", Some(phone_id), "not_an_agent");
                return Some(
                    json!({"type": "rows", "device_id": message.get("device_id"), "pane_id": message.get("pane_id"), "state": "gone"}),
                );
            };
            let Some(view) = View::of(message) else {
                refusal_logged("scope.refused", Some(phone_id), "view");
                return None;
            };
            let (ask, asks) = tokio::sync::watch::channel(Ask {
                view,
                lines: pane::FIRST_LINES,
            });
            let (older, older_asks) = mpsc::channel(1);
            // Opening it is a look: the core reads it as the desktop's focus
            // would while it stays open, and wakes nothing.
            mobile.set_viewing(connection, Some(key.clone()));
            open.look = None;
            let task = spawn_detail(Arc::clone(mobile), key, frames.clone(), asks, older_asks);
            open.detail = Some(Detail { ask, older, task });
            None
        }
        "view" => {
            match (open.detail.as_ref(), View::of(message)) {
                (Some(detail), Some(view)) => detail.ask.send_if_modified(|ask| {
                    let changed = ask.view != view;
                    ask.view = view;
                    changed
                }),
                (_, None) => {
                    refusal_logged("scope.refused", Some(phone_id), "view");
                    false
                }
                (None, Some(_)) => false,
            };
            None
        }
        "more" => {
            if let Some(detail) = open.detail.as_ref() {
                detail.ask.send_modify(|ask| {
                    ask.lines = (ask.lines + pane::MORE_LINES).min(pane::MAX_LINES);
                });
            }
            None
        }
        "older" => {
            match (
                open.detail.as_ref(),
                message.get("before").and_then(Value::as_u64),
            ) {
                // A pull while the last one is still being read is the same pull.
                (Some(detail), Some(cursor)) => {
                    let _ = detail.older.try_send(cursor);
                }
                (_, None) => refusal_logged("scope.refused", Some(phone_id), "older"),
                (None, Some(_)) => {}
            }
            None
        }
        "close" => {
            open.detail = None;
            open.look = None;
            mobile.set_viewing(connection, None);
            None
        }
        "look_open" => look_open(mobile, phone_id, message, open, frames, projection).await,
        "look_ack" => {
            match (
                open.look.as_ref(),
                message.get("request_id").and_then(Value::as_str),
                message.get("seq").and_then(Value::as_u64),
            ) {
                (Some(transfer), Some(request_id), Some(seq)) => transfer.ack(request_id, seq),
                (_, Some(_), Some(_)) => {}
                _ => refusal_logged("scope.refused", Some(phone_id), "look_ack"),
            }
            None
        }
        "look_seen" => {
            look_seen(mobile, phone_id, message, projection).await;
            None
        }
        "wake_agent" => Some(wake_agent(mobile, phone_id, message, projection).await),
        "answer_mission" => Some(answer_mission(mobile, phone_id, message).await),
        "read_pr" | "merge_pr" | "read_pr_feedback" => {
            Some(pr_request(mobile, phone_id, kind, message).await)
        }
        "delegate_pr" => {
            let busy = json!({"type": "start_result", "request_id": message.get("request_id"), "ok": false, "reason": "in_flight"});
            // Answered once the core says how the start went, like a start from the sheet.
            let (mobile, phone_id, message) =
                (Arc::clone(mobile), phone_id.to_owned(), message.clone());
            spawn_beside_loop(&mut open.start, frames, busy, async move {
                mobile.delegate_pr(&phone_id, &message).await
            })
        }
        "input" => {
            let busy = json!({
                "type": "input_result", "request_id": message.get("request_id"), "ok": false, "reason": "in_flight",
            });
            let (mobile, phone_id, message, projection) = (
                Arc::clone(mobile),
                phone_id.to_owned(),
                message.clone(),
                Arc::clone(projection),
            );
            spawn_beside_loop(&mut open.input, frames, busy, async move {
                input(&mobile, &phone_id, &message, &projection).await
            })
        }
        "wake_sleeping_session" | "check_sleeping_session" => {
            Some(sleeping_session(mobile, phone_id, message, projection).await)
        }
        "start_sheet" => {
            let Some(shown) = message.get("open").and_then(Value::as_bool) else {
                refusal_logged("scope.refused", Some(phone_id), "start_sheet");
                return None;
            };
            open.sheet = shown;
            mobile.set_start_sheet(connection, shown);
            shown.then(|| mobile.subscribe_catalog().borrow().frame())
        }
        "start_agent" => {
            let busy = json!({"type": "start_result", "request_id": message.get("request_id"), "ok": false, "reason": "in_flight"});
            // The answer can take the core's whole creation time, so it is
            // awaited beside the socket loop and arrives as a frame.
            let (mobile, phone_id, message) =
                (Arc::clone(mobile), phone_id.to_owned(), message.clone());
            spawn_beside_loop(&mut open.start, frames, busy, async move {
                mobile.start_agent(&phone_id, &message).await
            })
        }
        "ping" => liveness(message, mobile.still_admitted(phone_id), phone_id),
        "push_subscription" => {
            let subscription = message.get("subscription");
            let field = |name: &str| {
                subscription
                    .and_then(|value| value.pointer(name))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let push = PushSubscription {
                endpoint: field("/endpoint"),
                p256dh: field("/keys/p256dh"),
                auth: field("/keys/auth"),
            };
            let stored = mobile.set_subscription(phone_id, push);
            Some(
                json!({"type": "push_state", "notifications": if stored { "on" } else { "refused" }}),
            )
        }
        "push_permission" => {
            let notifications = match message.get("state").and_then(Value::as_str) {
                Some("denied") => Notifications::Off,
                Some("default") => Notifications::Unasked,
                _ => {
                    refusal_logged("scope.refused", Some(phone_id), "push_permission");
                    return None;
                }
            };
            mobile.set_notifications(phone_id, notifications);
            None
        }
        other => {
            // A file, a setting, a project or device change, the snapshot:
            // nothing a phone credential may ask for (B17).
            refusal_logged(
                "scope.refused",
                Some(phone_id),
                &format!("type:{}", other.chars().take(64).collect::<String>()),
            );
            Some(
                json!({"type": "refused_request", "request": other.chars().take(64).collect::<String>(), "reason": "not_allowed"}),
            )
        }
    }
}

/// The answer to a `look_*` request that cannot go on: its state says why.
fn look_refused(message: &Value, state: &str) -> Value {
    json!({
        "type": "look_result", "request_id": message.get("request_id"),
        "view_id": message.get("view_id"), "state": state,
    })
}

/// Why a `look_*` request was turned away: the `state` its answer carries and
/// the `reason` `scope.refused` records.
#[derive(Debug, Eq, PartialEq)]
struct LookRefusal {
    state: &'static str,
    reason: String,
}

/// The first checks of a `look_open` or `look_seen`, which need nothing from
/// the core (PRD mobile-companion-ui D-16): the request carries no field
/// beyond the ids, and the agent is one the phone's list holds.
fn look_scope<'a>(
    message: &'a Value,
    projection: &Projection,
) -> Result<(AgentKey, looks::Request<'a>), LookRefusal> {
    let request = looks::request(message).map_err(|error| match error {
        looks::Malformed::Beyond(field) => LookRefusal {
            state: "refused",
            reason: format!("look_field:{field}"),
        },
        looks::Malformed::Invalid => LookRefusal {
            state: "refused",
            reason: "look_request".to_owned(),
        },
    })?;
    let key = key_of(message)
        .filter(|key| projection.find(key).is_some())
        .ok_or_else(|| LookRefusal {
            state: "gone",
            reason: "not_an_agent".to_owned(),
        })?;
    Ok((key, request))
}

/// The View a `look_open` or `look_seen` names, checked in the order that
/// keeps the phone to the Views of an agent it can see: `look_scope`, the
/// phone still let in, and the View among the Views the core lists that agent
/// has open now. Every refusal is recorded as `scope.refused`; `Err` is the
/// answer to send.
async fn look_target(
    mobile: &Arc<Mobile>,
    phone_id: &str,
    message: &Value,
    projection: &Projection,
) -> Result<(String, hide_core::LookView), Value> {
    let (key, request) = match look_scope(message, projection) {
        Ok(scope) => scope,
        Err(refusal) => {
            refusal_logged("scope.refused", Some(phone_id), &refusal.reason);
            return Err(look_refused(message, refusal.state));
        }
    };
    if mobile.still_admitted(phone_id).is_err() {
        return Err(look_refused(message, "refused"));
    }
    let looks = match agent_looks(mobile, &key.pane_id).await {
        Ok(looks) => looks,
        Err(failure) => {
            looks_failed(&failure);
            return Err(look_refused(message, "unavailable"));
        }
    };
    match looks::find(&looks, request.view_id) {
        Some(view) => Ok((request.request_id.to_owned(), view)),
        None => {
            // Closed since the list was drawn, or never this agent's.
            refusal_logged("scope.refused", Some(phone_id), "view_not_open");
            Err(look_refused(message, "closed"))
        }
    }
}

/// `look_open`: starts sending the View's content; the next one replaces it.
async fn look_open(
    mobile: &Arc<Mobile>,
    phone_id: &str,
    message: &Value,
    open: &mut Open,
    frames: &mpsc::Sender<Value>,
    projection: &Arc<Projection>,
) -> Option<Value> {
    let (request_id, view) = match look_target(mobile, phone_id, message, projection).await {
        Ok(target) => target,
        Err(answer) => return Some(answer),
    };
    if request_id.is_empty() {
        refusal_logged("scope.refused", Some(phone_id), "look_request");
        return Some(look_refused(message, "refused"));
    }
    open.look = Some(looks::open(mobile, &request_id, view, frames.clone()));
    None
}

/// `look_seen`: the phone showed the View, so it is seen, on the phone and on
/// the desktop (B24). Repeating it changes nothing.
/// The dispatch waits on the core's owner thread, so it runs on a blocking
/// thread like every other dispatch here.
async fn look_seen(mobile: &Arc<Mobile>, phone_id: &str, message: &Value, projection: &Projection) {
    if let Ok((_, view)) = look_target(mobile, phone_id, message, projection).await {
        let mobile = Arc::clone(mobile);
        if let Err(error) = tokio::task::spawn_blocking(move || mobile.mark_seen(&view)).await {
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.seen_unrecorded",
                "message": error.to_string(),
            }));
        }
    }
}

/// The answer to a phone's `ping`: its own number back, so the phone knows
/// hided is there. The page cannot see the protocol-level ping hided sends, so
/// this is how a phone learns that a Mac gone to sleep stopped answering. It
/// reads one in-memory record and nothing else: no core lock, no engine call.
/// A phone no longer let in is told why instead, as it is when its socket is
/// closed, and a ping without a number is refused and recorded.
fn liveness(message: &Value, admitted: Result<(), &'static str>, phone_id: &str) -> Option<Value> {
    if let Err(reason) = admitted {
        return Some(json!({"type": "refused", "reason": reason}));
    }
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        refusal_logged("scope.refused", Some(phone_id), "ping");
        return None;
    };
    Some(json!({"type": "pong", "id": id}))
}

/// The phone supplies a stable intent, never a pane, path or arbitrary event.
/// The core repeats the current-generation and exact-record checks at effect.
async fn sleeping_session(
    mobile: &Arc<Mobile>,
    phone_id: &str,
    message: &Value,
    projection: &Projection,
) -> Value {
    let answer = |ok: bool, reason: Option<&str>| {
        json!({
            "type": "sleep_result", "request_id": message.get("request_id"),
            "sleep_id": message.get("sleep_id"), "ok": ok, "reason": reason,
        })
    };
    let Some(_) = request_id_of(message) else {
        return answer(false, Some("invalid_request"));
    };
    let Some((id, event)) = sleep_action(message, projection, mobile.node().as_str()) else {
        refusal_logged("scope.refused", Some(phone_id), "sleep_not_available");
        return answer(false, Some("unavailable"));
    };
    if mobile.still_admitted(phone_id).is_err() {
        return answer(false, Some("unavailable"));
    }
    let bytes = json!({"schema_version": hide_core::SCHEMA_VERSION, "kind": event, "payload": {"sleep_id": id}})
        .to_string()
        .into_bytes();
    let (mobile, phone_id) = (Arc::clone(mobile), phone_id.to_owned());
    let dispatched = tokio::task::spawn_blocking(move || {
        mobile.still_admitted(&phone_id).map_err(str::to_owned)?;
        mobile.config.core.dispatch(bytes)
    })
    .await;
    match dispatched {
        Ok(Ok(())) => answer(true, None),
        _ => answer(false, Some("unavailable")),
    }
}

/// What a phone's `answer_mission` asks, once it names a listed mission and a
/// well-formed answer: the one `mission_answer` event the desktop's decision
/// banner sends (`Event::MissionAnswer`), nothing else.
fn mission_answer_event(
    message: &Value,
    missions: &PhoneMissions,
) -> Result<Vec<u8>, &'static str> {
    let id = message.get("mission").and_then(Value::as_str);
    let Some(mission) = id.and_then(|id| missions.find(id)) else {
        return Err("gone");
    };
    let Some(key) = message.get("key").and_then(Value::as_u64) else {
        return Err("invalid_request");
    };
    let resend = match message.get("resend") {
        None => false,
        Some(Value::Bool(resend)) => *resend,
        Some(_) => return Err("invalid_request"),
    };
    let option = match message.get("option") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|option| u8::try_from(option).ok())
                .filter(|option| *option >= 1)
                .ok_or("invalid_request")?,
        ),
    };
    let text = match message.get("text") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => return Err("invalid_request"),
    };
    // Sending again names no answer; a first answer names exactly one.
    match (resend, option, text) {
        (true, None, None) => {}
        (false, Some(_), None) => {}
        (false, None, Some(text)) if hide_core::missions::answer_text(text).is_some() => {}
        _ => return Err("invalid_request"),
    }
    Ok(json!({
        "schema_version": hide_core::SCHEMA_VERSION, "kind": "mission_answer",
        "payload": {"mission": mission.id, "key": key, "option": option, "text": text, "resend": resend},
    })
    .to_string()
    .into_bytes())
}

/// The operator's answer to the decision a mission waits on, from a phone.
/// One claim per request id, taken and settled on one blocking thread, so a
/// repeat of the same press, on this socket or another, reaches the Conductor
/// once; the core then keeps the first answer to the question's key. A
/// refusal the operator can act on (a changed question, no Conductor, a
/// device that stopped answering) comes back on the mission itself, in the
/// desktop's words, as it does for the banner.
async fn answer_mission(mobile: &Arc<Mobile>, phone_id: &str, message: &Value) -> Value {
    let Some(request_id) = request_id_of(message) else {
        return json!({"type": "mission_result", "ok": false, "reason": "invalid_request"});
    };
    let answer = |ok: bool, reason: Option<&str>| {
        json!({
            "type": "mission_result", "request_id": request_id,
            "mission": message.get("mission").and_then(Value::as_str), "ok": ok, "reason": reason,
        })
    };
    let bytes = match mission_answer_event(message, &mobile.missions()) {
        Ok(bytes) => bytes,
        Err(reason) => {
            refusal_logged(
                "scope.refused",
                Some(phone_id),
                &format!("mission_answer:{reason}"),
            );
            return answer(false, Some(reason));
        }
    };
    let (mobile, phone_id, request) = (
        Arc::clone(mobile),
        phone_id.to_owned(),
        request_id.to_owned(),
    );
    let dispatched = tokio::task::spawn_blocking(move || {
        mobile.still_admitted(&phone_id)?;
        match mobile.reserve_input(&phone_id, &request) {
            Reservation::Write => {}
            Reservation::Seen(InputState::Written) => return Ok(()),
            Reservation::Seen(_) => return Err("in_flight"),
        }
        match mobile.config.core.dispatch(bytes) {
            Ok(()) => {
                mobile.settle_input(&phone_id, &request, Some(InputState::Written));
                Ok(())
            }
            Err(_) => {
                // Nothing reached the core, so the phone may send this id again.
                mobile.settle_input(&phone_id, &request, None);
                Err("unavailable")
            }
        }
    })
    .await;
    match dispatched {
        Ok(Ok(())) => answer(true, None),
        Ok(Err(reason)) => answer(false, Some(reason)),
        Err(_) => answer(false, Some("unavailable")),
    }
}

/// A phone's read, merge or feedback request for a listed pull request: the
/// one core event the desktop's card sends, claimed by its request id on one
/// blocking thread, so a repeat of the same press (on this socket or another)
/// reaches GitHub once. What GitHub then did comes back on the card, in the
/// `prs` frame; this answers only whether hided sent the request.
async fn pr_request(mobile: &Arc<Mobile>, phone_id: &str, kind: &str, message: &Value) -> Value {
    let Some(request_id) = request_id_of(message) else {
        return json!({"type": "pr_result", "action": kind, "ok": false, "reason": "invalid_request"});
    };
    let answer = |ok: bool, reason: Option<&str>| {
        json!({
            "type": "pr_result", "request_id": request_id, "action": kind,
            "pr": message.get("pr").and_then(Value::as_str), "ok": ok, "reason": reason,
        })
    };
    let listed = mobile.prs();
    let built = match kind {
        "read_pr" => prs::card_read(message, &listed),
        "merge_pr" => prs::merge(message, &listed),
        _ => prs::feedback(message, &listed, request_id),
    };
    let bytes = match built {
        Ok(bytes) => bytes,
        Err(reason) => {
            refusal_logged("scope.refused", Some(phone_id), &format!("{kind}:{reason}"));
            return answer(false, Some(reason));
        }
    };
    // Only a merge is an effect to claim: a read sent twice reads twice.
    let claims = kind == "merge_pr";
    let (mobile, phone_id, request) = (
        Arc::clone(mobile),
        phone_id.to_owned(),
        request_id.to_owned(),
    );
    let dispatched = tokio::task::spawn_blocking(move || {
        mobile.still_admitted(&phone_id)?;
        if claims {
            match mobile.reserve_input(&phone_id, &request) {
                Reservation::Write => {}
                Reservation::Seen(InputState::Written) => return Ok(()),
                Reservation::Seen(_) => return Err("in_flight"),
            }
        }
        match mobile.config.core.dispatch(bytes) {
            Ok(()) => {
                if claims {
                    mobile.settle_input(&phone_id, &request, Some(InputState::Written));
                }
                Ok(())
            }
            Err(_) => {
                // Nothing reached the core, so the phone may send this id again.
                if claims {
                    mobile.settle_input(&phone_id, &request, None);
                }
                Err("unavailable")
            }
        }
    })
    .await;
    match dispatched {
        Ok(Ok(())) => answer(true, None),
        Ok(Err(reason)) => answer(false, Some(reason)),
        Err(_) => answer(false, Some("unavailable")),
    }
}

/// Wake agent, Retry and Start new session from a phone: the desktop's own
/// `agent_wake` for a pane the projection says holds a sleeping or failed
/// agent. The phone names the pane it sees; the core checks its record again.
async fn wake_agent(
    mobile: &Arc<Mobile>,
    phone_id: &str,
    message: &Value,
    projection: &Projection,
) -> Value {
    let key = key_of(message);
    let answer = |ok: bool, reason: Option<&str>| {
        json!({
            "type": "sleep_result", "request_id": message.get("request_id"),
            "sleep_id": key.as_ref().map(AgentKey::tag), "ok": ok, "reason": reason,
        })
    };
    let (Some(_), Some(key)) = (request_id_of(message), key.as_ref()) else {
        return answer(false, Some("invalid_request"));
    };
    let bytes = match wake_action(message, projection, key) {
        Some(Wake::Dispatch(bytes)) => bytes,
        // A wake already running is the same intent and starts nothing.
        Some(Wake::Running) => return answer(true, None),
        None => {
            refusal_logged("scope.refused", Some(phone_id), "wake_not_available");
            return answer(false, Some("unavailable"));
        }
    };
    if mobile.still_admitted(phone_id).is_err() {
        return answer(false, Some("unavailable"));
    }
    let (mobile, phone_id) = (Arc::clone(mobile), phone_id.to_owned());
    let dispatched = tokio::task::spawn_blocking(move || {
        mobile.still_admitted(&phone_id).map_err(str::to_owned)?;
        mobile.config.core.dispatch(bytes)
    })
    .await;
    match dispatched {
        Ok(Ok(())) => answer(true, None),
        _ => answer(false, Some("unavailable")),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Wake {
    Dispatch(Vec<u8>),
    Running,
}

/// What a phone's wake does: the `agent_wake` event, nothing when the agent
/// is already waking, and no action when the pane holds no sleeping agent.
fn wake_action(message: &Value, projection: &Projection, key: &AgentKey) -> Option<Wake> {
    if !message.as_object()?.keys().all(|name| {
        matches!(
            name.as_str(),
            "type" | "request_id" | "device_id" | "pane_id" | "fresh"
        )
    }) {
        return None;
    }
    let fresh = match message.get("fresh") {
        None => false,
        Some(value) => value.as_bool()?,
    };
    let sleep = projection.find(key)?.sleep.as_ref()?;
    if sleep.state == "waking" {
        return Some(Wake::Running);
    }
    let event = json!({
        "schema_version": hide_core::SCHEMA_VERSION, "kind": "agent_wake",
        "payload": {"pane_id": key.pane_id, "fresh": fresh},
    });
    Some(Wake::Dispatch(event.to_string().into_bytes()))
}

fn sleep_action<'a>(
    message: &'a Value,
    projection: &Projection,
    node: &str,
) -> Option<(SleepId, &'a str)> {
    // Reject hidden address/lineage overrides instead of silently dropping them.
    if !message
        .as_object()?
        .keys()
        .all(|key| matches!(key.as_str(), "type" | "sleep_id" | "request_id"))
    {
        return None;
    }
    let id = serde_json::from_value::<SleepId>(message.get("sleep_id")?.clone()).ok()?;
    let row = projection
        .sleeping(&id)
        .filter(|row| row.device_id == node)?;
    let event = message.get("type")?.as_str()?;
    match event {
        "wake_sleeping_session" if row.wake_available => Some((id, event)),
        "check_sleeping_session"
            if !row.checking
                && matches!(
                    row.phase,
                    DormantPhase::CloseUnknown | DormantPhase::WakeUnknown
                ) =>
        {
            Some((id, event))
        }
        _ => None,
    }
}

async fn input(
    mobile: &Arc<Mobile>,
    phone_id: &str,
    message: &Value,
    projection: &Projection,
) -> Value {
    let Some(request_id) = request_id_of(message) else {
        return json!({"type": "input_result", "ok": false, "reason": "invalid_request"});
    };
    let answer = |ok: bool, reason: Option<&str>| json!({"type": "input_result", "request_id": request_id, "ok": ok, "reason": reason});
    let Some(key) = key_of(message).filter(|key| projection.find(key).is_some()) else {
        refusal_logged("scope.refused", Some(phone_id), "input_not_an_agent");
        return answer(false, Some("gone"));
    };
    let text = message.get("text").and_then(Value::as_str);
    let engine_key = message.get("key").and_then(Value::as_str);
    let problem = match (text, engine_key) {
        (Some(text), None) => pane::reply_problem(text),
        (None, Some(key)) => pane::engine_key(key).is_none().then_some("unknown_key"),
        _ => Some("invalid_request"),
    };
    if let Some(problem) = problem {
        return answer(false, Some(problem));
    }
    // One claim per request id, taken before writing: a repeat that arrives
    // while the first is still being written, or after the engine's answer was
    // lost, is answered from the claim and never written again (B27).
    match mobile.reserve_input(phone_id, request_id) {
        Reservation::Write => {}
        Reservation::Seen(InputState::Written) => return answer(true, None),
        Reservation::Seen(InputState::InFlight) => return answer(false, Some("in_flight")),
        Reservation::Seen(InputState::Uncertain) => return answer(false, Some("uncertain")),
    }
    let pane_id = key.engine_pane_id().to_owned();
    let core_pane_id = key.pane_id.clone();
    let device_id = key.device_id.clone();
    let writer = Arc::clone(mobile);
    let text = text.map(str::to_owned);
    let engine_key = engine_key.and_then(pane::engine_key);
    // The blocking write owns settlement even if its async caller is cancelled.
    // A panic or a queued closure dropped at shutdown conservatively settles Uncertain.
    struct Settlement {
        mobile: Arc<Mobile>,
        phone_id: String,
        request_id: String,
        state: Option<InputState>,
    }
    impl Drop for Settlement {
        fn drop(&mut self) {
            self.mobile
                .settle_input(&self.phone_id, &self.request_id, self.state);
        }
    }
    let mut settlement = Settlement {
        mobile: Arc::clone(mobile),
        phone_id: phone_id.to_owned(),
        request_id: request_id.to_owned(),
        state: Some(InputState::Uncertain),
    };
    let (certain, result) = tokio::task::spawn_blocking(move || {
        let (attempted, result) = (|| {
            let input = match (&text, engine_key) {
                (Some(text), _) => Input::Reply(text),
                (None, Some(key)) => Input::Key(key),
                (None, None) => unreachable!("validated above"),
            };
            if let Err(error) = pane::wait_for_input(writer.terminals(), &core_pane_id, &input) {
                return (false, Err(error));
            }
            // Asked here, at the write, and held until it returns: a reply queued
            // while the agent was awake is refused if its pane has closed since,
            // and a sleep that starts now waits for this write to land first. The
            // phone's own list may be a second behind either way.
            let _admission = match writer.admit_input(&core_pane_id) {
                Ok(admission) => admission,
                Err(error) => return (false, Err(error)),
            };
            let engine = match writer.engine(&device_id) {
                Ok(engine) => engine,
                Err(error) => return (false, Err(error)),
            };
            let reply = matches!(input, Input::Reply(_));
            // Before the write, so the doorbell cannot judge the pane clean
            // while the key is landing; a refused write only holds the pane.
            writer.note_input(&core_pane_id);
            let sent = pane::send(&engine, &pane_id, input);
            if reply && sent.is_ok() {
                writer.note_submit(&core_pane_id);
            }
            (true, sent)
        })();
        // A refusal before the write, or a definitive gone/refused answer,
        // releases the id. Every ambiguous write keeps it reserved.
        let certain = !attempted
            || matches!(
                result,
                Err(PaneError::Gone
                    | PaneError::DeviceUnreachable
                    | PaneError::InputRefused
                    | PaneError::Asleep)
            );
        settlement.state = match &result {
            Ok(()) => Some(InputState::Written),
            Err(_) if certain => None,
            Err(_) => Some(InputState::Uncertain),
        };
        drop(settlement);
        (certain, result)
    })
    .await
    .unwrap_or_else(|error| (false, Err(PaneError::Unavailable(error.to_string()))));
    match result {
        Ok(()) => {
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "input.sent", "phone_id": phone_id,
                "device_id": key.device_id, "kind_of_input": if message.get("key").is_some() { "key" } else { "reply" },
            }));
            answer(true, None)
        }
        Err(error) => {
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "input.failed", "phone_id": phone_id,
                "reason": error.reason(),
                "message": match &error { PaneError::Unavailable(message) => Some(message.clone()), _ => None },
            }));
            let reason = if certain { error.reason() } else { "uncertain" };
            answer(false, Some(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_core::missions::ANSWER_LIMIT;

    #[test]
    fn a_look_request_names_an_agent_and_a_view_id_and_nothing_a_path_could_ride_in() {
        let projection = sleeping_projection(None);
        let ask = |field: Option<&str>, pane: &str| {
            let mut message = json!({
                "type": "look_open", "request_id": "r-1", "device_id": "test-node",
                "pane_id": pane, "view_id": "view-1",
            });
            if let Some(field) = field {
                message[field] = json!("/etc/passwd");
            }
            message
        };
        let refused = |message: &Value| look_scope(message, &projection).err();
        assert!(look_scope(&ask(None, "w1:p1"), &projection).is_ok());
        for field in ["path", "root", "checkout", "file"] {
            assert_eq!(
                refused(&ask(Some(field), "w1:p1")),
                Some(LookRefusal {
                    state: "refused",
                    reason: format!("look_field:{field}")
                })
            );
        }
        // An agent the list does not hold is gone, whatever View it names.
        assert_eq!(
            refused(&ask(None, "w9:p9")),
            Some(LookRefusal {
                state: "gone",
                reason: "not_an_agent".to_owned()
            })
        );
        let mut no_view = ask(None, "w1:p1");
        no_view.as_object_mut().unwrap().remove("view_id");
        assert_eq!(
            refused(&no_view).map(|refusal| refusal.reason),
            Some("look_request".to_owned())
        );
    }

    #[test]
    fn a_phone_can_act_only_on_a_current_local_sleep_intent() {
        let mut row = json!({
            "sleep_id": "sleep-a-1", "node_id": "local", "kind": "pi",
            "identity_label": "Review parser", "group": "seen", "phase": "sleeping",
            "since_unix_ms": 1, "wake_available": true, "checking": false, "reason": null,
        });
        let projection = |row: &Value| {
            super::super::projection::project(
                &json!({"navigator": {"agents": [], "sleeping_sessions": [row]}}),
                "local",
            )
        };
        let mut message = json!({"type": "wake_sleeping_session", "sleep_id": "sleep-a-1", "request_id": "request-1"});
        assert!(sleep_action(&message, &projection(&row), "local").is_some());
        assert!(sleep_action(&message, &projection(&row), "other").is_none());
        message["pane_id"] = json!("retired-pane");
        assert!(sleep_action(&message, &projection(&row), "local").is_none());
        message.as_object_mut().unwrap().remove("pane_id");
        message["sleep_id"] = json!("sleep-a-2");
        assert!(sleep_action(&message, &projection(&row), "local").is_none());
        message["sleep_id"] = json!("sleep-a-1");
        row["wake_available"] = json!(false);
        row["phase"] = json!("wake_unknown");
        assert!(sleep_action(&message, &projection(&row), "local").is_none());
        message["type"] = json!("check_sleeping_session");
        assert!(sleep_action(&message, &projection(&row), "local").is_some());
        row["checking"] = json!(true);
        assert!(sleep_action(&message, &projection(&row), "local").is_none());
    }

    fn sleeping_projection(state: Option<&str>) -> Projection {
        let mut agent = json!({
            "pane_id": "w1:p1", "group": "seen", "mark": "asleep", "identity_label": "Review parser",
            "agent_kind": "claude", "status_code": "sleeping", "demand": "none",
        });
        if let Some(state) = state {
            agent["sleep"] = json!({
                "state": state, "reason": null, "since_unix_ms": 1, "progress": null,
            });
        }
        super::super::projection::project(&json!({"navigator": {"agents": [agent]}}), "test-node")
    }

    fn pane() -> AgentKey {
        AgentKey {
            device_id: "test-node".to_owned(),
            pane_id: "w1:p1".to_owned(),
        }
    }

    fn mobile_with(
        directory: &std::path::Path,
        gate: &Arc<hide_core::input_gate::PaneInputGate>,
    ) -> Arc<Mobile> {
        let core = Arc::new(crate::core::CoreHandle::bare(directory));
        let boundary = Arc::new(crate::boundary::Boundary::new(directory).unwrap());
        Mobile::start(super::super::Config {
            state_dir: directory.join("state"),
            home: directory.to_path_buf(),
            port: 0,
            relay_url: crate::env::RELAY_URL.to_owned(),
            cli: super::super::tailscale::CliSource {
                pinned: None,
                search_path: None,
            },
            host_name: None,
            core: Arc::clone(&core),
            input_gate: Arc::clone(gate),
            renderers: Arc::default(),
            start_demand: Arc::default(),
            // These tests drive the phone boundary directly, never a network supervisor.
            open: tokio::sync::watch::channel(false).1,
            desktop: Arc::default(),
            boundary: Arc::clone(&boundary),
            roots: Arc::new(crate::RootFollower::new(
                Arc::clone(&core),
                boundary,
                Arc::new(crate::watch::WatchService::new(Arc::clone(&core))),
                Arc::new(crate::index::IndexService::new()),
            )),
            browser_control: Arc::default(),
        })
    }

    fn input_message(request_id: &str, body: Value) -> Value {
        let mut message = json!({
            "type": "input", "request_id": request_id, "device_id": "test-node", "pane_id": "w1:p1",
        });
        message
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        message
    }

    fn reason(answer: &Value) -> Option<&str> {
        answer["reason"].as_str()
    }

    use hide_node_link::terminal::{
        GridSize, InputPermitError, InputPermitReply, KeyTarget, TerminalControl, TerminalNode,
        TerminalRoutes,
    };
    struct Pending(tokio::sync::mpsc::UnboundedSender<InputPermitReply>);
    impl TerminalNode for Pending {
        fn input_permit(&self, pane: &str, _: usize, reply: InputPermitReply) {
            assert_eq!(pane, "w1:p1");
            assert!(self.0.send(reply).is_ok());
        }
        fn control(&self, _: TerminalControl) {}
        fn key(&self, _: KeyTarget, _: Vec<u8>, _: u64) {
            panic!("no raw phone keys");
        }
        fn view(&self, _: &str, _: GridSize, _: bool) {
            panic!("no phone attach");
        }
        fn redraw(&self, _: &str) {
            panic!("no phone redraw");
        }
    }

    #[tokio::test]
    async fn a_relay_connection_lost_during_input_does_not_leave_the_retry_in_flight() {
        use hide_relay_protocol::noise::{NoiseKeypair, Session};
        async fn connect(
            mobile: Arc<Mobile>,
            credential: &str,
            id: u32,
        ) -> (
            tokio::task::JoinHandle<()>,
            mpsc::Sender<Vec<u8>>,
            mpsc::Receiver<super::super::relay::Output>,
            Session,
        ) {
            let key = NoiseKeypair::generate().unwrap();
            let mut responder = Session::responder(&key, id).unwrap();
            let mut initiator = Session::initiator(&key.public_key(), id).unwrap();
            responder
                .read_handshake(&initiator.write_handshake().unwrap())
                .unwrap();
            initiator
                .read_handshake(&responder.write_handshake().unwrap())
                .unwrap();
            let (incoming, rx) = mpsc::channel(8);
            let (tx, outgoing) = mpsc::channel(32);
            let socket = Socket::relay(responder, rx, tx, id);
            let auth = json!({"client_kind":"phone", "token":credential});
            let task = tokio::spawn(serve_socket(
                socket,
                auth,
                mobile,
                u64::from(id),
                String::new(),
            ));
            (task, incoming, outgoing, initiator)
        }
        async fn send(tx: &mpsc::Sender<Vec<u8>>, session: &mut Session, input: &Value) {
            for frame in session.send_message(input.to_string().as_bytes()).unwrap() {
                tx.send(frame).await.unwrap();
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let (asked, mut pending) = tokio::sync::mpsc::unbounded_channel();
        mobile
            .config
            .core
            .terminals
            .install_device("mini", Arc::new(Pending(asked)));
        let listed = super::super::projection::project(
            &json!({
                "navigator":{"agents":[]}, "status":{"remote":[{"target_id":"mini","state":"connected","session":{
                    "agents":[{"pane_id":"remote:mini:pane:w1:p1","group":"seen","mark":"quiet","identity_label":"Review","agent_kind":"pi","status_code":"idle","demand":"none"}]
                }}]}
            }),
            "test-node",
        );
        mobile.projection.send_replace(Arc::new(listed));
        let code = {
            let mut inner = mobile.lock();
            inner.settings.enabled = true;
            inner.exposure = super::super::Exposure::Relay;
            inner.phones.new_code(super::super::now_ms());
            inner.phones.code().unwrap().0.to_owned()
        };
        let (_, credential) = mobile.pair(&code, "Phone").unwrap();
        let input = json!({"type":"input","request_id":"same-intent","device_id":"mini","pane_id":"remote:mini:pane:w1:p1","text":"yes"});
        let (first, tx, _rx, mut phone) = connect(Arc::clone(&mobile), &credential, 1).await;
        send(&tx, &mut phone, &input).await;
        let permission = tokio::time::timeout(Duration::from_secs(20), pending.recv())
            .await
            .unwrap()
            .unwrap();
        // This is the cancellation the relay owner performs on link loss.
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        drop(tx);
        permission(Err(InputPermitError::Discarded));
        let (second, tx, mut rx, mut phone) = connect(Arc::clone(&mobile), &credential, 2).await;
        send(&tx, &mut phone, &input).await;
        let mut retried_writes = 0;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                tokio::select! {
                    permission = pending.recv() => {
                        retried_writes += 1;
                        permission.unwrap()(Err(InputPermitError::Discarded));
                    }
                    frame = rx.recv() => {
                        let Some(super::super::relay::Output::Frame { bytes, .. }) = frame else { panic!("relay phone closed"); };
                        if let Some(plain) = phone.recv_message(&bytes, 0).unwrap() {
                            let answer: Value = serde_json::from_slice(&plain).unwrap();
                            if answer["type"] != "input_result" { continue; }
                            if answer["reason"] == "in_flight" { send(&tx, &mut phone, &input).await; continue; }
                            assert_eq!(answer["reason"], "input_refused");
                            break;
                        }
                    }
                }
            }
        }).await.expect("the retry receives the settled refusal, not permanent in_flight");
        assert_eq!(retried_writes, 1);
        second.abort();
        let _ = second.await;
        mobile.shutdown().await;
    }

    /// B21: a page whose screen cannot be taken (no desktop window here, no
    /// such checkout) is a refusal with its reason, which `take` logs, and
    /// never a picture or a hang; the phone is answered with the address.
    #[tokio::test]
    async fn a_page_with_no_desktop_window_cannot_be_shot() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let view = hide_core::LookView {
            device_id: "test-node".to_owned(),
            checkout_path: "/repo".to_owned(),
            display_id: "browser-1".to_owned(),
            kind: hide_core::DisplayKind::Browser,
            label: "landing".to_owned(),
            path: String::new(),
            url: Some("http://localhost:5173/".to_owned()),
            committed: None,
            output: None,
            unseen: true,
            at_unix_ms: 1,
        };
        let reason = super::super::page_shot::take(&mobile, &view, super::super::looks::IMAGE_CAP)
            .await
            .err()
            .expect("no window, no picture");
        assert!(
            !reason.is_empty(),
            "the refusal names its reason for the log"
        );
    }

    #[tokio::test]
    async fn refused_permissions_allow_retry_and_waiting_permissions_do_not_block_sleep() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let (asked, mut pending) = tokio::sync::mpsc::unbounded_channel();
        mobile
            .config
            .core
            .terminals
            .install_device("mini", Arc::new(Pending(asked)));
        for attempt in 0..3 {
            let writer = Arc::clone(&mobile);
            let task = tokio::spawn(async move {
                let listed = super::super::projection::project(
                    &json!({
                        "navigator": {"agents": []},
                        "status": {"remote": [{"target_id": "mini", "state": "connected", "session": {
                            "agents": [{"pane_id": "remote:mini:pane:w1:p1", "group": "seen", "mark": "quiet", "identity_label": "Review", "agent_kind": "pi", "status_code": "idle", "demand": "none"}]
                        }}]}
                    }),
                    "test-node",
                );
                input(&writer, "phone-a", &json!({"type": "input", "request_id": "retry", "device_id": "mini", "pane_id": "remote:mini:pane:w1:p1", "text": "yes"}), &listed).await
            });
            let permission = tokio::time::timeout(Duration::from_secs(5), pending.recv())
                .await
                .unwrap()
                .unwrap();
            if attempt == 2 {
                gate.close("remote:mini:pane:w1:p1");
                assert_eq!(
                    gate.wait_drained("remote:mini:pane:w1:p1", Duration::ZERO),
                    hide_core::input_gate::Drain::Drained
                );
                permission(Ok(()));
            } else {
                permission(Err(InputPermitError::Discarded));
            }
            let answer = task.await.unwrap();
            assert_eq!(
                reason(&answer),
                Some(if attempt == 2 {
                    "asleep"
                } else {
                    "input_refused"
                })
            );
        }
    }

    /// A reply meant for the agent must never reach the shell under it. The
    /// refusal is the core's gate at the write, not the phone's list, which
    /// is a moment behind: every projection here still says the agent is
    /// awake. The engine here holds no pane, so an input that gets as far as
    /// writing answers `gone`; `asleep` is the refusal before that.
    #[tokio::test]
    async fn input_to_a_pane_whose_agent_is_ending_asleep_or_waking_is_refused_at_the_write() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let listed_awake = sleeping_projection(None);

        // The agent is awake: the write is attempted.
        let answer = input(
            &mobile,
            "phone-a",
            &input_message("awake", json!({"text": "yes"})),
            &listed_awake,
        )
        .await;
        assert_eq!(reason(&answer), Some("gone"));

        // Its pane closes (the end begins) after the phone listed it awake.
        gate.close("w1:p1");
        for (id, body) in [
            ("reply", json!({"text": "echo R11-PHONE-SHELL"})),
            ("key", json!({"key": "enter"})),
        ] {
            let answer = input(&mobile, "phone-a", &input_message(id, body), &listed_awake).await;
            assert_eq!(answer["ok"], false, "{id}");
            assert_eq!(reason(&answer), Some("asleep"), "{id}");
            assert!(
                matches!(mobile.reserve_input("phone-a", id), Reservation::Write),
                "a refused input claims nothing, so the phone may send it once the agent is back"
            );
        }

        // The agent is back: the same input reaches the write step again.
        gate.set_closed(&std::collections::HashSet::new());
        let answer = input(
            &mobile,
            "phone-a",
            &input_message("back", json!({"text": "yes"})),
            &listed_awake,
        )
        .await;
        assert_eq!(reason(&answer), Some("gone"));
    }

    /// A reply that landed before the agent slept is answered from its claim
    /// when the phone repeats it after a lost answer: the sleep must not turn
    /// the recorded success, or the recorded doubt, into a refusal that
    /// invites a second manual send.
    #[tokio::test]
    async fn a_repeated_request_keeps_its_recorded_answer_across_a_sleep() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let listed = sleeping_projection(None);
        for (id, state) in [
            ("written", InputState::Written),
            ("uncertain", InputState::Uncertain),
        ] {
            assert!(matches!(
                mobile.reserve_input("phone-a", id),
                Reservation::Write
            ));
            mobile.settle_input("phone-a", id, Some(state));
        }
        gate.close("w1:p1");
        let written = input(
            &mobile,
            "phone-a",
            &input_message("written", json!({"text": "yes"})),
            &listed,
        )
        .await;
        assert_eq!(
            (written["ok"].as_bool(), reason(&written)),
            (Some(true), None)
        );
        let uncertain = input(
            &mobile,
            "phone-a",
            &input_message("uncertain", json!({"text": "yes"})),
            &listed,
        )
        .await;
        assert_eq!(
            (uncertain["ok"].as_bool(), reason(&uncertain)),
            (Some(false), Some("uncertain"))
        );
        // Only a new request is refused.
        let fresh = input(
            &mobile,
            "phone-a",
            &input_message("fresh", json!({"text": "yes"})),
            &listed,
        )
        .await;
        assert_eq!(reason(&fresh), Some("asleep"));
    }

    #[test]
    fn a_phone_wakes_only_a_sleeping_or_failed_pane_and_may_ask_for_a_new_session() {
        let message = |extra: Value| {
            let mut message = json!({
                "type": "wake_agent", "request_id": "request-1", "device_id": "test-node", "pane_id": "w1:p1",
            });
            message
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            message
        };
        let event = |action: Option<Wake>| match action {
            Some(Wake::Dispatch(bytes)) => Some(serde_json::from_slice::<Value>(&bytes).unwrap()),
            _ => None,
        };
        let sleeping = sleeping_projection(Some("sleeping"));
        let resume = event(wake_action(&message(json!({})), &sleeping, &pane())).expect("a wake");
        assert_eq!(resume["kind"], "agent_wake");
        assert_eq!(
            resume["payload"],
            json!({"pane_id": "w1:p1", "fresh": false})
        );

        let failed = sleeping_projection(Some("failed"));
        let fresh = event(wake_action(
            &message(json!({"fresh": true})),
            &failed,
            &pane(),
        ))
        .expect("a new session");
        assert_eq!(fresh["payload"], json!({"pane_id": "w1:p1", "fresh": true}));
        assert_eq!(
            wake_action(&message(json!({"fresh": "yes"})), &failed, &pane()),
            None
        );

        // A wake already running starts nothing; an awake pane has nothing to wake.
        let waking = sleeping_projection(Some("waking"));
        assert_eq!(
            wake_action(&message(json!({})), &waking, &pane()),
            Some(Wake::Running)
        );
        assert_eq!(
            wake_action(&message(json!({})), &sleeping_projection(None), &pane()),
            None
        );

        // The phone names a pane the list holds and nothing else.
        let other = AgentKey {
            device_id: "test-node".to_owned(),
            pane_id: "w1:p9".to_owned(),
        };
        assert_eq!(wake_action(&message(json!({})), &sleeping, &other), None);
        assert_eq!(
            wake_action(&message(json!({"cwd": "/"})), &sleeping, &pane()),
            None
        );
    }

    #[tokio::test]
    async fn slow_work_runs_beside_the_loop_one_piece_at_a_time_and_answers_as_a_frame() {
        let (frames, mut received) = mpsc::channel::<Value>(8);
        let mut slot = None;
        let busy = json!({"busy": true});
        // Work that finishes only when the test lets it, so nothing here depends on a clock.
        let gated = |answer: &'static str| {
            let (release, wait) = tokio::sync::oneshot::channel::<()>();
            (release, async move {
                let _ = wait.await;
                json!({"answer": answer})
            })
        };
        let (release_first, first) = gated("first");
        // The caller gets control back at once, with nothing to send yet.
        assert_eq!(
            spawn_beside_loop(&mut slot, &frames, busy.clone(), first),
            None
        );
        assert!(received.try_recv().is_err(), "the work is still running");
        // A second piece while the first waits is refused, not queued.
        let (_unused, second) = gated("second");
        assert_eq!(
            spawn_beside_loop(&mut slot, &frames, busy.clone(), second),
            Some(busy.clone())
        );
        release_first.send(()).unwrap();
        assert_eq!(received.recv().await, Some(json!({"answer": "first"})));
        // Once it answered, the slot takes the next one.
        slot.take().unwrap().await.unwrap();
        let (release_third, third) = gated("third");
        assert_eq!(spawn_beside_loop(&mut slot, &frames, busy, third), None);
        release_third.send(()).unwrap();
        assert_eq!(received.recv().await, Some(json!({"answer": "third"})));
    }

    #[test]
    fn a_ping_is_answered_with_its_number_only_while_the_phone_is_let_in() {
        let ping = json!({"type": "ping", "id": 7});
        assert_eq!(
            liveness(&ping, Ok(()), "phone"),
            Some(json!({"type": "pong", "id": 7}))
        );
        for reason in ["revoked", "mobile_off"] {
            assert_eq!(
                liveness(&ping, Err(reason), "phone"),
                Some(json!({"type": "refused", "reason": reason}))
            );
        }
        for bad in [
            json!({"type": "ping"}),
            json!({"type": "ping", "id": "7"}),
            json!({"type": "ping", "id": -1}),
        ] {
            assert_eq!(liveness(&bad, Ok(()), "phone"), None, "{bad}");
        }
    }

    #[test]
    fn the_agents_frame_carries_the_interface_language_and_color_theme() {
        let mut projection = Projection::default();
        assert_eq!(
            agents_frame(&projection),
            (
                json!({"type": "agents", "groups": [], "omitted": 0, "devices": [], "interface_language": null, "color_theme": null}),
                0
            )
        );
        projection.interface_language = Some("ko".to_owned());
        let (frame, _) = agents_frame(&projection);
        assert_eq!(frame["interface_language"], "ko");
        assert_eq!(frame["type"], "agents");
        projection.color_theme = Some("neutral".to_owned());
        assert_eq!(agents_frame(&projection).0["color_theme"], "neutral");
    }

    /// The list frame is the tree the core decided: a group's rows are tagged
    /// by kind, a parent carries its children, and the devices ride beside.
    #[test]
    fn the_agents_frame_carries_the_tree_and_the_devices() {
        let agent = |pane: &str, extra: Value| {
            let mut row = json!({
                "pane_id": pane, "group": "working", "mark": "going", "identity_label": pane,
                "agent_kind": "claude", "status_code": "idle", "demand": "none",
                "last_activity": "1790000000000", "state": {"tree_rank": 2},
            });
            for (key, value) in extra.as_object().unwrap() {
                row[key] = value.clone();
            }
            row
        };
        let projection = super::super::projection::project(
            &json!({"navigator": {"agents": [
                agent("p", json!({"lineage_child_pane_ids": ["c"]})),
                agent("c", json!({"delegated": true, "lineage_parent_pane_id": "p"})),
            ]}}),
            "test-node",
        );
        let (frame, _) = agents_frame(&projection);
        let row = &frame["groups"][0]["rows"][0];
        assert_eq!(
            (row["kind"].as_str(), row["pane_id"].as_str()),
            (Some("agent"), Some("p"))
        );
        assert_eq!(row["children"][0]["pane_id"], "c");
        assert_eq!(frame["groups"][0]["count"], 1);
        assert_eq!(
            frame["devices"],
            json!([{"device_id": "test-node", "label": null, "local": true, "connected": true, "needs_you": 0, "working": 1}])
        );
    }

    /// One mission waiting on a decision, as the projection lists it.
    fn deciding() -> PhoneMissions {
        let rest = json!({
            "navigator": {"agents": []},
            "missions": {"items": [{
                "id": "mission-1", "title": "Ship it", "device_id": "test-node", "status": "running",
                "decision": "Merge now?",
                "question": {"key": 4, "options": ["merge", "wait"], "recommend": 1, "asked_at_unix_ms": 1},
                "conductor": {"pane_id": "w1:p1", "name": "Conductor", "agent_kind": "claude", "present": true, "takes_answers": true},
                "nodes": [], "created_at_unix_ms": 1,
            }]},
        });
        PhoneMissions::project(
            &rest,
            "test-node",
            &super::super::projection::project(&rest, "test-node"),
        )
    }

    fn mission_message(request_id: &str, body: Value) -> Value {
        let mut message = json!({
            "type": "answer_mission", "request_id": request_id, "mission": "mission-1", "key": 4,
        });
        message
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        message
    }

    #[test]
    fn a_phone_answer_is_the_one_event_the_desktop_banner_sends() {
        let missions = deciding();
        let event = |body: Value| {
            mission_answer_event(&mission_message("r", body), &missions)
                .map(|bytes| serde_json::from_slice::<Value>(&bytes).unwrap())
        };
        let option = event(json!({"option": 2})).unwrap();
        assert_eq!(option["kind"], "mission_answer");
        assert_eq!(
            option["payload"],
            json!({"mission": "mission-1", "key": 4, "option": 2, "text": null, "resend": false})
        );
        let text = event(json!({"text": "  merge after lunch "})).unwrap();
        assert_eq!(text["payload"]["text"], "  merge after lunch ");
        assert_eq!(text["payload"]["option"], Value::Null);
        let resend = event(json!({"resend": true})).unwrap();
        assert_eq!(
            resend["payload"],
            json!({"mission": "mission-1", "key": 4, "option": null, "text": null, "resend": true})
        );
    }

    #[test]
    fn a_phone_answer_naming_no_listed_mission_or_no_single_answer_is_refused() {
        let missions = deciding();
        let refused = |message: Value| mission_answer_event(&message, &missions).unwrap_err();
        let mut other = mission_message("r", json!({"option": 1}));
        other["mission"] = json!("mission-9");
        assert_eq!(refused(other), "gone");
        let mut no_key = mission_message("r", json!({"option": 1}));
        no_key.as_object_mut().unwrap().remove("key");
        assert_eq!(refused(no_key), "invalid_request");
        for body in [
            json!({}),
            json!({"option": 0}),
            json!({"option": 300}),
            json!({"option": "1"}),
            json!({"option": 1, "text": "both"}),
            json!({"option": 1, "resend": true}),
            json!({"text": "   "}),
            json!({"text": "two\nlines"}),
            json!({"text": "x".repeat(ANSWER_LIMIT + 1)}),
            json!({"text": 7}),
            json!({"resend": "yes"}),
            json!({"resend": true, "text": "again"}),
        ] {
            assert_eq!(
                refused(mission_message("r", body.clone())),
                "invalid_request",
                "{body}"
            );
        }
        // The limit counts characters, so a full line of Korean is taken,
        // and as the core counts them, after the spaces around the line go
        // (`missions::answer_text`): the core takes this one too.
        let full = "가".repeat(ANSWER_LIMIT);
        assert!(
            mission_answer_event(&mission_message("r", json!({"text": full})), &missions).is_ok()
        );
        let padded = format!("  {full} \n");
        assert!(
            mission_answer_event(&mission_message("r", json!({"text": padded})), &missions).is_ok()
        );
    }

    /// A phone the Mac let in, so a request reaches its claim.
    fn paired(mobile: &Arc<Mobile>) -> String {
        let code = {
            let mut inner = mobile.lock();
            inner.settings.enabled = true;
            inner.exposure = super::super::Exposure::Relay;
            inner.phones.new_code(super::super::now_ms());
            inner.phones.code().unwrap().0.to_owned()
        };
        mobile.pair(&code, "Phone").unwrap().0.id
    }

    #[tokio::test]
    async fn the_same_answer_request_is_answered_from_its_claim_and_reaches_the_core_once() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        mobile.missions.send_replace(Arc::new(deciding()));
        let phone = paired(&mobile);

        let first = answer_mission(
            &mobile,
            &phone,
            &mission_message("press-1", json!({"option": 1})),
        )
        .await;
        assert_eq!(first["type"], "mission_result");
        assert_eq!(first["request_id"], "press-1");
        assert_eq!(first["mission"], "mission-1");
        assert_eq!(first["ok"], true, "{first}");
        assert!(
            matches!(
                mobile.reserve_input(&phone, "press-1"),
                Reservation::Seen(InputState::Written)
            ),
            "the press is recorded as written, so a repeat is not typed again"
        );
        let again = answer_mission(
            &mobile,
            &phone,
            &mission_message("press-1", json!({"option": 1})),
        )
        .await;
        assert_eq!(again["ok"], true, "{again}");

        // A press still being sent is told so, never sent a second time.
        assert!(matches!(
            mobile.reserve_input(&phone, "press-2"),
            Reservation::Write
        ));
        let racing = answer_mission(
            &mobile,
            &phone,
            &mission_message("press-2", json!({"option": 1})),
        )
        .await;
        assert_eq!(
            (racing["ok"].as_bool(), reason(&racing)),
            (Some(false), Some("in_flight"))
        );
    }

    #[tokio::test]
    async fn a_refused_answer_claims_nothing_and_a_request_without_an_id_is_invalid() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        mobile.missions.send_replace(Arc::new(deciding()));
        let phone = paired(&mobile);

        let refused = answer_mission(
            &mobile,
            &phone,
            &mission_message("bad", json!({"text": "   "})),
        )
        .await;
        assert_eq!(reason(&refused), Some("invalid_request"));
        assert!(
            matches!(mobile.reserve_input(&phone, "bad"), Reservation::Write),
            "a refused answer may be sent again once it is fixed"
        );
        let mut anonymous = mission_message("x", json!({"option": 1}));
        anonymous.as_object_mut().unwrap().remove("request_id");
        let answer = answer_mission(&mobile, &phone, &anonymous).await;
        assert_eq!(
            (answer["ok"].as_bool(), reason(&answer)),
            (Some(false), Some("invalid_request"))
        );
    }

    /// A phone with the one pull request list the PR requests are checked against.
    fn listing(mobile: &Arc<Mobile>, card_head: &str) {
        use super::super::prs::fixture::{card, prs};
        mobile
            .prs
            .send_replace(Arc::new(prs(card(card_head, false, &["squash"]))));
    }

    fn pr_message(kind: &str, request_id: &str, body: Value) -> Value {
        let mut message = json!({"type": kind, "request_id": request_id, "pr": "w1#1"});
        message
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        message
    }

    #[tokio::test]
    async fn the_same_merge_request_is_answered_from_its_claim_and_reaches_the_core_once() {
        use super::super::prs::fixture::HEAD;
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        listing(&mobile, HEAD);
        let phone = paired(&mobile);
        let ask = pr_message(
            "merge_pr",
            "merge-1",
            json!({"head": HEAD, "method": "squash"}),
        );

        let first = pr_request(&mobile, &phone, "merge_pr", &ask).await;
        assert_eq!(first["type"], "pr_result");
        assert_eq!(
            (first["request_id"].as_str(), first["pr"].as_str()),
            (Some("merge-1"), Some("w1#1"))
        );
        assert_eq!(first["ok"], true, "{first}");
        assert!(
            matches!(
                mobile.reserve_input(&phone, "merge-1"),
                Reservation::Seen(InputState::Written)
            ),
            "the press is recorded as written, so a repeat is not sent to GitHub again"
        );
        let again = pr_request(&mobile, &phone, "merge_pr", &ask).await;
        assert_eq!(again["ok"], true, "{again}");

        // A press still being sent is told so, never sent a second time.
        assert!(matches!(
            mobile.reserve_input(&phone, "merge-2"),
            Reservation::Write
        ));
        let racing = pr_request(
            &mobile,
            &phone,
            "merge_pr",
            &pr_message(
                "merge_pr",
                "merge-2",
                json!({"head": HEAD, "method": "squash"}),
            ),
        )
        .await;
        assert_eq!(
            (racing["ok"].as_bool(), reason(&racing)),
            (Some(false), Some("in_flight"))
        );
    }

    #[tokio::test]
    async fn a_merge_for_a_moved_head_or_an_unlisted_pull_request_claims_nothing() {
        use super::super::prs::fixture::HEAD;
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        listing(&mobile, HEAD);
        let phone = paired(&mobile);

        let moved = pr_request(
            &mobile,
            &phone,
            "merge_pr",
            &pr_message(
                "merge_pr",
                "m-moved",
                json!({"head": "f".repeat(40), "method": "squash"}),
            ),
        )
        .await;
        assert_eq!(
            (moved["ok"].as_bool(), reason(&moved)),
            (Some(false), Some("head_moved"))
        );
        assert!(
            matches!(mobile.reserve_input(&phone, "m-moved"), Reservation::Write),
            "a refused merge may be sent again once the card is read"
        );
        let mut gone = pr_message(
            "merge_pr",
            "m-gone",
            json!({"head": HEAD, "method": "squash"}),
        );
        gone["pr"] = json!("w9#4");
        let gone = pr_request(&mobile, &phone, "merge_pr", &gone).await;
        assert_eq!(reason(&gone), Some("gone"));
        let mut anonymous = pr_message("merge_pr", "x", json!({"head": HEAD}));
        anonymous.as_object_mut().unwrap().remove("request_id");
        let invalid = pr_request(&mobile, &phone, "merge_pr", &anonymous).await;
        assert_eq!(reason(&invalid), Some("invalid_request"));
    }

    #[tokio::test]
    async fn reading_a_card_or_its_feedback_is_not_claimed_and_a_hand_over_is_answered_by_its_start()
     {
        use super::super::prs::fixture::HEAD;
        use super::super::start::Answer;
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        listing(&mobile, HEAD);
        let phone = paired(&mobile);

        for (kind, id) in [("read_pr", "read-1"), ("read_pr_feedback", "feedback-1")] {
            let sent = pr_request(&mobile, &phone, kind, &pr_message(kind, id, json!({}))).await;
            assert_eq!(sent["ok"], true, "{sent}");
            assert!(
                matches!(mobile.reserve_input(&phone, id), Reservation::Write),
                "a read sent twice reads twice, so it claims nothing"
            );
        }

        // The core's start answer for the hand-over's request id comes back as the start's own result.
        mobile.answers.send_replace(Arc::new(vec![(
            "hand-1".to_owned(),
            Answer::Started {
                device_id: "test-node".into(),
                pane_id: "w1:p9".into(),
            },
        )]));
        let started = mobile
            .delegate_pr(
                &phone,
                &pr_message(
                    "delegate_pr",
                    "hand-1",
                    json!({"kind": "claude", "prompt": "Fix PR #1"}),
                ),
            )
            .await;
        assert_eq!(started["type"], "start_result");
        assert_eq!(
            (started["ok"].as_bool(), started["pane_id"].as_str()),
            (Some(true), Some("w1:p9"))
        );
        let refused = mobile
            .delegate_pr(
                &phone,
                &pr_message(
                    "delegate_pr",
                    "hand-2",
                    json!({"kind": "terminal", "prompt": "go"}),
                ),
            )
            .await;
        assert_eq!(
            (refused["ok"].as_bool(), reason(&refused)),
            (Some(false), Some("unknown_kind"))
        );
    }

    /// Opening an agent's detail tells the core the phone looks at it, so it
    /// is read on both surfaces as the desktop's focus reads it; it stays
    /// looked at while any phone has it open, and a phone that closes the
    /// detail or goes away stops looking (`docs/status-model.md`).
    #[tokio::test]
    async fn an_agent_a_phone_opens_is_looked_at_until_every_phone_lets_it_go() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let (frames, _received) = mpsc::channel::<Value>(64);
        let projection = Arc::new(sleeping_projection(None));
        let (mut first, mut second) = (Open::default(), Open::default());
        let send = async |connection: u64, open: &mut Open, message: Value| {
            handle(
                &mobile,
                connection,
                "phone",
                &message,
                open,
                &frames,
                &projection,
            )
            .await
        };
        let _ = mobile.register(1, "phone-1");
        let _ = mobile.register(2, "phone-2");
        let open = json!({"type": "open", "device_id": "test-node", "pane_id": "w1:p1", "view": "conversation"});
        let looked_at = std::collections::BTreeSet::from(["w1:p1".to_owned()]);

        let gone = json!({"type": "open", "device_id": "test-node", "pane_id": "w1:p9", "view": "conversation"});
        send(1, &mut first, gone).await;
        assert!(mobile.viewing_announced().is_empty(), "no agent, no look");

        send(1, &mut first, open.clone()).await;
        assert_eq!(mobile.viewing_announced(), looked_at);
        send(2, &mut second, open).await;
        send(1, &mut first, json!({"type": "close"})).await;
        assert_eq!(
            mobile.viewing_announced(),
            looked_at,
            "the other phone still has it open"
        );
        mobile.unregister(2);
        assert!(mobile.viewing_announced().is_empty());
        mobile.shutdown().await;
    }

    /// A newer bundle under the same phone protocol may send a request this
    /// Hide predates (`docs/ARCHITECTURE.md`, the phone app): it is answered
    /// with a refusal that names it, never dropped or taken for another.
    #[tokio::test]
    async fn a_request_this_hide_predates_is_refused_by_name() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = mobile_with(directory.path(), &gate);
        let (frames, _received) = mpsc::channel::<Value>(8);
        let mut open = Open::default();
        let projection = Arc::new(sleeping_projection(None));
        let later = json!({"type": "later_screen", "request_id": "r-1"});
        let refused = handle(
            &mobile,
            1,
            "phone-1",
            &later,
            &mut open,
            &frames,
            &projection,
        )
        .await;
        assert_eq!(
            refused,
            Some(
                json!({"type": "refused_request", "request": "later_screen", "reason": "not_allowed"})
            )
        );
        mobile.shutdown().await;
    }
}
