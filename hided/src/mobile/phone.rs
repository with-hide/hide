//! One phone's connection on `/ws` (PRD D-04, D-13, D-16, D-17).
//!
//! The first frame names it: `{client_kind: "phone", token}` for a paired
//! phone, `{client_kind: "phone", pair, name}` to pair with the live code.
//! After that the phone may only: open one agent's detail, choose its
//! conversation or its terminal, ask for older messages or more rows, close
//! it, send that pane a reply or one key, open the start sheet and start an
//! agent from it, wake a sleeping agent in its pane, wake or inspect a listed
//! sleeping conversation, and register
//! or report its push subscription. Anything else is refused and recorded.
//! It never receives a file, a path, a setting or the core's snapshot; the one
//! exception is the interface language, which rides each `agents` frame so the
//! phone speaks the language the operator chose.
//!
//! Every frame to the phone goes through `encode`, the one place a relay
//! transport would wrap in end-to-end encryption (PRD D-01).

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use herdr_core::agent_state::phone::{DormantPhase, SleepId};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::conversation::{self, Tail, Transcript};
use super::pane::{self, Input, PaneError};
use super::projection::{AgentKey, Projection};
use super::store::{Notifications, PushSubscription};
use super::{InputState, Mobile, PhoneMeta, Reservation};

/// How often an open detail reads its pane again.
const DETAIL_INTERVAL: Duration = Duration::from_secs(1);
/// The largest frame a phone may send.
const MAX_FRAME: usize = 16 * 1024;
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

/// Sends one message; false when the socket failed or stalled past
/// `SEND_LIMIT`, and the connection should end.
async fn send(socket: &mut WebSocket, message: Message, phone_id: &str) -> bool {
    match tokio::time::timeout(SEND_LIMIT, socket.send(message)).await {
        Ok(result) => result.is_ok(),
        Err(_) => {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phone.stalled", "phone_id": phone_id,
            }));
            false
        }
    }
}

/// The one function every frame to a phone passes through.
pub fn encode(frame: &Value) -> Message {
    Message::Text(frame.to_string().into())
}

fn refusal_logged(kind: &str, phone_id: Option<&str>, reason: &str) {
    herdr_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": kind, "phone_id": phone_id, "reason": reason,
    }));
}

