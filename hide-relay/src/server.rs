use crate::{
    apns::{Apns, Sent},
    event::Event,
    fcm::Fcm,
    limits::*,
    state::{End, Outbound, PushCap, Rejection, State},
};
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::admission::{
    ADMISSION_TIMEOUT_MS, ClientMessage, PendingAdmission, Push, PushAttempt, PushOutcome,
    PushResult, REGISTERED_FIELD_FCM, REGISTERED_FIELDS_HEADER, Refusal, ServerMessage,
    decode_push, registered_fields_name,
};
use http_body_util::Full;
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    future::Future,
    io,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, mpsc, watch},
    task::JoinSet,
    time::{timeout, timeout_at},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        handshake::server::create_response_with_body,
        protocol::{CloseFrame, Role as WsRole, WebSocketConfig, frame::coding::CloseCode},
    },
};

type Socket = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;
type EventSink = Arc<dyn Fn(Event) + Send + Sync>;
type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

struct Shared {
    state: Mutex<State>,
    clock: Clock,
    events: EventSink,
    idle_timeout: Duration,
    /// `None` until the relay is given Apple's key; every push request then
    /// answers `unavailable` (`docs/RELAY_PROTOCOL.md`, Push relay).
    apns: Option<Arc<Apns>>,
    /// `None` until the relay is given an FCM project; every FCM request then
    /// answers `unavailable`.
    fcm: Option<Arc<Fcm>>,
    /// Requests in flight to Apple and Google together (`concurrent_pushes`).
    in_flight: Arc<Semaphore>,
    /// Set when `serve` ends, on every way out: each push send in flight
    /// stops at once rather than outliving the relay.
    stopping: tokio::sync::watch::Sender<bool>,
}
impl Shared {
    fn emit(&self, events: Vec<Event>) {
        for event in events {
            (self.events)(event);
        }
    }
    fn rejected(&self, id: Option<u64>, error: Rejection) {
        let state = self.state.lock().unwrap();
        // Owner removal already logged the refusal.
        if id.is_some_and(|id| state.role(id).is_err()) {
            return;
        }
        drop(state);
        (self.events)(Event::new(error.event, error.cap));
    }
}
struct Registration {
    shared: Arc<Shared>,
    id: u64,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let events = self
            .shared
            .state
            .lock()
            .unwrap()
            .remove(self.id, End::PeerClosed);
        self.shared.emit(events);
    }
}
struct Admission {
    registration: Registration,
    output: mpsc::Receiver<Outbound>,
    end: watch::Receiver<Option<End>>,
    /// Whether the device named `fcm` in `REGISTERED_FIELDS_HEADER`.
    reads_fcm: bool,
}
struct Upgrade {
    on_upgrade: hyper::upgrade::OnUpgrade,
    admission: Result<Admission, Rejection>,
}

