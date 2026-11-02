use crate::{
    event::Event,
    limits::*,
    state::{End, Rejection, State},
};
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::admission::{
    ADMISSION_TIMEOUT_MS, ClientMessage, PendingAdmission, Refusal, ServerMessage,
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
    sync::{mpsc, watch},
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
}
impl Shared {
    fn emit(&self, events: Vec<Event>) {
        for event in events {
            (self.events)(event);
        }
    }
    fn rejected(&self, id: Option<u64>, error: Rejection) {
        let room = id.and_then(|id| self.state.lock().unwrap().room(id));
        (self.events)(Event::new(error.event, room, error.cap));
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
    output: mpsc::Receiver<Message>,
    end: watch::Receiver<Option<End>>,
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
        if !limits.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid_limits",
            ));
        }
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::new(limits)),
                clock: Arc::new(clock),
                events: Arc::new(events),
            }),
            limits,
            trusted_proxy,
        })
    }

    /// The caller owns the listener and shutdown signal. Dropping this future
    /// aborts every socket task; each task's registration releases its peers.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()>,
    ) -> io::Result<()> {
        let mut tasks = JoinSet::new();
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
        (self.shared.events)(Event::new("server_stopping", None, None));
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
            (self.shared.events)(Event::new(
                "shutdown_timeout",
                None,
                Some("shutdown_deadline"),
            ));
            return Err(io::Error::other("shutdown_incomplete"));
        }
        result
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
                        let (tx, output) = mpsc::channel(MAX_QUEUED_FRAMES);
                        let (end_tx, end) = watch::channel(None);
                        let id =
                            state
                                .state
                                .lock()
                                .unwrap()
                                .reserve(ip, (state.clock)(), tx, end_tx)?;
                        Ok(Admission {
                            registration: Registration {
                                shared: state.clone(),
                                id,
                            },
                            output,
                            end,
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
        (shared.events)(Event::new("http_failed", None, None));
        return;
    }
    let next = upgrade.lock().unwrap().take();
    let Some(next) = next else { return };
    let Ok(Ok(io)) = timeout(HTTP_TIMEOUT, next.on_upgrade).await else {
        (shared.events)(Event::new("upgrade_failed", None, None));
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
                let Some(message) = outgoing else {
                    // Removing a peer closes its queue after publishing its end.
                    // Either receiver may wake first; retain the published reason.
                    break (*admission.end.borrow_and_update()).expect("closed peer queue carries its end");
                };
                if !matches!(timeout(WRITE_TIMEOUT, socket.send(message)).await, Ok(Ok(()))) {
                    break End::Refused(Rejection::cap(Refusal::Timeout, "write_deadline"));
                }
            }
            incoming = socket.next() => {
                let result = match incoming {
                    Some(Ok(Message::Text(text))) => control(&shared, id, &text, &mut pending, &mut admitted),
                    Some(Ok(Message::Binary(bytes))) if admitted => shared.state.lock().unwrap().forward(id, bytes.to_vec(), (shared.clock)()),
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => Ok(()),
                    Some(Ok(Message::Close(_))) | None => break End::PeerClosed,
                    Some(Err(tungstenite::Error::Capacity(_))) => Err(Rejection::cap(Refusal::FrameLimit, "frame_size")),
                    Some(Err(_)) => break End::Refused(Rejection::new(Refusal::InvalidMessage, "websocket_failed")),
                    _ => Err(Rejection::new(Refusal::InvalidMessage, "invalid_message")),
                };
                if let Err(error) = result { break End::Refused(error); }
                if admitted { deadline = tokio::time::Instant::now() + IDLE_TIMEOUT; }
                // Flush automatic WebSocket pong responses with the same write bound.
                if !matches!(timeout_at(deadline.min(tokio::time::Instant::now() + WRITE_TIMEOUT), socket.flush()).await, Ok(Ok(()))) {
                    break End::Refused(Rejection::cap(Refusal::Timeout, "write_deadline"));
                }
            }
        }
    };
    if let End::Refused(error) = end {
        shared.rejected(Some(id), error);
    }
    // Unregister before awaiting the final write, so other peers see the loss
    // even when the offender no longer reads its socket.
    drop(admission.registration);
    finish(&mut socket, end).await;
}

fn control(
    shared: &Shared,
    id: u64,
    text: &str,
    pending: &mut Option<PendingAdmission>,
    admitted: &mut bool,
) -> Result<(), Rejection> {
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
            shared.state.lock().unwrap().register(id, hello, now)?;
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

async fn finish(socket: &mut Socket, end: End) {
    let deadline = tokio::time::Instant::now() + WRITE_TIMEOUT;
    let oversized = matches!(
        end,
        End::Refused(Rejection {
            cap: Some("frame_size"),
            ..
        })
    );
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
    if oversized {
        // The parser rejects the length before reading its payload. Dropping a
        // TCP socket with unread bytes can reset it and erase the refusal. Send
        // FIN, then discard a bounded remainder within the same write deadline.
        let _ = timeout_at(deadline, async {
            socket.get_mut().shutdown().await?;
            let mut remaining = socket.get_mut().take(MAX_CLOSE_DRAIN_BYTES as u64);
            tokio::io::copy(&mut remaining, &mut tokio::io::sink()).await
        })
        .await;
    }
}
