//! One outbound relay socket, owned by Mobile. The relay routes opaque frames;
//! phone admission and every encrypted application frame stay in phone.rs.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::{
    Error,
    admission::{ClientMessage, DeviceHello, DeviceKey, Refusal, ServerMessage},
    frame::{Frame, MAX_FRAME},
    noise::{NoiseKeypair, Session},
    qr::PairingQr,
};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use super::{
    Mobile, phone,
    store::{Method, RelayKeys},
};

const DEADLINE: Duration = Duration::from_secs(10);
const MAX_PEERS: usize = 8;
const INBOUND_QUEUE: usize = 8;
const OUTBOUND_QUEUE: usize = 32;
/// The phone application contract, independent of relay wire version.
pub const PHONE_PROTOCOL: u32 = 1;

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
    herdr_core::diagnostic!(json!({
        "component":"mobile_relay", "kind":"relay.closed", "connection":connection, "reason":reason,
    }));
}

fn desired(mobile: &Mobile) -> bool {
    let inner = mobile.lock();
    inner.settings.enabled
        && inner.settings.method == Method::Relay
        && inner.settings.serve.is_none()
        && *mobile.config.open.borrow()
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
        if !desired(&mobile) {
            tokio::select! {
                _ = changes.changed() => {},
                _ = open.changed(), if open.has_changed().is_ok() => {},
                _ = stopping.changed() => break,
            }
            attempt = 0;
            continue;
        }
        let keys = match mobile.relay_keys() {
            Ok(keys) => Arc::new(keys),
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
                mobile.relay_state(State::KeyUnavailable);
                break;
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
        if !desired(mobile) {
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
    tokio::time::timeout(DEADLINE, async {
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
            ServerMessage::Registered => Ok(()),
            ServerMessage::Refused { reason } => Err(LinkError::Refused(reason)),
            _ => Err(LinkError::Closed),
        }
    })
    .await
    .map_err(|_| LinkError::Closed)??;
    mobile.relay_state(State::Connected);
    let (outgoing, mut output) = mpsc::channel::<Output>(OUTBOUND_QUEUE);
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
                            if peers.len() >= MAX_PEERS {
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
    let result = async {
        let mut session = Session::responder(&keys.noise, connection)?;
        tokio::time::timeout(DEADLINE, async {
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
        let first = tokio::time::timeout(DEADLINE, socket.recv())
            .await
            .map_err(|_| Error::Timeout)?;
        let Some(Ok(axum::extract::ws::Message::Text(text))) = first else {
            return Err(Error::InvalidMessage);
        };
        if text.len() > 16 * 1024 {
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