/// An independently owned service, with no global state or on-disk state.
pub struct Relay {
    shared: Arc<Shared>,
    limits: Limits,
    trusted_proxy: Option<IpAddr>,
}
impl Relay {
    pub fn new(
        limits: Limits,
        trusted_proxy: Option<IpAddr>,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let started = Instant::now();
        Self::with_clock(limits, trusted_proxy, events, move || started.elapsed())
    }
    /// A monotonic uptime source, shared by admission and traffic windows.
    pub fn with_clock(
        limits: Limits,
        trusted_proxy: Option<IpAddr>,
        events: impl Fn(Event) + Send + Sync + 'static,
        clock: impl Fn() -> Duration + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::with_push(limits, trusted_proxy, events, clock, None)
    }
    /// A relay that forwards push requests to Apple through `apns`. Without
    /// one it answers `registered {push: false}` and every push request
    /// `unavailable`, which is the state of a relay without the key.
    pub fn with_push(
        limits: Limits,
        trusted_proxy: Option<IpAddr>,
        events: impl Fn(Event) + Send + Sync + 'static,
        clock: impl Fn() -> Duration + Send + Sync + 'static,
        apns: Option<Apns>,
    ) -> io::Result<Self> {
        let push_limits = PushLimits::default();
        if !limits.valid() || !push_limits.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid_limits",
            ));
        }
        let clock: Clock = Arc::new(clock);
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::new(limits, push_limits, apns.is_some())),
                clock: clock.clone(),
                events: Arc::new(events),
                idle_timeout: limits.idle_timeout,
                apns: apns.map(Arc::new),
                fcm: None,
                in_flight: Arc::new(Semaphore::new(push_limits.concurrent)),
                stopping: tokio::sync::watch::Sender::new(false),
            }),
            limits,
            trusted_proxy,
        })
    }
    /// Lower push budgets for a test; they may only be lowered.
    pub fn with_push_limits(mut self, push: PushLimits) -> io::Result<Self> {
        if !push.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid_limits",
            ));
        }
        let shared = Arc::get_mut(&mut self.shared)
            .ok_or_else(|| io::Error::other("relay already serving"))?;
        shared.state.get_mut().unwrap().set_push_limits(push);
        shared.in_flight = Arc::new(Semaphore::new(push.concurrent));
        Ok(self)
    }
    /// A relay that also forwards FCM requests to Google through `fcm`, under
    /// the same push budgets as Apple's (`docs/RELAY_PROTOCOL.md`, Push
    /// relay). Without it every FCM request answers `unavailable`.
    pub fn with_fcm(mut self, fcm: Fcm) -> io::Result<Self> {
        let shared = Arc::get_mut(&mut self.shared)
            .ok_or_else(|| io::Error::other("relay already serving"))?;
        shared.state.get_mut().unwrap().set_fcm_enabled();
        shared.fcm = Some(Arc::new(fcm));
        Ok(self)
    }

    /// The caller owns the listener and shutdown signal. Dropping this future
    /// aborts every socket task; each task's registration releases its peers.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()>,
    ) -> io::Result<()> {
        let mut tasks = JoinSet::new();
        // Push sends run beside the socket tasks; this ends them when `serve`
        // returns or is dropped.
        let _pushes = StopPushes(Arc::clone(&self.shared));
        tokio::pin!(shutdown);
        let result = loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => break Ok(()),
                result = tasks.join_next(), if !tasks.is_empty() => {
                    if result.is_some_and(|r| r.is_err()) {
                        break Err(io::Error::other("connection_task_failed"));
                    }
                }
                accepted = listener.accept(), if tasks.len() < self.limits.connections + MAX_PENDING_HTTP => {
                    match accepted {
                        Ok((stream, address)) => {
                            let shared = self.shared.clone();
                            let proxy = self.trusted_proxy;
                            tasks.spawn(async move { connection(stream, address.ip(), proxy, shared).await });
                        }
                        Err(error) => break Err(error),
                    }
                }
            }
        };
        drop(listener);
        let events = self.shared.state.lock().unwrap().restart();
        self.shared.emit(events);
        (self.shared.events)(Event::new("server_stopping", None));
        let drained = timeout(SHUTDOWN_TIMEOUT, async {
            while let Some(result) = tasks.join_next().await {
                result.map_err(|_| io::Error::other("connection_task_failed"))?;
            }
            Ok::<_, io::Error>(())
        })
        .await;
        if !matches!(drained, Ok(Ok(()))) {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            (self.shared.events)(Event::new("shutdown_timeout", None));
            return Err(io::Error::other("shutdown_incomplete"));
        }
        result
    }
}

struct StopPushes(Arc<Shared>);
impl Drop for StopPushes {
    fn drop(&mut self) {
        self.0.stopping.send_replace(true);
    }
}

