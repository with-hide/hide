//! A screen of this machine, served by the node's daemon: held while the
//! link is down, then attached to its own relay to the core, whose frames
//! it reads into a capped backlog, and whose events it routes here or up.

use super::terminals::{InputNotices, take_terminal_key};
use super::*;
use crate::core_move::control::MoveScreen;

/// Core frames one screen may leave untaken before its backlog is dropped
/// and it is drawn again from a fresh snapshot (D-20).
const SCREEN_BACKLOG_BYTES: usize = 4 * 1024 * 1024;
/// A screen's frames waiting to go up its relay; past it the screen's
/// socket is read no further until the relay takes them.
const SCREEN_TO_CORE: usize = 64;

pub(super) async fn screen(
    mut socket: WebSocket,
    state: NodeState,
    origin: Option<String>,
    proxied: bool,
) {
    if proxied || check_origin(origin.as_deref(), &state.allowed_origins).is_err() {
        refuse(&mut socket, CloseReason::OriginNotAllowed, None).await;
        return;
    }
    let handshake_text = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text.to_string(),
        _ => {
            refuse(&mut socket, CloseReason::InvalidToken, None).await;
            return;
        }
    };
    let Ok(handshake) = serde_json::from_str::<Handshake>(&handshake_text) else {
        refuse(&mut socket, CloseReason::InvalidToken, None).await;
        return;
    };
    if handshake.schema_version != SCHEMA_VERSION {
        refuse(&mut socket, CloseReason::SchemaMismatch, None).await;
        return;
    }
    if !crate::server::token_matches(&handshake.token, &state.token) {
        refuse(&mut socket, CloseReason::InvalidToken, None).await;
        return;
    }
    let previous = state.clients.fetch_add(1, Ordering::SeqCst);
    if previous >= MAX_CLIENTS {
        state.clients.fetch_sub(1, Ordering::SeqCst);
        refuse(&mut socket, CloseReason::ClientLimit, Some(previous + 1)).await;
        return;
    }
    let connection = state.connections.fetch_add(1, Ordering::SeqCst);
    let desktop = handshake.client_kind.as_deref() == Some("desktop");
    if desktop {
        state.desktop_screens.fetch_add(1, Ordering::SeqCst);
    }
    let mut local_reads = 0_u64;
    // This screen's hold on a move it asks for, which its end lets go of.
    let mover = crate::core_move::control::MoveScreen::new(&state.seat.moves);
    // B8: the dial this screen's Retry started ends with the screen,
    // whether or not it asked anything yet.
    let _press = Press {
        release: Arc::clone(&state.release_sign_in),
        connection,
    };
    let ended = attached(
        &mut socket,
        &state,
        &mover,
        handshake,
        &forwarded_handshake(&handshake_text),
        connection,
        &mut local_reads,
    )
    .await;
    if desktop {
        state.desktop_screens.fetch_sub(1, Ordering::SeqCst);
    }
    herdr_core::diagnostic!(json!({
        "component": "node_daemon",
        "kind": "screen.ended",
        "connection": connection,
        "reason": ended.reason(),
        "local_file_reads": local_reads,
    }));
    if let Some(close) = ended.close() {
        let _ = socket.send(Message::Close(Some(close))).await;
    }
    let remaining = state
        .clients
        .fetch_sub(1, Ordering::SeqCst)
        .saturating_sub(1);
    if remaining == 0 {
        *state
            .last_client_gone
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
    }
}

