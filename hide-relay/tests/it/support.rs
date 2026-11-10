use futures_util::{SinkExt, StreamExt};
use hide_relay::{
    Relay,
    apns::{Apns, Clock, ProviderKey},
    event::Event,
    limits::{Limits, PushLimits, SHUTDOWN_TIMEOUT},
};
use hide_relay_protocol::{
    admission::{ClientMessage, DeviceHello, DeviceKey, PushOutcome, ServerMessage},
    noise::{NoiseKeypair, Session},
};
use http_body_util::Full;
use hyper::{Response, StatusCode, body::Bytes, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{
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
        Self::build(limits, PushLimits::default(), address, trusted, None).await
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
        )
        .await
    }
    async fn build(
        limits: Limits,
        push: PushLimits,
        address: SocketAddr,
        trusted: bool,
        apns: Option<Box<dyn FnOnce(Clock) -> Apns>>,
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
        let apns = {
            let time = unix.clone();
            apns.map(|make| {
                make(Arc::new(move || {
                    Duration::from_millis(time.load(Ordering::SeqCst))
                }))
            })
        };
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
        let mut request = format!("ws://{}/ws", self.address)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("x-real-ip", ip.parse().unwrap());
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

/// One request a stand-in Apple received, as the relay sent it.
#[derive(Clone, Debug)]
pub struct AppleRequest {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl AppleRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A local HTTP/2 (h2c) stand-in for Apple's push service. Every request is
/// recorded, and answered with whatever the test set.
pub struct Apple {
    pub address: SocketAddr,
    pub requests: Arc<Mutex<Vec<AppleRequest>>>,
    answer: Arc<Mutex<(StatusCode, String)>>,
    /// The answer to the next request only, so a test can make Apple refuse
    /// once and then accept.
    next: Arc<Mutex<Option<(StatusCode, String)>>>,
    /// How many TCP connections this Apple accepted, so a test can prove the
    /// relay keeps one and does not open one per request.
    connections: Arc<AtomicU64>,
    /// Open by default; while closed, each request is recorded and its
    /// answer waits, so a test can hold the relay's sends in flight.
    gate: Arc<tokio::sync::watch::Sender<bool>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Apple {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(Mutex::new((StatusCode::OK, String::new())));
        let next = Arc::new(Mutex::new(None));
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
                                let path = request.uri().path().to_owned();
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
                                let body =
                                    match http_body_util::BodyExt::collect(request.into_body())
                                        .await
                                    {
                                        Ok(collected) => collected.to_bytes().to_vec(),
                                        Err(_) => Vec::new(),
                                    };
                                recorded.lock().unwrap().push(AppleRequest {
                                    path,
                                    headers,
                                    body,
                                });
                                let _ = gate.wait_for(|open| *open).await;
                                let (status, reason) = once
                                    .lock()
                                    .unwrap()
                                    .take()
                                    .unwrap_or_else(|| answering.lock().unwrap().clone());
                                let body = if reason.is_empty() {
                                    String::new()
                                } else {
                                    format!(r#"{{"reason":"{reason}"}}"#)
                                };
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .status(status)
                                        .header("apns-id", "00000000-0000-0000-0000-000000000000")
                                        .body(Full::new(Bytes::from(body)))
                                        .unwrap(),
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

    /// The answer every request gets from now on.
    pub fn answer(&self, status: StatusCode, reason: &str) {
        *self.answer.lock().unwrap() = (status, reason.to_owned());
    }

    /// The answer the next request gets, before the standing one.
    pub fn answer_next(&self, status: StatusCode, reason: &str) {
        *self.next.lock().unwrap() = Some((status, reason.to_owned()));
    }

    /// Holds every answer until `release`.
    pub fn hold(&self) {
        self.gate.send_replace(false);
    }
    pub fn release(&self) {
        self.gate.send_replace(true);
    }

    pub fn requests(&self) -> Vec<AppleRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn connections(&self) -> u64 {
        self.connections.load(Ordering::SeqCst)
    }

    /// Waits for the `count`-th request, so a test reads what Apple saw
    /// without polling a fixed number of times.
    pub async fn wait_for(&self, count: usize) -> Vec<AppleRequest> {
        for _ in 0..3_000 {
            let requests = self.requests();
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "apple received {} of {count} requests",
            self.requests().len()
        );
    }
}

impl Drop for Apple {
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

async fn register_answering(socket: &mut Socket, key: &DeviceKey, push: bool) -> DeviceHello {
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
    let ServerMessage::Registered { push: advertised } = control(socket).await else {
        panic!("registered")
    };
    assert_eq!(advertised, push, "the relay must say whether it pushes");
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