async fn refuse(socket: &mut WebSocket, reason: &str) {
    let refusal = encode(&json!({"type": "refused", "reason": reason}));
    let _ = tokio::time::timeout(SEND_LIMIT, async {
        let _ = socket.send(refusal).await;
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

fn agents_frame(projection: &Projection) -> Value {
    json!({
        "type": "agents",
        "groups": projection.groups,
        // The core's language as stored, or null to follow the phone's own.
        "interface_language": projection.interface_language,
        // The operator's palette; the phone keeps its own light or dark.
        "color_theme": projection.color_theme,
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

/// What a phone has open: at most one agent's detail, the start sheet, and
/// at most one start waiting for the core's answer.
#[derive(Default)]
struct Open {
    detail: Option<Detail>,
    /// The sheet gets the catalog as it changes.
    sheet: bool,
    /// Not aborted when the connection ends: the start already reached the
    /// core, and settling its request id lets a reconnecting phone's repeat
    /// read the answer. `start::ANSWER_LIMIT` bounds it.
    start: Option<tokio::task::JoinHandle<()>>,
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
        loop {
            let ask = *asks.borrow_and_update();
            let mut outgoing = Vec::new();
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

fn conversation_failed(message: &str) {
    herdr_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": "conversation.read_failed", "message": message,
    }));
}

/// One pass over the pane's conversation: open its transcript and send the
/// newest page, or send what the agent appended. The transcript comes back
/// `None` when the pane has no conversation to show. A failed Herdr read
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
    let pane_id = key.herdr_pane_id().to_owned();
    let (transcript, page) = tokio::task::spawn_blocking(move || {
        let source = reader
            .herdr_api(reader.node().as_str())
            .and_then(|connector| conversation::source(&connector, &pane_id));
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
/// line count changed. Herdr's read `revision` counts pane state, not output,
/// so it cannot say that new rows arrived.
#[derive(Default)]
struct RowsReader {
    last: Option<(String, u32)>,
    last_error: Option<&'static str>,
}

impl RowsReader {
    async fn read(&mut self, mobile: &Arc<Mobile>, key: &AgentKey, asked: u32) -> Option<Value> {
        let pane_id = key.herdr_pane_id().to_owned();
        let device_id = key.device_id.clone();
        let reader = Arc::clone(mobile);
        // Resolving a device's connection waits on the core owner thread,
        // so it runs on the blocking pool with the read itself.
        let read = tokio::task::spawn_blocking(move || {
            reader
                .herdr_api(&device_id)
                .and_then(|connector| pane::read(&connector, &pane_id, asked))
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
                    // Herdr says whether it cut older rows off this read.
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
                    herdr_core::diagnostic!(json!({
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
    mut socket: WebSocket,
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
    if let Err(reason) = mobile.still_admitted(&phone.id) {
        refuse(&mut socket, reason).await;
        mobile.unregister(connection);
        return;
    }
    if let Some(paired) = paired
        && !send(&mut socket, encode(&paired), &phone.id).await
    {
        mobile.unregister(connection);
        return;
    }
    run(&mut socket, &mobile, connection, &phone.id, close).await;
    mobile.unregister(connection);
}

async fn run(
    socket: &mut WebSocket,
    mobile: &Arc<Mobile>,
    connection: u64,
    phone_id: &str,
    close: Arc<tokio::sync::Notify>,
) {
    let mut projection = mobile.subscribe_projection();
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
    let first = [
        hello,
        meta_frame(&meta.borrow_and_update()),
        agents_frame(&projection.borrow_and_update()),
    ];
    for frame in &first {
        if !send(socket, encode(frame), phone_id).await {
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
                let frame = agents_frame(&projection.borrow_and_update());
                if !send(socket, encode(&frame), phone_id).await { return; }
            }
            changed = meta.changed() => {
                if changed.is_err() { return; }
                let frame = meta_frame(&meta.borrow_and_update());
                if !send(socket, encode(&frame), phone_id).await { return; }
            }
            changed = catalog.changed() => {
                if changed.is_err() { return; }
                let frame = catalog.borrow_and_update().frame();
                if open.sheet && !send(socket, encode(&frame), phone_id).await { return; }
            }
            Some(frame) = frames.recv() => {
                if !send(socket, encode(&frame), phone_id).await { return; }
            }
            _ = ping.tick() => {
                if heard.elapsed() >= SILENT_LIMIT {
                    herdr_core::diagnostic!(json!({
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
                    && !send(socket, encode(&reply), phone_id).await
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
    projection: &Projection,
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
            mobile.set_viewing(connection, Some(key.clone()));
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
            mobile.set_viewing(connection, None);
            None
        }
        "input" => Some(input(mobile, phone_id, message, projection).await),
        "wake_agent" => Some(wake_agent(mobile, phone_id, message, projection).await),
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
            if open.start.as_ref().is_some_and(|task| !task.is_finished()) {
                return Some(
                    json!({"type": "start_result", "request_id": message.get("request_id"), "ok": false, "reason": "in_flight"}),
                );
            }
            // The answer can take the core's whole creation time, so it is
            // awaited beside the socket loop and arrives as a frame.
            let (mobile, phone_id, message, frames) = (
                Arc::clone(mobile),
                phone_id.to_owned(),
                message.clone(),
                frames.clone(),
            );
            open.start = Some(tokio::spawn(async move {
                let answer = mobile.start_agent(&phone_id, &message).await;
                let _ = frames.send(answer).await;
            }));
            None
        }
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
    let bytes = json!({"schema_version": herdr_core::SCHEMA_VERSION, "kind": event, "payload": {"sleep_id": id}})
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
        "schema_version": herdr_core::SCHEMA_VERSION, "kind": "agent_wake",
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
    // The shell under a sleeping agent is not who the phone talks to: nothing
    // reaches the pane until the agent is back, whatever the phone drew.
    if projection
        .find(&key)
        .is_some_and(|agent| agent.sleep.is_some())
    {
        refusal_logged("scope.refused", Some(phone_id), "input_to_sleeping_agent");
        return answer(false, Some("asleep"));
    }
    let text = message.get("text").and_then(Value::as_str);
    let herdr_key = message.get("key").and_then(Value::as_str);
    let problem = match (text, herdr_key) {
        (Some(text), None) => pane::reply_problem(text),
        (None, Some(key)) => pane::herdr_key(key).is_none().then_some("unknown_key"),
        _ => Some("invalid_request"),
    };
    if let Some(problem) = problem {
        return answer(false, Some(problem));
    }
    // One claim per request id, taken before writing: a repeat that arrives
    // while the first is still being written, or after Herdr's answer was
    // lost, is answered from the claim and never written again (B27).
    match mobile.reserve_input(phone_id, request_id) {
        Reservation::Write => {}
        Reservation::Seen(InputState::Written) => return answer(true, None),
        Reservation::Seen(InputState::InFlight) => return answer(false, Some("in_flight")),
        Reservation::Seen(InputState::Uncertain) => return answer(false, Some("uncertain")),
    }
    let pane_id = key.herdr_pane_id().to_owned();
    let core_pane_id = key.pane_id.clone();
    let device_id = key.device_id.clone();
    let writer = Arc::clone(mobile);
    let text = text.map(str::to_owned);
    let herdr_key = herdr_key.and_then(pane::herdr_key);
    // `attempted` is whether the write reached Herdr at all.
    let (attempted, result) = tokio::task::spawn_blocking(move || {
        let connector = match writer.herdr_api(&device_id) {
            Ok(connector) => connector,
            Err(error) => return (false, Err(error)),
        };
        let input = match (&text, herdr_key) {
            (Some(text), _) => Input::Reply(text),
            (None, Some(key)) => Input::Key(key),
            (None, None) => unreachable!("validated above"),
        };
        let reply = matches!(input, Input::Reply(_));
        // Before the write, so the doorbell cannot judge the pane clean
        // while the key is landing; a refused write only holds the pane.
        writer.note_input(&core_pane_id);
        let sent = pane::send(&connector, &pane_id, input);
        if reply && sent.is_ok() {
            writer.note_submit(&core_pane_id);
        }
        (true, sent)
    })
    .await
    .unwrap_or_else(|error| (true, Err(PaneError::Unavailable(error.to_string()))));
    // Nothing reached Herdr, or Herdr says the pane is gone: nothing was
    // written and the id may be sent again. Any other failure may have landed.
    let certain =
        !attempted || matches!(result, Err(PaneError::Gone | PaneError::DeviceUnreachable));
    let settled = match &result {
        Ok(()) => Some(InputState::Written),
        Err(_) if certain => None,
        Err(_) => Some(InputState::Uncertain),
    };
    mobile.settle_input(phone_id, request_id, settled);
    match result {
        Ok(()) => {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "input.sent", "phone_id": phone_id,
                "device_id": key.device_id, "kind_of_input": if message.get("key").is_some() { "key" } else { "reply" },
            }));
            answer(true, None)
        }
        Err(error) => {
            herdr_core::diagnostic!(json!({
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

    /// A reply meant for the agent must never reach the shell under a sleeping
    /// one. This Mac has no Herdr socket here, so an input that gets as far as
    /// writing answers `unavailable`; `asleep` is the refusal before that.
    #[tokio::test]
    async fn input_to_a_pane_whose_agent_is_not_running_is_refused_before_anything_is_written() {
        let directory = tempfile::tempdir().unwrap();
        let mobile = Mobile::start(super::super::Config {
            state_dir: directory.path().join("state"),
            home: directory.path().to_path_buf(),
            port: 0,
            cli: super::super::tailscale::CliSource {
                pinned: None,
                search_path: None,
            },
            host_name: None,
            core: Arc::new(crate::core::CoreHandle::bare(directory.path())),
            herdr_socket: None,
            renderers: Arc::default(),
            start_demand: Arc::default(),
            open: tokio::sync::watch::channel(true).1,
            desktop: Arc::default(),
        });
        let message = |request_id: &str, body: Value| {
            let mut message = json!({
                "type": "input", "request_id": request_id, "device_id": "test-node", "pane_id": "w1:p1",
            });
            message
                .as_object_mut()
                .unwrap()
                .extend(body.as_object().unwrap().clone());
            message
        };
        let reason = |answer: Value| answer["reason"].as_str().map(str::to_owned);

        for state in ["sleeping", "waking", "failed"] {
            let projection = sleeping_projection(Some(state));
            for (id, body) in [
                ("reply", json!({"text": "echo R11-PHONE-SHELL"})),
                ("key", json!({"key": "enter"})),
            ] {
                let request_id = format!("{state}-{id}");
                let answer =
                    input(&mobile, "phone-a", &message(&request_id, body), &projection).await;
                assert_eq!(answer["ok"], false, "{state} {id}");
                assert_eq!(reason(answer), Some("asleep".to_owned()), "{state} {id}");
                assert!(
                    matches!(
                        mobile.reserve_input("phone-a", &request_id),
                        Reservation::Write
                    ),
                    "a refused input claims nothing, so the phone may send it once the agent is back"
                );
            }
        }

        // The agent is back (no sleep on its row): the same input now reaches
        // the write step, which has no Herdr to write to.
        let awake = sleeping_projection(None);
        let answer = input(
            &mobile,
            "phone-a",
            &message("awake", json!({"text": "yes"})),
            &awake,
        )
        .await;
        assert_eq!(reason(answer), Some("unavailable".to_owned()));
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

    #[test]
    fn the_agents_frame_carries_the_interface_language_and_color_theme() {
        let mut projection = Projection::default();
        assert_eq!(
            agents_frame(&projection),
            json!({"type": "agents", "groups": [], "interface_language": null, "color_theme": null})
        );
        projection.interface_language = Some("ko".to_owned());
        let frame = agents_frame(&projection);
        assert_eq!(frame["interface_language"], "ko");
        assert_eq!(frame["type"], "agents");
        projection.color_theme = Some("neutral".to_owned());
        assert_eq!(agents_frame(&projection)["color_theme"], "neutral");
    }
}