async fn connection(stream: TcpStream, remote: IpAddr, proxy: Option<IpAddr>, shared: Arc<Shared>) {
    let upgrade = Arc::new(Mutex::new(None));
    let pending = upgrade.clone();
    let state = shared.clone();
    let service = service_fn(move |mut request: Request<Incoming>| {
        let mut response = if request.method() == hyper::Method::GET && request.uri() == "/health" {
            Response::new(Full::new(Bytes::from_static(b"ok\n")))
        } else if request.uri() == "/ws" {
            match create_response_with_body(&request, || Full::new(Bytes::new())) {
                Ok(response) => {
                    let admission = client_ip(&request, remote, proxy).and_then(|ip| {
                        let (tx, output) = mpsc::channel(MAX_QUEUED_FRAMES + MAX_QUEUED_CONTROLS);
                        let (end_tx, end) = watch::channel(None);
                        let id =
                            state
                                .state
                                .lock()
                                .unwrap()
                                .reserve(ip, (state.clock)(), tx, end_tx)?;
                        // A device names the `registered` fields it reads; a
                        // released one names none and gets the shape it knows.
                        let reads_fcm = request
                            .headers()
                            .get_all(REGISTERED_FIELDS_HEADER)
                            .iter()
                            .filter_map(|value| value.to_str().ok())
                            .any(|value| registered_fields_name(value, REGISTERED_FIELD_FCM));
                        Ok(Admission {
                            registration: Registration {
                                shared: state.clone(),
                                id,
                            },
                            output,
                            end,
                            reads_fcm,
                        })
                    });
                    if let Err(error) = admission.as_ref() {
                        state.rejected(None, *error);
                    }
                    *pending.lock().unwrap() = Some(Upgrade {
                        on_upgrade: hyper::upgrade::on(&mut request),
                        admission,
                    });
                    response
                }
                Err(_) => http_error(StatusCode::BAD_REQUEST),
            }
        } else {
            http_error(StatusCode::NOT_FOUND)
        };
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            response.headers_mut().insert(
                hyper::header::CONNECTION,
                hyper::header::HeaderValue::from_static("close"),
            );
        }
        async { Ok::<_, Infallible>(response) }
    });
    let http = http1::Builder::new()
        .max_headers(32)
        .max_buf_size(8192)
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    if !matches!(timeout(HTTP_TIMEOUT, http).await, Ok(Ok(()))) {
        (shared.events)(Event::new("http_failed", None));
        return;
    }
    let next = upgrade.lock().unwrap().take();
    let Some(next) = next else { return };
    let Ok(Ok(io)) = timeout(HTTP_TIMEOUT, next.on_upgrade).await else {
        (shared.events)(Event::new("upgrade_failed", None));
        return;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME))
        .read_buffer_size(8192)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_FRAME + 1024);
    let mut socket =
        WebSocketStream::from_raw_socket(TokioIo::new(io), WsRole::Server, Some(config)).await;
    match next.admission {
        Ok(admission) => session(socket, admission).await,
        Err(error) => finish(&mut socket, End::Refused(error)).await,
    }
}

fn http_error(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from_static(b"unavailable\n")));
    *response.status_mut() = status;
    response
}
fn client_ip(
    request: &Request<Incoming>,
    remote: IpAddr,
    proxy: Option<IpAddr>,
) -> Result<IpAddr, Rejection> {
    if proxy != Some(remote) {
        return Ok(remote.to_canonical());
    }
    let mut headers = request.headers().get_all("x-real-ip").iter();
    let ip = headers
        .next()
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<IpAddr>().ok());
    match (ip, headers.next()) {
        (Some(ip), None) => Ok(ip.to_canonical()),
        _ => Err(Rejection::new(
            Refusal::InvalidMessage,
            "invalid_proxy_address",
        )),
    }
}

