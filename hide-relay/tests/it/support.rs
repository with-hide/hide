use futures_util::{SinkExt, StreamExt};
use hide_relay::{
    Relay,
    event::Event,
    limits::{Limits, SHUTDOWN_TIMEOUT},
};
use hide_relay_protocol::{
    admission::{ClientMessage, DeviceHello, DeviceKey, ServerMessage},
    noise::{NoiseKeypair, Session},
};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
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
        let listener = TcpListener::bind(address).await.unwrap();
        let address = listener.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let clock = Arc::new(AtomicU64::new(0));
        let time = clock.clone();
        let relay = Relay::with_clock(
            limits,
            trusted.then_some(address.ip()),
            move |event| {
                let mut log = log.lock().unwrap();
                assert!(log.len() < 20_000, "bounded fixture event capture");
                log.push(event);
            },
            move || Duration::from_millis(time.load(Ordering::SeqCst)),
        )
        .unwrap();
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(relay.serve(listener, async {
            let _ = stopped.await;
        }));
        Self {
            address,
            events,
            clock,
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
pub async fn send(socket: &mut Socket, message: ClientMessage) {
    socket
        .send(Message::Text(message.encode().unwrap().into()))
        .await
        .unwrap();
}
pub async fn next(socket: &mut Socket) -> Message {
    timeout(Duration::from_secs(15), socket.next())
        .await
        .expect("server message deadline")
        .expect("socket ended")
        .expect("websocket failed")
}
pub async fn control(socket: &mut Socket) -> ServerMessage {
    let Message::Text(text) = next(socket).await else {
        panic!("expected control message")
    };
    ServerMessage::decode(&text).unwrap()
}
pub async fn binary(socket: &mut Socket) -> Vec<u8> {
    let Message::Binary(bytes) = next(socket).await else {
        panic!("expected opaque binary")
    };
    bytes.to_vec()
}
pub async fn closed(socket: &mut Socket, code: u16) {
    let Message::Close(Some(frame)) = next(socket).await else {
        panic!("expected close")
    };
    assert_eq!(u16::from(frame.code), code);
}
pub async fn refused(socket: &mut Socket, reason: hide_relay_protocol::admission::Refusal) {
    assert_eq!(control(socket).await, ServerMessage::Refused { reason });
}
pub async fn register(socket: &mut Socket, key: &DeviceKey) -> DeviceHello {
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
    assert_eq!(control(socket).await, ServerMessage::Registered);
    hello
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
