use futures_util::{SinkExt, StreamExt};
use hide_relay::{
    Relay,
    apns::{Apns, Clock, ProviderKey},
    event::Event,
    fcm::{Fcm, FcmEndpoints},
    limits::{Limits, PushLimits, SHUTDOWN_TIMEOUT},
};
use hide_relay_protocol::{
    admission::{
        ClientMessage, DeviceHello, DeviceKey, PushOutcome, REGISTERED_FIELD_FCM,
        REGISTERED_FIELDS_HEADER, ServerMessage,
    },
    noise::{NoiseKeypair, Session},
};
use http_body_util::Full;
use hyper::{Response, StatusCode, body::Bytes, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpSocket, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type MakeApns = Box<dyn FnOnce(Clock) -> Apns>;
type MakeFcm = Box<dyn FnOnce(Clock) -> Fcm>;
pub struct Fixture {
    pub address: SocketAddr,
    pub events: Arc<Mutex<Vec<Event>>>,
    pub clock: Arc<AtomicU64>,
    /// The Unix time Apple's client reads, in milliseconds, moved by a test
    /// to walk the provider token's schedule.
    pub unix: Arc<AtomicU64>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}
impl Fixture {
    pub async fn start(limits: Limits) -> Self {
        Self::at(limits, "127.0.0.1:0".parse().unwrap()).await
    }
    pub async fn at(limits: Limits, address: SocketAddr) -> Self {
        Self::with_proxy(limits, address, true).await
    }
    pub async fn with_proxy(limits: Limits, address: SocketAddr, trusted: bool) -> Self {
        Self::build(limits, PushLimits::default(), address, trusted, None, None).await
    }
    /// A relay with a stand-in for Apple's push service, forwarding to it.
    /// Apple's client reads `unix`, which starts at the real time, so a test
    /// moves the provider token's schedule without waiting.
    pub async fn pushing(push: PushLimits, apple: &Apple, key: &TestKey) -> Self {
        Self::pushing_with(Limits::default(), push, apple, key).await
    }
    /// The same, with the relay's own budgets lowered too.
    pub async fn pushing_with(
        limits: Limits,
        push: PushLimits,
        apple: &Apple,
        key: &TestKey,
    ) -> Self {
        let endpoint = apple.url();
        let origin = key.provider();
        Self::build(
            limits,
            push,
            "127.0.0.1:0".parse().unwrap(),
            true,
            Some(Box::new(move |clock| {
                Apns::new(origin, endpoint, push, clock).unwrap()
            })),
            None,
        )
        .await
    }
    /// A relay with stand-ins for Google's FCM and its metadata server, and
    /// for Apple too when `apple` is given, so a test can prove the two share
    /// one set of budgets.
    pub async fn google(
        push: PushLimits,
        google: &Google,
        apple: Option<(&Apple, &TestKey)>,
    ) -> Self {
        let endpoints = FcmEndpoints {
            send: Some(google.fcm.url()),
            metadata: Some(google.metadata.url()),
        };
        let apns = apple.map(|(apple, key)| {
            let endpoint = apple.url();
            let origin = key.provider();
            Box::new(move |clock| Apns::new(origin, endpoint, push, clock).unwrap()) as MakeApns
        });
        Self::build(
            Limits::default(),
            push,
            "127.0.0.1:0".parse().unwrap(),
            true,
            apns,
            Some(Box::new(move |clock| {
                Fcm::new("hide-android", endpoints, push, clock).unwrap()
            })),
        )
        .await
    }
    async fn build(
        limits: Limits,
        push: PushLimits,
        address: SocketAddr,
        trusted: bool,
        apns: Option<MakeApns>,
        fcm: Option<MakeFcm>,
    ) -> Self {
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let clock = Arc::new(AtomicU64::new(0));
        let time = clock.clone();
        let unix = Arc::new(AtomicU64::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        ));
        let unix_clock = || -> Clock {
            let time = unix.clone();
            Arc::new(move || Duration::from_millis(time.load(Ordering::SeqCst)))
        };
        let apns = apns.map(|make| make(unix_clock()));
        let fcm = fcm.map(|make| make(unix_clock()));
        let relay = Relay::with_push(
            limits,
            trusted.then_some(address.ip()),
            move |event| {
                let mut log = log.lock().unwrap();
                assert!(log.len() < 20_000, "bounded fixture event capture");
                log.push(event);
            },
            move || Duration::from_millis(time.load(Ordering::SeqCst)),
            apns,
        )
        .unwrap()
        .with_push_limits(push)
        .unwrap();
        let relay = match fcm {
            Some(fcm) => relay.with_fcm(fcm).unwrap(),
            None => relay,
        };
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(relay.serve(listener, async {
            let _ = stopped.await;
        }));
        Self {
            address,
            events,
            clock,
            unix,
            stop: Some(stop),
            task: Some(task),
        }
    }
    pub async fn socket(&self, ip: &str) -> Socket {
        self.socket_naming(ip, None).await
    }
    /// A device socket whose upgrade request names `fcm` among the
    /// `registered` fields it reads, as a device that sends FCM does.
    pub async fn socket_reading_fcm(&self, ip: &str) -> Socket {
        self.socket_naming(ip, Some(REGISTERED_FIELD_FCM)).await
    }
    pub async fn socket_naming(&self, ip: &str, fields: Option<&str>) -> Socket {
        let mut request = format!("ws://{}/ws", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("x-real-ip", ip.parse().unwrap());
        if let Some(fields) = fields {
            request
                .headers_mut()
                .insert(REGISTERED_FIELDS_HEADER, fields.parse().unwrap());
        }
        timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .unwrap()
        .unwrap()
        .0
    }
    pub async fn socket_with_small_receive_window(&self, ip: &str) -> Socket {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(1024).unwrap();
        let stream = socket.connect(self.address).await.unwrap();
        let mut request = format!("ws://{}/ws", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("x-real-ip", ip.parse().unwrap());
        tokio_tungstenite::client_async(request, MaybeTlsStream::Plain(stream))
            .await
            .unwrap()
            .0
    }
    pub async fn stop(&mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        timeout(
            SHUTDOWN_TIMEOUT + Duration::from_secs(1),
            self.task.take().unwrap(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    }
    /// Asserts the relay logged the fixed event `name`.
    pub fn logged(&self, name: &str) {
        assert!(
            self.events.lock().unwrap().iter().any(|e| e.event == name),
            "missing event {name}"
        );
    }
    /// How many `cap_exceeded` lines name `cap`.
    pub fn caps(&self, name: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.event == "cap_exceeded" && e.cap == Some(name))
            .count()
    }
    pub fn cap(&self, name: &str) {
        assert!(
            self.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.event == "cap_exceeded" && e.cap == Some(name)),
            "missing cap event {name}"
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// One request a stand-in received, as the relay sent it.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

type Answer = (StatusCode, String);

/// A local HTTP/2 (h2c) stand-in for a push service, Apple's or Google's.
/// Every request is recorded, and answered with whatever the test set.
pub struct H2StandIn {
    pub address: SocketAddr,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    answer: Arc<Mutex<Answer>>,
    /// Answers for the next requests only, in order, before the standing
    /// one, so a test can make the service refuse once and then accept.
    next: Arc<Mutex<VecDeque<Answer>>>,
    /// How many TCP connections were accepted, so a test can prove the relay
    /// keeps one and does not open one per request.
    connections: Arc<AtomicU64>,
    /// Open by default; while closed, each request is recorded and its
    /// answer waits, so a test can hold the relay's sends in flight.
    gate: Arc<tokio::sync::watch::Sender<bool>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl H2StandIn {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(Mutex::new((StatusCode::OK, String::new())));
        let next = Arc::new(Mutex::new(VecDeque::new()));
        let connections = Arc::new(AtomicU64::new(0));
        let gate = Arc::new(tokio::sync::watch::Sender::new(true));
        let gated = gate.clone();
        let recorded = requests.clone();
        let answering = answer.clone();
        let once = next.clone();
        let accepted_count = connections.clone();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut stopped = stopped;
            loop {
                let accepted = tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { break };
                accepted_count.fetch_add(1, Ordering::SeqCst);
                let recorded = recorded.clone();
                let answering = answering.clone();
                let once = once.clone();
                let gated = gated.clone();
                tokio::spawn(async move {
                    // h2c: the HTTP/2 protocol before any TLS this stand-in
                    // does not carry, which is the whole point of a local
                    // stand-in (`docs/RELAY_PROTOCOL.md`, Push relay).
                    let service =
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let recorded = recorded.clone();
                            let answering = answering.clone();
                            let once = once.clone();
                            let mut gate = gated.subscribe();
                            async move {
                                let (path, headers) = record(&request);
                                let body =
                                    match http_body_util::BodyExt::collect(request.into_body())
                                        .await
                                    {
                                        Ok(collected) => collected.to_bytes().to_vec(),
                                        Err(_) => Vec::new(),
                                    };
                                recorded.lock().unwrap().push(Recorded {
                                    path,
                                    headers,
                                    body,
                                });
                                let _ = gate.wait_for(|open| *open).await;
                                let (status, body) = once
                                    .lock()
                                    .unwrap()
                                    .pop_front()
                                    .unwrap_or_else(|| answering.lock().unwrap().clone());
                                let mut response = Response::builder().status(status);
                                // A redirect names a place the relay must
                                // never go.
                                if status.is_redirection() {
                                    response = response.header("location", "http://127.0.0.1:9/");
                                }
                                Ok::<_, std::convert::Infallible>(
                                    response.body(Full::new(Bytes::from(body))).unwrap(),
                                )
                            }
                        });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            address,
            requests,
            answer,
            next,
            connections,
            gate,
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The status and body every request gets from now on.
    pub fn answer_with(&self, status: StatusCode, body: &str) {
        *self.answer.lock().unwrap() = (status, body.to_owned());
    }

    /// The status and body the next request gets, after any queued before it
    /// and before the standing one.
    pub fn answer_next_with(&self, status: StatusCode, body: &str) {
        self.next
            .lock()
            .unwrap()
            .push_back((status, body.to_owned()));
    }

    /// Holds every answer until `release`.
    pub fn hold(&self) {
        self.gate.send_replace(false);
    }
    pub fn release(&self) {
        self.gate.send_replace(true);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn connections(&self) -> u64 {
        self.connections.load(Ordering::SeqCst)
    }

    /// Waits for the `count`-th request, so a test reads what the service saw
    /// without polling a fixed number of times.
    pub async fn wait_for(&self, count: usize) -> Vec<Recorded> {
        for _ in 0..3_000 {
            let requests = self.requests();
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "the stand-in received {} of {count} requests",
            self.requests().len()
        );
    }
}

impl Drop for H2StandIn {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn record(request: &hyper::Request<hyper::body::Incoming>) -> (String, Vec<(String, String)>) {
    let path = request
        .uri()
        .path_and_query()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_default();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    (path, headers)
}

/// Apple's push service: the h2c stand-in, answering Apple's error shape.
pub struct Apple(H2StandIn);

impl std::ops::Deref for Apple {
    type Target = H2StandIn;
    fn deref(&self) -> &H2StandIn {
        &self.0
    }
}

impl Apple {
    pub async fn start() -> Self {
        Self(H2StandIn::start().await)
    }

    fn body(reason: &str) -> String {
        if reason.is_empty() {
            String::new()
        } else {
            format!(r#"{{"reason":"{reason}"}}"#)
        }
    }

    /// The answer every request gets from now on.
    pub fn answer(&self, status: StatusCode, reason: &str) {
        self.answer_with(status, &Self::body(reason));
    }

    /// The answer the next request gets, before the standing one.
    pub fn answer_next(&self, status: StatusCode, reason: &str) {
        self.answer_next_with(status, &Self::body(reason));
    }
}

/// Google: FCM's HTTP v1 endpoint as an h2c stand-in, and the GCE metadata
/// server as an HTTP/1 stand-in that issues numbered access tokens.
pub struct Google {
    pub fcm: H2StandIn,
    pub metadata: Metadata,
}

impl Google {
    pub async fn start() -> Self {
        Self {
            fcm: H2StandIn::start().await,
            metadata: Metadata::start().await,
        }
    }

    /// FCM's error shape: the gRPC status, the FCM error code and, when
    /// `field` is given, the field Google faults.
    pub fn error(status: &str, code: &str, field: Option<&str>) -> String {
        let violations = field
            .map(|field| {
                format!(
                    r#",{{"@type":"type.googleapis.com/google.rpc.BadRequest","fieldViolations":[{{"field":"{field}","description":"Invalid registration token"}}]}}"#
                )
            })
            .unwrap_or_default();
        format!(
            r#"{{"error":{{"code":400,"message":"secret-free text","status":"{status}","details":[{{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"{code}"}}{violations}]}}}}"#
        )
    }
}

/// The GCE metadata server: each token request is recorded and answered with
/// `access-<n>`, the n-th token it issued, valid for `expires_in` seconds.
pub struct Metadata {
    pub address: SocketAddr,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
    /// The lifetime of each token issued from now on, in seconds.
    pub expires_in: Arc<AtomicU64>,
    /// Whether answers carry `Metadata-Flavor: Google`, and their status.
    answer: Arc<Mutex<(StatusCode, bool)>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Metadata {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let expires_in = Arc::new(AtomicU64::new(3599));
        let answer = Arc::new(Mutex::new((StatusCode::OK, true)));
        let recorded = requests.clone();
        let lifetime = expires_in.clone();
        let answering = answer.clone();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut stopped = stopped;
            loop {
                let accepted = tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { break };
                let recorded = recorded.clone();
                let lifetime = lifetime.clone();
                let answering = answering.clone();
                tokio::spawn(async move {
                    let service = service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let (path, headers) = record(&request);
                            let issued = {
                                let mut recorded = recorded.lock().unwrap();
                                recorded.push(Recorded {
                                    path,
                                    headers,
                                    body: Vec::new(),
                                });
                                recorded.len()
                            };
                            let (status, flavored) = *answering.lock().unwrap();
                            let body = format!(
                                r#"{{"access_token":"access-{issued}","expires_in":{},"token_type":"Bearer"}}"#,
                                lifetime.load(Ordering::SeqCst)
                            );
                            let mut response = Response::builder().status(status);
                            if flavored {
                                response = response.header("metadata-flavor", "Google");
                            }
                            async move {
                                Ok::<_, std::convert::Infallible>(
                                    response.body(Full::new(Bytes::from(body))).unwrap(),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            address,
            requests,
            expires_in,
            answer,
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Answers every token request with `status`, and without the
    /// `Metadata-Flavor` header when `flavored` is false.
    pub fn answer(&self, status: StatusCode, flavored: bool) {
        *self.answer.lock().unwrap() = (status, flavored);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Metadata {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
pub async fn send(socket: &mut Socket, message: ClientMessage) {
    socket
        .send(Message::Text(message.encode().unwrap().into()))
        .await
        .unwrap();
}
pub async fn text(socket: &mut Socket, text: &str) {
    socket
        .send(Message::Text(text.to_owned().into()))
        .await
        .unwrap();
}
/// One push message as the wire spells it, so a test can make a field wrong.
pub fn push_message(id: u32, token: &str, collapse: &str, payload: &str) -> String {
    format!(
        r#"{{"type":"push_request","id":{id},"environment":"sandbox","token":"{token}","collapse":"{collapse}","payload":"{payload}"}}"#
    )
}
/// One FCM message as the wire spells it, so a test can make a field wrong.
pub fn fcm_message(id: u32, token: &str, payload: &str) -> String {
    format!(r#"{{"type":"fcm_request","id":{id},"token":"{token}","payload":"{payload}"}}"#)
}
/// Sends one push message and reads the result that answers it.
pub async fn answer_push(socket: &mut Socket, message: &str, id: u32) -> PushOutcome {
    text(socket, message).await;
    let ServerMessage::PushResult(result) = control(socket).await else {
        panic!("expected push_result")
    };
    assert_eq!(result.id, id);
    result.result
}
/// A ciphertext-sized payload, base64url like the real one.
pub fn payload() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    URL_SAFE_NO_PAD.encode(vec![7u8; 103 + 64])
}
pub async fn next(socket: &mut Socket) -> Message {
    timeout(Duration::from_secs(15), socket.next())
        .await
        .expect("server message deadline")
        .expect("socket ended")
        .expect("websocket failed")
}
pub async fn control(socket: &mut Socket) -> ServerMessage {
    let message = next(socket).await;
    let Message::Text(text) = message else {
        panic!("expected control message, got {message:?}")
    };
    ServerMessage::decode(&text).unwrap()
}
pub async fn binary(socket: &mut Socket) -> Vec<u8> {
    let message = next(socket).await;
    let Message::Binary(bytes) = message else {
        panic!("expected opaque binary, got {message:?}")
    };
    bytes.to_vec()
}
pub async fn closed(socket: &mut Socket, code: u16) {
    let message = next(socket).await;
    let Message::Close(Some(frame)) = message else {
        panic!("expected close, got {message:?}")
    };
    assert_eq!(u16::from(frame.code), code);
}
pub async fn refused(socket: &mut Socket, reason: hide_relay_protocol::admission::Refusal) {
    assert_eq!(control(socket).await, ServerMessage::Refused { reason });
}
pub async fn register(socket: &mut Socket, key: &DeviceKey) -> DeviceHello {
    register_answering(socket, key, false).await
}

/// Registers a device on a relay that forwards push requests.
pub async fn register_push(socket: &mut Socket, key: &DeviceKey) -> DeviceHello {
    register_answering(socket, key, true).await
}

/// Registers a device that named `fcm` on its upgrade, and checks what the
/// relay says it forwards.
pub async fn register_fcm(
    socket: &mut Socket,
    key: &DeviceKey,
    push: bool,
    fcm: bool,
) -> DeviceHello {
    let (hello, registered) = admit(socket, key).await;
    assert_eq!(registered, ServerMessage::Registered { push, fcm });
    hello
}

/// The device's hello and proof, and the `registered` that answers them.
pub async fn admit(socket: &mut Socket, key: &DeviceKey) -> (DeviceHello, ServerMessage) {
    let hello = DeviceHello::new(&key.public_key(), 7).unwrap();
    send(
        socket,
        ClientMessage::Device {
            hello: hello.clone(),
        },
    )
    .await;
    let ServerMessage::Challenge { challenge } = control(socket).await else {
        panic!("challenge")
    };
    send(
        socket,
        ClientMessage::Proof {
            signature: key.sign(&hello, &challenge).unwrap(),
        },
    )
    .await;
    (hello, control(socket).await)
}

async fn register_answering(socket: &mut Socket, key: &DeviceKey, push: bool) -> DeviceHello {
    let (hello, registered) = admit(socket, key).await;
    // A device that did not name `fcm` never reads it.
    assert_eq!(
        registered,
        ServerMessage::Registered { push, fcm: false },
        "the relay must say whether it pushes"
    );
    hello
}

/// The `.p8` a test relay signs with: a P-256 key made here, so no Apple
/// material is ever in the tree. The public key is kept so a test can verify
/// what the relay sent.
pub struct TestKey {
    pkcs8: Vec<u8>,
    public: Vec<u8>,
}

impl TestKey {
    /// The key as Apple ships a `.p8`: PEM around the PKCS#8 DER. Only the
    /// key-file test reads it, and that test needs Unix file modes.
    #[cfg(unix)]
    pub fn pem(&self) -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let base64 = STANDARD.encode(&self.pkcs8);
        let lines: Vec<&str> = base64
            .as_bytes()
            .chunks(64)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect();
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            lines.join("\n")
        )
    }
    pub fn new() -> Self {
        use ring::signature::KeyPair;
        let pair = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &ring::rand::SystemRandom::new(),
        )
        .unwrap();
        let key = ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            pair.as_ref(),
            &ring::rand::SystemRandom::new(),
        )
        .unwrap();
        let public = key.public_key().as_ref().to_vec();
        Self {
            pkcs8: pair.as_ref().to_vec(),
            public,
        }
    }

    /// What a relay reads from the `.p8` on disk.
    pub fn provider(&self) -> ProviderKey {
        ProviderKey::from_pkcs8(&self.pkcs8, "KEYID1234", "TEAMID5678", "me.grab.hide.ios").unwrap()
    }

    /// The provider token in an `authorization` header, checked as Apple
    /// checks it: the key id, the team, and the ES256 signature.
    pub fn verify(&self, authorization: &str) -> (serde_json::Value, serde_json::Value) {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let token = authorization
            .strip_prefix("bearer ")
            .expect("a bearer provider token");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "a JWT has three parts");
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "KEYID1234");
        assert_eq!(claims["iss"], "TEAMID5678");
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_FIXED,
            &self.public,
        )
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
        )
        .expect("the provider token verifies");
        (header, claims)
    }
}
pub async fn join(socket: &mut Socket, room: &str, device: &mut Socket) -> u32 {
    send(
        socket,
        ClientMessage::Phone {
            version: 1,
            room: room.to_owned(),
        },
    )
    .await;
    let ServerMessage::Joined {
        connection,
        phone_protocol,
    } = control(socket).await
    else {
        panic!("joined")
    };
    assert_eq!(phone_protocol, 7);
    assert_eq!(
        control(device).await,
        ServerMessage::PeerJoined { connection }
    );
    connection
}
pub async fn noise(phone: &mut Socket, device: &mut Socket, connection: u32) -> (Session, Session) {
    let key = NoiseKeypair::generate().unwrap();
    let mut initiator = Session::initiator(&key.public_key(), connection).unwrap();
    let mut responder = Session::responder(&key, connection).unwrap();
    let first = initiator.write_handshake().unwrap();
    phone
        .send(Message::Binary(first.clone().into()))
        .await
        .unwrap();
    let forwarded = binary(device).await;
    assert_eq!(forwarded, first);
    responder.read_handshake(&forwarded).unwrap();
    let reply = responder.write_handshake().unwrap();
    device
        .send(Message::Binary(reply.clone().into()))
        .await
        .unwrap();
    let forwarded = binary(phone).await;
    assert_eq!(forwarded, reply);
    initiator.read_handshake(&forwarded).unwrap();
    (initiator, responder)
}
pub async fn exchange(
    sender: &mut Socket,
    receiver: &mut Socket,
    sender_noise: &mut Session,
    receiver_noise: &mut Session,
    plaintext: &[u8],
) {
    let frames = sender_noise.send_message(plaintext).unwrap();
    let mut output = None;
    for frame in frames {
        sender
            .send(Message::Binary(frame.clone().into()))
            .await
            .unwrap();
        let forwarded = binary(receiver).await;
        assert_eq!(forwarded, frame, "relay changed ciphertext");
        output = receiver_noise.recv_message(&forwarded, 0).unwrap();
    }
    assert_eq!(output.unwrap(), plaintext);
}