async fn session(mut socket: Socket, mut admission: Admission) {
    let shared = admission.registration.shared.clone();
    let id = admission.registration.id;
    let mut pending = None;
    let mut admitted = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_millis(ADMISSION_TIMEOUT_MS);
    let end = loop {
        // Closing has a dedicated channel, so a full data queue cannot hide it.
        if let Some(end) = *admission.end.borrow_and_update() {
            break end;
        }
        tokio::select! {
            changed = admission.end.changed() => {
                if changed.is_err() { break End::PeerClosed; }
            }
            _ = tokio::time::sleep_until(deadline) => {
                break End::Refused(Rejection::new(Refusal::Timeout, if admitted { "idle_timeout" } else { "admission_timeout" }));
            }
            outgoing = admission.output.recv() => {
                let Some(outbound) = outgoing else {
                    // Removing a peer closes its queue after publishing its end.
                    // Either receiver may wake first; retain the published reason.
                    break (*admission.end.borrow_and_update()).expect("closed peer queue carries its end");
                };
                // Hold the class-specific slot through the write as well.
                let _capacity = outbound.capacity;
                if !matches!(timeout(WRITE_TIMEOUT, socket.send(outbound.message)).await, Ok(Ok(()))) {
                    break End::Refused(Rejection::new(Refusal::Timeout, "write_timeout"));
                }
            }
            incoming = socket.next() => {
                let result = match incoming {
                    Some(Ok(Message::Text(text))) => control(&shared, id, &text, &mut pending, &mut admitted, admission.reads_fcm),
                    Some(Ok(Message::Binary(bytes))) if admitted => {
                        let result = shared.state.lock().unwrap().forward(id, bytes.to_vec(), (shared.clock)());
                        result.map(|events| shared.emit(events))
                    },
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => Ok(()),
                    Some(Ok(Message::Close(_))) | None => break End::PeerClosed,
                    Some(Err(tungstenite::Error::Capacity(_))) => Err(Rejection::cap(Refusal::FrameLimit, "frame_size")),
                    Some(Err(_)) => break End::Refused(Rejection::new(Refusal::InvalidMessage, "websocket_failed")),
                    _ => Err(Rejection::new(Refusal::InvalidMessage, "invalid_message")),
                };
                if let Err(error) = result { break End::Refused(error); }
                if admitted { deadline = tokio::time::Instant::now() + shared.idle_timeout; }
                // Flush automatic WebSocket pong responses with the same write bound.
                if !matches!(timeout_at(deadline.min(tokio::time::Instant::now() + WRITE_TIMEOUT), socket.flush()).await, Ok(Ok(()))) {
                    break End::Refused(Rejection::new(Refusal::Timeout, "write_timeout"));
                }
            }
        }
    };
    // An owner-initiated close can race an input already selected for reading.
    // Its published reason remains authoritative after membership is removed.
    let end = (*admission.end.borrow_and_update()).unwrap_or(end);
    if let End::Refused(error) = end {
        shared.rejected(Some(id), error);
    }
    // Unregister before awaiting the final write, so other peers see the loss
    // even when the offender no longer reads its socket.
    drop(admission.registration);
    finish(&mut socket, end).await;
}