/// Waits for a live link, holding the screen: it is told where the link
/// stands each time that changes (`core_link` frame), and where a move of
/// the core stands from the moment one is under way (`core_move` frame), a
/// window shown nothing of a move until then; what it sends meanwhile is
/// dropped and counted, but for the operator's reconnect (B16), an answer to
/// what the dial their Retry started asks (`core_link` {action: `sign_in`},
/// PRD system-ssh-device-link B33) and a move's asks, such as giving up a
/// move whose peer never answers. `None` when the screen left first.
async fn hold(
    socket: &mut WebSocket,
    state: &NodeState,
    mover: &MoveScreen,
    connection: u64,
) -> Option<Arc<LiveLink>> {
    let mut live = state.live.clone();
    let mut phases = state.phases.clone();
    let mut sign_ins = state.sign_ins.clone();
    let mut told: Option<Value> = None;
    let mut move_frames = state.seat.moves.subscribe();
    move_frames.mark_changed();
    let mut move_told = false;
    loop {
        if let Some(link) = live.borrow_and_update().clone() {
            return Some(link);
        }
        // Only the screen whose Retry started the dial is shown what it
        // asks; another sees the link connecting.
        let sign_in = sign_ins
            .borrow_and_update()
            .clone()
            .filter(|shown| shown.owner == connection)
            .map(|shown| shown.sign_in);
        let frame = phases
            .borrow_and_update()
            .link_frame(&state.core_alias, sign_in.as_ref());
        if let Some(frame) = frame
            && told.as_ref() != Some(&frame)
        {
            if socket
                .send(Message::Text(frame.to_string().into()))
                .await
                .is_err()
            {
                return None;
            }
            told = Some(frame);
        }
        tokio::select! {
            changed = live.changed() => {
                if changed.is_err() {
                    return None;
                }
            }
            changed = phases.changed() => {
                if changed.is_err() {
                    return None;
                }
            }
            changed = sign_ins.changed() => {
                if changed.is_err() {
                    return None;
                }
            }
            changed = move_frames.changed() => {
                if changed.is_err() {
                    return None;
                }
                let view = move_frames.borrow_and_update().clone();
                move_told |= view.state.busy();
                if move_told {
                    let frame = mover.frame(&view);
                    if socket.send(Message::Text(frame.into())).await.is_err() {
                        return None;
                    }
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
                Some(Ok(Message::Text(text))) => {
                    match screen_event::read(&text, &[Kind::CoreLink, Kind::CoreMove]) {
                        Some(routed) if routed.kind == Kind::CoreLink => {
                            if control_link(socket, state, &routed.event, connection)
                                .await
                                .is_err()
                            {
                                return None;
                            }
                        }
                        Some(routed) => {
                            if let Err(reason) = mover.request_event(&routed.event) {
                                let frame = crate::core_move::control::refusal_frame(reason);
                                if socket.send(Message::Text(frame.into())).await.is_err() {
                                    return None;
                                }
                            }
                        }
                        None => {
                            state.held_frames.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                Some(Ok(_)) => {
                    state.held_frames.fetch_add(1, Ordering::Relaxed);
                }
            },
        }
    }
}

/// A screen's `core_link` event: the operator ends the link from this
/// machine's window or restores it (B16), or retries a link that waits for
/// them. The record and the link change
/// off this task, since the record is written durably and a dial in
/// progress is waited for. A change that did not happen is told to the
/// screen; why goes to the log. `Err` when the screen left.
async fn control_link(
    socket: &mut WebSocket,
    state: &NodeState,
    event: &Value,
    connection: u64,
) -> Result<(), ()> {
    if event.pointer("/payload/action").and_then(Value::as_str) == Some("sign_in") {
        // The answer goes to the waiting question and nowhere else, and only
        // from the screen whose Retry started the dial: the log hears only
        // that it came (B10).
        let answer = event.get("payload").cloned().and_then(|payload| {
            serde_json::from_value::<crate::screen_sign_in::SignInAnswer>(payload).ok()
        });
        if let Some(answer) = answer {
            let _ = (state.answer_sign_in)(connection, answer.question, answer.reply);
        } else {
            herdr_core::diagnostic!(json!({
                "component": "node_daemon",
                "kind": "core_link.sign_in_malformed",
            }));
        }
        return Ok(());
    }
    let action = event
        .pointer("/payload/action")
        .cloned()
        .and_then(|action| serde_json::from_value::<LinkAction>(action).ok());
    let outcome = match action {
        Some(action) => {
            let control = Arc::clone(&state.link_control);
            tokio::task::spawn_blocking(move || control(action, connection))
                .await
                .unwrap_or_else(|error| Err(error.to_string()))
        }
        None => Err("a core_link event names disconnect, reconnect or retry".to_owned()),
    };
    herdr_core::diagnostic!(json!({
        "component": "node_daemon",
        "kind": "core_link.control",
        "action": action.map(|action| format!("{action:?}").to_lowercase()),
        "ok": outcome.is_ok(),
        "reason": outcome.as_ref().err(),
    }));
    if outcome.is_ok() {
        return Ok(());
    }
    let frame = json!({
        "type": "error",
        "payload": {"kind": "core_link", "reason": "not_changed"},
        "message": "the link to the core could not be changed",
    });
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .map_err(|_| ())
}

/// One screen while the link stands: its core traffic through its relay,
/// its terminals from this daemon's hub. Answers why it ended.
async fn attached(
    socket: &mut WebSocket,
    state: &NodeState,
    mover: &MoveScreen,
    handshake: Handshake,
    handshake_text: &str,
    connection: u64,
    local_reads: &mut u64,
) -> ScreenEnd {
    let Some(link) = hold(socket, state, mover, connection).await else {
        return ScreenEnd::Left;
    };
    let relay = tokio::select! {
        opened = ScreenRelay::open(&link, handshake_text, connection) => opened,
        // What the screen sends before its relay stands is dropped like
        // what it sends while the link is down, never delivered late (B8).
        () = drop_until_closed(socket, state) => return ScreenEnd::Left,
    };
    let relay = match relay {
        Ok(relay) => relay,
        Err(message) => {
            herdr_core::diagnostic!(json!({
                "component": "node_daemon",
                "kind": "screen.relay_failed",
                "connection": connection,
                "message": message,
            }));
            return ScreenEnd::LinkLost;
        }
    };
    let mut live = state.live.clone();
    let mut terminals: Option<HubClient> = None;
    let mut told = InputNotices::default();
    let unsent = state.terminals.unsent.watch(connection);
    let mut uploads = ScreenUploads::new(
        Arc::clone(&state.attachments),
        connection,
        state.boundary.node().as_str(),
        &state.terminals.own_prefix,
    );
    // A move of the core is this machine's hided's, beside the core's
    // traffic.
    let mut move_frames = state.seat.moves.subscribe();
    let frame = mover.frame(&move_frames.borrow_and_update());
    if socket.send(Message::Text(frame.into())).await.is_err() {
        return ScreenEnd::Left;
    }
    loop {
        let shared = Arc::clone(&relay.shared);
        tokio::select! {
            changed = move_frames.changed() => {
                if changed.is_err() {
                    return ScreenEnd::Left;
                }
                let frame = mover.frame(&move_frames.borrow_and_update());
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return ScreenEnd::Left;
                }
            }
            changed = live.changed() => {
                let same = changed.is_ok()
                    && live
                        .borrow_and_update()
                        .as_ref()
                        .is_some_and(|now| now.generation == link.generation);
                if !same {
                    return ScreenEnd::LinkLost;
                }
            }
            () = async {
                match &terminals {
                    Some(client) => client.ready().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(client) = &terminals else { continue };
                let (frame, redraws) = client.take();
                for pane in redraws {
                    state.terminals.redraw(connection, &pane);
                }
                if let Some(frame) = frame
                    && socket.send(Message::Text(frame.text.into())).await.is_err()
                {
                    return ScreenEnd::Left;
                }
            }
            () = unsent.ready() => {
                if let Some(message) = unsent.take() {
                    let frame = json!({"type": "error", "payload": {}, "message": message});
                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                        return ScreenEnd::Left;
                    }
                }
            }
            () = shared.ready.notified() => {
                loop {
                    let next = lock(&shared.backlog).pop();
                    let Some(message) = next else { break };
                    match message {
                        tungstenite::Message::Text(text) => {
                            let kind = frame_kind(&text);
                            match (kind, &terminals) {
                                (Some(kind), None) => {
                                    terminals = Some(state.hub.connect(resume(kind, &handshake)));
                                }
                                (Some(FrameStart::Snapshot), Some(client)) => client.restart(),
                                _ => {}
                            }
                            if socket.send(Message::Text(text.as_str().into())).await.is_err() {
                                return ScreenEnd::Left;
                            }
                        }
                        tungstenite::Message::Binary(bytes) => {
                            if socket.send(Message::Binary(bytes)).await.is_err() {
                                return ScreenEnd::Left;
                            }
                        }
                        _ => {}
                    }
                }
                let end = lock(&shared.backlog).end.take();
                match end {
                    None => {}
                    Some(RelayEnd::Closed(None)) => return ScreenEnd::LinkLost,
                    Some(RelayEnd::Closed(Some(refusal))) => return ScreenEnd::Refused(refusal),
                    // The screen fell behind and what it missed is gone,
                    // answers to its reads among them: it is closed, so it
                    // fails what it waits for and reattaches from what it
                    // last applied.
                    Some(RelayEnd::Overflowed) => return ScreenEnd::FellBehind,
                }
            }
            from_screen = socket.recv() => {
                let message = match from_screen {
                    Some(Ok(message)) => message,
                    _ => return ScreenEnd::Left,
                };
                match message {
                    Message::Text(text) => {
                        let routed = screen_event::read(&text, TAKEN_HERE);
                        let routed = routed.as_ref().map(|routed| (routed.kind, &routed.event));
                        if let Some((Kind::CoreLink, event)) = routed {
                            if control_link(socket, state, event, connection).await.is_err() {
                                return ScreenEnd::Left;
                            }
                            continue;
                        }
                        if let Some((Kind::CoreMove, event)) = routed {
                            if let Err(reason) = mover.request_event(event) {
                                let frame = crate::core_move::control::refusal_frame(reason);
                                if socket.send(Message::Text(frame.into())).await.is_err() {
                                    return ScreenEnd::Left;
                                }
                            }
                            continue;
                        }
                        if let Some((Kind::FileBytes, event)) = routed
                            && let Some(opened) = own_file(state, event)
                        {
                            *local_reads += 1;
                            if crate::server::send_opened_file_bytes(socket, event, opened).await.is_err() {
                                return ScreenEnd::Left;
                            }
                            continue;
                        }
                        let handled = match routed {
                            Some((kind, event)) => uploads.event(kind, event, &text).await,
                            None => Handled::NotUpload,
                        };
                        match handled {
                            Handled::NotUpload => {}
                            Handled::Answer(frames) => {
                                for frame in frames {
                                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                                        return ScreenEnd::Left;
                                    }
                                }
                                continue;
                            }
                            Handled::Up(frames) => {
                                for frame in frames {
                                    if relay.up(frame).await.is_err() {
                                        return ScreenEnd::LinkLost;
                                    }
                                }
                                continue;
                            }
                        }
                        if let Some((Kind::Key, event)) = routed {
                            let reply = take_terminal_key(state, connection, event);
                            match reply {
                                Ok(Some(pane)) => {
                                    if let Some(notice) = told.notice(&pane, Instant::now())
                                        && relay.up(tungstenite::Message::Text(notice.into())).await.is_err()
                                    {
                                        return ScreenEnd::LinkLost;
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    let frame = json!({"type":"error","payload":{},"message": error});
                                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                                        return ScreenEnd::Left;
                                    }
                                }
                            }
                            continue;
                        }
                        if relay.up(tungstenite::Message::Text(text.as_str().into())).await.is_err() {
                            return ScreenEnd::LinkLost;
                        }
                    }
                    // Every binary frame a screen sends is an upload chunk,
                    // staged on this machine (`node_uploads`).
                    Message::Binary(bytes) => {
                        for frame in uploads.chunk(&bytes) {
                            if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                                return ScreenEnd::Left;
                            }
                        }
                    }
                    Message::Close(_) => return ScreenEnd::Left,
                    _ => {}
                }
            }
        }
    }
}

/// Why a screen's relay gives no more frames.
#[derive(Clone, Debug, Eq, PartialEq)]
enum RelayEnd {
    /// The core or the link closed it; with the core's refusal of the
    /// screen (4001 to 4004), which the screen is told as is.
    Closed(Option<CloseFrame>),
    /// The screen left more than [`SCREEN_BACKLOG_BYTES`] untaken.
    Overflowed,
}

/// The core frames one screen's relay read that the screen has not taken.
#[derive(Default)]
struct Backlog {
    frames: VecDeque<tungstenite::Message>,
    bytes: usize,
    end: Option<RelayEnd>,
}

impl Backlog {
    /// Holds `frame`, or, past the cap, drops every frame held and ends the
    /// relay. One frame is always held, however large (a file read sends
    /// 4 MiB at a time), so only a screen that leaves frames untaken falls
    /// behind. Answers whether the relay goes on.
    fn push(&mut self, frame: tungstenite::Message) -> bool {
        if self.end.is_some() {
            return false;
        }
        if !self.frames.is_empty() && self.bytes + frame.len() > SCREEN_BACKLOG_BYTES {
            self.frames.clear();
            self.bytes = 0;
            self.end = Some(RelayEnd::Overflowed);
            return false;
        }
        self.bytes += frame.len();
        self.frames.push_back(frame);
        true
    }

    fn pop(&mut self) -> Option<tungstenite::Message> {
        let frame = self.frames.pop_front()?;
        self.bytes -= frame.len();
        Some(frame)
    }

    fn end(&mut self, end: RelayEnd) {
        self.end.get_or_insert(end);
    }
}

#[derive(Default)]
struct RelayShared {
    backlog: Mutex<Backlog>,
    ready: Notify,
}

/// One screen's relay to its core. Its task reads the core whatever the
/// screen takes: the connection to the core closes a stream whose reader
/// falls behind rather than stall every other screen, the panes and the
/// link (`link_mux`), so a relay left unread would lose its screen.
struct ScreenRelay {
    shared: Arc<RelayShared>,
    to_core: mpsc::Sender<tungstenite::Message>,
    task: tokio::task::JoinHandle<()>,
}

impl ScreenRelay {
    /// The relay of the screen `connection`, opened with `handshake`.
    async fn open(link: &LiveLink, handshake: &str, connection: u64) -> Result<Self, String> {
        let mut upstream = open_relay(link, "screen").await?;
        upstream
            .send(tungstenite::Message::Text(handshake.into()))
            .await
            .map_err(|error| error.to_string())?;
        let shared = Arc::new(RelayShared::default());
        let (to_core, mut from_screen) = mpsc::channel(SCREEN_TO_CORE);
        let task = tokio::spawn({
            let shared = Arc::clone(&shared);
            async move {
                let (mut sink, mut stream) = upstream.split();
                let read = async {
                    loop {
                        match stream.next().await {
                            Some(Ok(
                                frame @ (tungstenite::Message::Text(_)
                                | tungstenite::Message::Binary(_)),
                            )) => {
                                if !lock(&shared.backlog).push(frame) {
                                    // Logged now: the screen may not read for a while.
                                    herdr_core::diagnostic!(json!({
                                        "component": "node_daemon",
                                        "kind": "screen.fell_behind",
                                        "connection": connection,
                                        "cap": SCREEN_BACKLOG_BYTES,
                                    }));
                                    shared.ready.notify_one();
                                    return;
                                }
                                shared.ready.notify_one();
                            }
                            Some(Ok(tungstenite::Message::Close(frame))) => {
                                let refusal = frame
                                    .filter(|frame| (4001..=4004).contains(&u16::from(frame.code)))
                                    .map(|frame| CloseFrame {
                                        code: frame.code.into(),
                                        reason: frame.reason.as_str().into(),
                                    });
                                lock(&shared.backlog).end(RelayEnd::Closed(refusal));
                                return;
                            }
                            None | Some(Err(_)) => return,
                            Some(Ok(_)) => {}
                        }
                    }
                };
                let write = async {
                    while let Some(frame) = from_screen.recv().await {
                        if sink.send(frame).await.is_err() {
                            return;
                        }
                    }
                    let _ = sink.close().await;
                };
                tokio::select! {
                    () = read => {}
                    () = write => {}
                }
                lock(&shared.backlog).end(RelayEnd::Closed(None));
                shared.ready.notify_one();
            }
        });
        Ok(Self {
            shared,
            to_core,
            task,
        })
    }

    /// Sends a screen's frame up, waiting while the relay is full.
    async fn up(&self, frame: tungstenite::Message) -> Result<(), ()> {
        self.to_core.send(frame).await.map_err(|_| ())
    }
}

impl Drop for ScreenRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A screen's handshake as its core is sent it: without this daemon's
/// screen token, which the core does not read (the relay grant admits the
/// screen) and has no use for.
fn forwarded_handshake(handshake: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(handshake) else {
        return handshake.to_owned();
    };
    if let Some(token) = value.get_mut("token") {
        *token = Value::String(String::new());
    }
    value.to_string()
}

/// How a screen's session ended, and what it is told.
#[derive(Debug)]
enum ScreenEnd {
    /// The screen left.
    Left,
    /// The link to the core ended: the screen reconnects to the held node.
    LinkLost,
    /// The screen left more than [`SCREEN_BACKLOG_BYTES`] of the core's
    /// frames untaken.
    FellBehind,
    /// The core refused the screen, with its own close (4001 to 4004).
    Refused(CloseFrame),
}

impl ScreenEnd {
    fn reason(&self) -> &str {
        match self {
            Self::Left => "screen_closed",
            Self::LinkLost => "link_ended",
            Self::FellBehind => "fell_behind",
            Self::Refused(frame) => frame.reason.as_str(),
        }
    }

    /// The close the screen is sent; none for a screen that left.
    fn close(self) -> Option<CloseFrame> {
        match self {
            Self::Left => None,
            Self::LinkLost => Some(CloseFrame {
                code: 1012,
                reason: "core_link_lost".into(),
            }),
            Self::FellBehind => Some(CloseFrame {
                code: 1013,
                reason: "screen_fell_behind".into(),
            }),
            Self::Refused(frame) => Some(frame),
        }
    }
}

/// Reads the screen and drops what it sends until it leaves.
async fn drop_until_closed(socket: &mut WebSocket, state: &NodeState) {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
            Some(Ok(_)) => {
                state.held_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameStart {
    Snapshot,
    Delta,
}

/// A core frame's `type`, wherever it stands among the frame's keys; the
/// rest of the frame is skipped, not built.
#[derive(Deserialize)]
struct FrameHead {
    #[serde(rename = "type")]
    kind: FrameType,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FrameType {
    Snapshot,
    Delta,
    #[serde(other)]
    Other,
}

/// Whether a core frame is a whole snapshot or a delta, read from its
/// `type` alone.
fn frame_kind(text: &str) -> Option<FrameStart> {
    match serde_json::from_str::<FrameHead>(text).ok()?.kind {
        FrameType::Snapshot => Some(FrameStart::Snapshot),
        FrameType::Delta => Some(FrameStart::Delta),
        FrameType::Other => None,
    }
}

/// Where a screen's terminals start in this daemon's hub, by the first
/// core frame it got, as a core's daemon decides it.
fn resume(first: FrameStart, handshake: &Handshake) -> Resume {
    match (
        first,
        handshake.have_terminal_sequence,
        handshake.have_terminal_epoch.clone(),
    ) {
        (FrameStart::Snapshot, _, _) => Resume::Fresh,
        (FrameStart::Delta, Some(cursor), Some(epoch)) => Resume::After { epoch, cursor },
        (FrameStart::Delta, _, _) => Resume::Redraw,
    }
}

/// The kinds of a screen's events this daemon takes before the core: a read
/// of a file here, an upload, a key, a move of the core, which this
/// machine's own hided drives (PRD core-host-node-move amendment 2), and
/// the operator ending or restoring the link (B16).
const TAKEN_HERE: &[Kind] = &[
    Kind::FileBytes,
    Kind::AttachmentStage,
    Kind::AttachmentCancel,
    Kind::AttachmentCommit,
    Kind::Key,
    Kind::CoreMove,
    Kind::CoreLink,
];

/// A screen connection's hold on the dial its Retry starts: dropping it, as
/// the connection ends, ends that dial's sign-in (B8).
struct Press {
    release: Arc<dyn Fn(u64) + Send + Sync>,
    connection: u64,
}

impl Drop for Press {
    fn drop(&mut self) {
        (self.release)(self.connection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame is a snapshot or a delta by its `type`, wherever the core's
    /// serializer put that key; any other frame starts nothing.
    #[test]
    fn a_frame_s_kind_is_its_type_wherever_the_key_stands() {
        assert_eq!(
            frame_kind(r#"{"type":"snapshot","revision":1}"#),
            Some(FrameStart::Snapshot)
        );
        assert_eq!(
            frame_kind(r#"{"revision":2,"changes":{"type":"x"},"type":"delta"}"#),
            Some(FrameStart::Delta)
        );
        assert_eq!(frame_kind(r#"{"type":"error","message":"no"}"#), None);
        assert_eq!(frame_kind(r#"{"revision":3}"#), None);
        assert_eq!(frame_kind("not json"), None);
    }

    #[test]
    fn a_screen_that_falls_behind_drops_its_backlog_past_the_cap() {
        let mut backlog = Backlog::default();
        let whole = tungstenite::Message::Binary(vec![0_u8; SCREEN_BACKLOG_BYTES + 64].into());
        assert!(backlog.push(whole), "one frame is held however large");
        assert!(backlog.pop().is_some());
        let frame = || tungstenite::Message::Binary(vec![0_u8; 1024 * 1024].into());
        for _ in 0..4 {
            assert!(backlog.push(frame()));
        }
        assert_eq!(backlog.end, None);
        assert!(backlog.pop().is_some(), "a frame taken makes room");
        assert!(backlog.push(frame()));
        assert!(!backlog.push(tungstenite::Message::Text("x".into())));
        assert_eq!(backlog.end, Some(RelayEnd::Overflowed));
        assert!(backlog.pop().is_none(), "what the screen missed is dropped");
        assert!(
            !backlog.push(tungstenite::Message::Text("y".into())),
            "nothing is held after the end"
        );
        backlog.end(RelayEnd::Closed(None));
        assert_eq!(
            backlog.end,
            Some(RelayEnd::Overflowed),
            "the first end stands"
        );
    }

    #[test]
    fn a_screen_s_handshake_reaches_its_core_without_its_token() {
        let forwarded: Value = serde_json::from_str(&forwarded_handshake(
            r#"{"token":"secret","schema_version":2,"have_revision":7}"#,
        ))
        .unwrap();
        assert_eq!(
            forwarded,
            json!({"token":"","schema_version":2,"have_revision":7})
        );
    }
}