fn control(
    shared: &Arc<Shared>,
    id: u64,
    text: &str,
    pending: &mut Option<PendingAdmission>,
    admitted: &mut bool,
    reads_fcm: bool,
) -> Result<(), Rejection> {
    // A push message is read before the rest: one whose fields are wrong is
    // still answered under its own id, and the room stays connected
    // (`docs/RELAY_PROTOCOL.md`, Push relay).
    match decode_push(text) {
        PushAttempt::Push {
            id: push_id,
            request,
        } if *admitted => {
            return push(shared, id, text.len() as u64, push_id, request);
        }
        PushAttempt::Push { .. } => {
            return Err(Rejection::new(
                Refusal::InvalidMessage,
                "invalid_transition",
            ));
        }
        PushAttempt::Unaddressable(error) => {
            return Err(if error == hide_relay_protocol::Error::SizeLimit {
                Rejection::cap(Refusal::FrameLimit, "control_size")
            } else {
                crate::protocol_error(error)
            });
        }
        PushAttempt::NotPush => {}
    }
    let message = ClientMessage::decode(text).map_err(|error| {
        if error == hide_relay_protocol::Error::SizeLimit {
            Rejection::cap(Refusal::FrameLimit, "control_size")
        } else {
            crate::protocol_error(error)
        }
    })?;
    let now = (shared.clock)();
    let now_ms = u64::try_from(now.as_millis())
        .map_err(|_| Rejection::new(Refusal::Timeout, "clock_overflow"))?;
    match message {
        ClientMessage::Device { hello } if !*admitted && pending.is_none() => {
            let challenge = PendingAdmission::new(hello, now_ms).map_err(crate::protocol_error)?;
            let message = ServerMessage::Challenge {
                challenge: challenge.challenge().clone(),
            };
            // Queue before retaining the challenge, with the same bounded path.
            let result = shared.state.lock().unwrap().control(id, message);
            result?;
            *pending = Some(challenge);
        }
        ClientMessage::Proof { signature } if !*admitted => {
            let challenge = pending
                .take()
                .ok_or(Rejection::new(Refusal::InvalidProof, "unexpected_proof"))?;
            let hello = challenge
                .verify(&signature, now_ms)
                .map_err(crate::protocol_error)?;
            shared
                .state
                .lock()
                .unwrap()
                .register(id, hello, now, reads_fcm)?;
            *admitted = true;
        }
        ClientMessage::Proof { .. } => {
            return Err(Rejection::new(Refusal::InvalidProof, "replayed_admission"));
        }
        ClientMessage::Phone { room, .. } if !*admitted && pending.is_none() => {
            shared.state.lock().unwrap().join(id, room)?;
            *admitted = true;
        }
        ClientMessage::Close { connection } if *admitted => {
            let events = shared.state.lock().unwrap().close_phone(id, connection)?;
            shared.emit(events);
        }
        _ => {
            return Err(Rejection::new(
                Refusal::InvalidMessage,
                "invalid_transition",
            ));
        }
    }
    Ok(())
}

/// One push request from an admitted device socket, for Apple or for Google:
/// charged and counted against the one set of push budgets, checked, then sent
/// beside the socket loop so the room keeps reading. Every answer is a
/// `push_result` under the request's id; only the room's byte budget, as for
/// any frame, closes the socket.
fn push(
    shared: &Arc<Shared>,
    id: u64,
    size: u64,
    push_id: u32,
    request: Result<Push, hide_relay_protocol::admission::PushField>,
) -> Result<(), Rejection> {
    let now = (shared.clock)();
    // Only a room's device socket asks for a push; a phone socket keeps the
    // existing closed refusal. Every request is counted, a malformed one
    // included.
    let cap = shared.state.lock().unwrap().push_cap(id, size, now)?;
    if let PushCap::Over { cap, log } = cap {
        return answer_push_result(shared, id, push_id, rate_limited(shared, cap, log));
    }
    let request = match request {
        Ok(request) => request,
        Err(field) => {
            (shared.events)(Event::new(field_event(field), None));
            return answer_push_result(shared, id, push_id, PushOutcome::Failed);
        }
    };
    let service = match request {
        Push::Apns(request) => shared.apns.clone().map(|apns| Service::Apns(apns, request)),
        Push::Fcm(request) => shared.fcm.clone().map(|fcm| Service::Fcm(fcm, request)),
    };
    let Some(service) = service else {
        return answer_push_result(shared, id, push_id, PushOutcome::Unavailable);
    };
    let Ok(slot) = Arc::clone(&shared.in_flight).try_acquire_owned() else {
        let log = shared.state.lock().unwrap().log_concurrent_cap(now);
        return answer_push_result(
            shared,
            id,
            push_id,
            rate_limited(shared, "concurrent_pushes", log),
        );
    };
    // Beside the socket loop: waiting out Apple or Google must not stop the
    // room from reading its next frame. The send, and its in-flight slot, end
    // with the relay.
    let shared = Arc::clone(shared);
    let mut stopping = shared.stopping.subscribe();
    tokio::spawn(async move {
        tokio::select! {
            sent = service.send() => {
                // The service has answered: the slot is free before the
                // device reads the result.
                drop(slot);
                (shared.events)(Event::new(sent.reason, None));
                let _ = answer_push_result(&shared, id, push_id, sent.outcome);
            }
            _ = stopping.wait_for(|stopping| *stopping) => {}
        }
    });
    Ok(())
}

/// The service one checked request goes to.
enum Service {
    Apns(Arc<Apns>, hide_relay_protocol::admission::PushRequest),
    Fcm(Arc<Fcm>, hide_relay_protocol::admission::FcmRequest),
}
impl Service {
    async fn send(&self) -> Sent {
        match self {
            Self::Apns(apns, request) => apns.send(request).await,
            Self::Fcm(fcm, request) => fcm.send(request).await,
        }
    }
}

/// A refusal past a cap: logged once per window, answered every time.
fn rate_limited(shared: &Shared, cap: &'static str, log: bool) -> PushOutcome {
    if log {
        (shared.events)(Event::new("cap_exceeded", Some(cap)));
    }
    PushOutcome::RateLimited
}

/// The fixed event each malformed field is recorded under, and never a value.
fn field_event(field: hide_relay_protocol::admission::PushField) -> &'static str {
    use hide_relay_protocol::admission::PushField;
    match field {
        PushField::Token => "push_token_invalid",
        PushField::Environment => "push_environment_invalid",
        PushField::Payload | PushField::Collapse => "push_payload_invalid",
        PushField::Shape => "push_invalid",
    }
}

/// Queues one result under its request id, on the same bounded control path
/// as every other answer. A full queue ends the socket as it does elsewhere.
fn answer_push_result(
    shared: &Shared,
    id: u64,
    push_id: u32,
    result: PushOutcome,
) -> Result<(), Rejection> {
    shared.state.lock().unwrap().control(
        id,
        ServerMessage::PushResult(PushResult {
            id: push_id,
            result,
        }),
    )
}

async fn finish(socket: &mut Socket, end: End) {
    let deadline = tokio::time::Instant::now() + WRITE_TIMEOUT;
    let refused = matches!(end, End::Refused(_));
    let (code, reason) = match end {
        End::Restart => (CloseCode::Restart, "restart"),
        End::PeerClosed => (CloseCode::Normal, "peer_closed"),
        End::Refused(error) => {
            if let Ok(text) = (ServerMessage::Refused {
                reason: error.reason,
            })
            .encode()
            {
                let _ = timeout_at(deadline, socket.send(Message::Text(text.into()))).await;
            }
            (
                if error.cap.is_some() {
                    CloseCode::Again
                } else {
                    CloseCode::Policy
                },
                error.event,
            )
        }
    };
    let _ = timeout_at(
        deadline,
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
    if refused {
        // An offender may already have pipelined its next frame. Dropping TCP
        // with unread bytes can reset it and erase the refusal. Send FIN and
        // discard a bounded remainder within the same write deadline. If that
        // budget fills, keep the half-closed socket until the deadline so the
        // reader can receive the refusal without us consuming unlimited input.
        let _ = timeout_at(deadline, async {
            socket.get_mut().shutdown().await?;
            let mut remaining = socket.get_mut().take(MAX_CLOSE_DRAIN_BYTES as u64);
            let discarded = tokio::io::copy(&mut remaining, &mut tokio::io::sink()).await?;
            if discarded == MAX_CLOSE_DRAIN_BYTES as u64 {
                tokio::time::sleep_until(deadline).await;
            }
            Ok::<_, io::Error>(())
        })
        .await;
    }
}
