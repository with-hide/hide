//! The CDP relay behind `hide browser`: a pane's CLI, local or on a connected
//! device (through the reverse Workspace forward), reaches the desktop's
//! scoped gateway only through a Hide daemon, so the gateway's loopback
//! address and capability URL never leave that daemon's machine. The core
//! relays to its own window's gateway, or through a node's link to the
//! browser relay of the node whose window shows the page; a node's daemon
//! relays a caller on its own machine to its own window, so that CDP never
//! crosses the link (PRD core-host-node-remote-core B4, B13, B15). The relay
//! runs on its own task, outside the core's lock, and touches no core state.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message as GatewayMessage};

use crate::browser_control::Failure;
use crate::workspace_cli::Transport;

/// The gateway's own message cap (`browserCdp.ts`, `MAX_MESSAGE_BYTES`): a
/// larger frame could not be legitimate in either direction.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// A command polls or answers well within this; a silent pair is abandoned.
const IDLE: Duration = Duration::from_secs(60);
/// No command runs this long (`wait` is at most a minute), so a relay that
/// does is released whatever it carries.
const LIFETIME: Duration = Duration::from_secs(300);
/// A peer that does not take a frame within this has stopped reading.
const SEND: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Close codes the CLI reads as a reason (RFC 6455 4000-4999 are private).
pub const CLOSE_MESSAGE_LIMIT: u16 = 4009;
pub const CLOSE_IDLE: u16 = 4008;
pub const CLOSE_GATEWAY_LOST: u16 = 4011;

/// The gateway side of a relay: the gateway itself, or the browser relay
/// of a node's daemon through the node's link.
pub(crate) type Gateway = tokio_tungstenite::WebSocketStream<Pin<Box<dyn Transport>>>;

/// Where a relay's CDP runs, as its diagnostic names it.
#[derive(Clone, Copy, Debug)]
pub enum Way {
    /// The core to its own window's gateway.
    Core,
    /// The core through a node's link to that node's browser relay.
    Link,
    /// A node's daemon to its own window's gateway, for a caller on its
    /// machine.
    Node,
}

impl Way {
    fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Link => "link",
            Self::Node => "node",
        }
    }
}

fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
}

/// Opens the gateway side before the CLI is told the relay is ready, so a
/// refused upgrade is an answer the caller can act on, not a dropped socket.
pub(crate) async fn connect(browser_ws_url: &str) -> Result<Gateway, Failure> {
    let address = browser_ws_url
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|uri| Some((uri.host()?.to_owned(), uri.port_u16()?)))
        .ok_or_else(unavailable)?;
    let opened = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let stream = tokio::net::TcpStream::connect(address)
            .await
            .map_err(|_| unavailable())?;
        handshake(browser_ws_url, Box::pin(stream)).await
    })
    .await;
    opened.unwrap_or_else(|_| Err(unavailable()))
}

/// The same over `transport`, which already reaches the URL's host: a
/// stream through a node's link to the node's browser relay.
pub(crate) async fn connect_over(
    url: &str,
    transport: Pin<Box<dyn Transport>>,
) -> Result<Gateway, Failure> {
    tokio::time::timeout(CONNECT_TIMEOUT, handshake(url, transport))
        .await
        .unwrap_or_else(|_| Err(unavailable()))
}

async fn handshake(url: &str, transport: Pin<Box<dyn Transport>>) -> Result<Gateway, Failure> {
    let request = url.into_client_request().map_err(|_| unavailable())?;
    match tokio_tungstenite::client_async_with_config(request, transport, Some(config())).await {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response)) => Err(refused(response.status().as_u16())),
        Err(_) => Err(unavailable()),
    }
}

/// Opens the relay a linked node handed out (`relay_url`) through a stream
/// of that node's link, for a caller on another machine than the node's
/// window. The stream's bytes cross the link; at the link's cap of relay
/// streams the open is refused with its reason.
pub(crate) async fn connect_through(
    link: hide_node::ssh::RemoteHost,
    relay_url: &str,
) -> Result<Gateway, Failure> {
    let opened = tokio::task::spawn_blocking(move || link.browser_relay_stream())
        .await
        .map_err(|_| unavailable())?;
    let stream = opened.map_err(|error| match error {
        // This side's cap, or the node's while a stream this side let go is
        // still closing there.
        hide_node::ssh::OpenError::Cap(_) => refused(503),
        hide_node::ssh::OpenError::Refused(hide_node_link::LinkError::Refused(refusal))
            if refusal.code == hide_node_link::ErrorCode::Busy =>
        {
            refused(503)
        }
        _ => unavailable(),
    })?;
    connect_over(
        relay_url,
        Box::pin(bridge(stream).map_err(|_| unavailable())?),
    )
    .await
}

/// The link stream as an async stream: its reads and writes block on the
/// link, so a thread each moves its bytes through an in-memory pipe, and
/// either side ending ends the other.
pub(crate) fn bridge(
    stream: hide_node::ssh::LinkStream,
) -> std::io::Result<tokio::io::DuplexStream> {
    use std::io::{Read, Write};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (ours, theirs) = tokio::io::duplex(hide_node_link::panes::MAX_CHUNK);
    let (mut outgoing, mut incoming) = tokio::io::split(theirs);
    let runtime = tokio::runtime::Handle::current();
    let reading = runtime.clone();
    let mut writer = stream.writer();
    let closer = stream.closer();
    let unstarted = stream.closer();
    let mut reader = stream;
    std::thread::Builder::new()
        .name("browser-link-write".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; hide_node_link::panes::MAX_CHUNK];
            while let Ok(read) = runtime.block_on(outgoing.read(&mut buffer)) {
                if read == 0 || writer.write_all(&buffer[..read]).is_err() {
                    break;
                }
            }
            closer.close();
        })?;
    // The writer runs; a reader that cannot start ends the stream, which
    // ends the writer with it.
    let started = std::thread::Builder::new()
        .name("browser-link-read".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; hide_node_link::panes::MAX_CHUNK];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0
                    || reading
                        .block_on(incoming.write_all(&buffer[..read]))
                        .is_err()
                {
                    break;
                }
            }
            let _ = reading.block_on(incoming.shutdown());
        });
    if let Err(error) = started {
        unstarted.close();
        return Err(error);
    }
    Ok(ours)
}

/// What a refused upgrade means to the caller: a gateway or a node relay
/// at its cap, or a capability that is gone.
pub(crate) fn refused(status: u16) -> Failure {
    match status {
        429 => (
            "browser_control_busy",
            "Close another CDP client of this desktop window and retry",
        ),
        503 => (
            "browser_relay_limit",
            "Wait for another hide browser command to finish and retry",
        ),
        _ => unavailable(),
    }
}

fn unavailable() -> Failure {
    (
        "browser_control_unavailable",
        "Reconnect the Hide desktop app and retry",
    )
}

/// Moves text frames both ways until either side closes, a frame crosses the
/// cap, no CDP text moves for the idle bound, the relay reaches its
/// lifetime, or `ended` resolves (the link a node's relay was handed out
/// over is gone), which ends it as a lost gateway; a peer that stops reading
/// is dropped after a bounded send. Every exit closes both sides, and logs
/// the bytes it carried and why it ended.
pub(crate) async fn pump(
    client: &mut WebSocket,
    mut gateway: Gateway,
    display_id: &str,
    way: Way,
    ended: impl Future<Output = ()>,
) {
    let end = tokio::time::Instant::now() + LIFETIME;
    let mut deadline = tokio::time::Instant::now() + IDLE;
    let mut bytes = 0_usize;
    let mut ended = std::pin::pin!(ended);
    let (code, reason) = loop {
        tokio::select! {
            incoming = client.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if text.len() > MAX_MESSAGE_BYTES {
                        break (CLOSE_MESSAGE_LIMIT, "browser_relay_message_limit");
                    }
                    bytes += text.len();
                    let sent = tokio::time::timeout(SEND, gateway.send(GatewayMessage::Text(text.as_str().into()))).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        break (CLOSE_GATEWAY_LOST, "browser_relay_gateway_lost");
                    }
                }
                Some(Ok(Message::Binary(_))) => break (CLOSE_MESSAGE_LIMIT, "browser_relay_text_only"),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break (1000, "client_closed"),
                // Pings keep nothing alive: only CDP traffic resets the idle bound.
                Some(Ok(_)) => continue,
            },
            outgoing = gateway.next() => match outgoing {
                Some(Ok(GatewayMessage::Text(text))) => {
                    bytes += text.len();
                    let sent = tokio::time::timeout(SEND, client.send(Message::Text(text.as_str().into()))).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        break (1000, "client_closed");
                    }
                }
                Some(Ok(GatewayMessage::Close(frame))) => {
                    // The gateway's own reason (a closed display, a resource
                    // limit) reaches the CLI unchanged.
                    let (code, text) = frame.map_or((1000, String::new()), |frame| (u16::from(frame.code), frame.reason.to_string()));
                    let _ = tokio::time::timeout(SEND, client.send(Message::Close(Some(CloseFrame { code, reason: text.into() })))).await;
                    log(display_id, way, "gateway_closed", code, bytes);
                    return;
                }
                Some(Err(tungstenite::Error::Capacity(_))) => break (CLOSE_MESSAGE_LIMIT, "browser_relay_message_limit"),
                Some(Ok(_)) => continue,
                None | Some(Err(_)) => break (CLOSE_GATEWAY_LOST, "browser_relay_gateway_lost"),
            },
            () = &mut ended => break (CLOSE_GATEWAY_LOST, "browser_relay_gateway_lost"),
            () = tokio::time::sleep_until(deadline.min(end)) => break (CLOSE_IDLE, if deadline < end { "browser_relay_idle" } else { "browser_relay_lifetime" }),
        }
        deadline = tokio::time::Instant::now() + IDLE;
    };
    let _ = tokio::time::timeout(
        SEND,
        gateway.close(Some(tungstenite::protocol::CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        })),
    )
    .await;
    if code != 1000 {
        let _ = tokio::time::timeout(
            SEND,
            client.send(Message::Close(Some(CloseFrame {
                code,
                reason: reason.into(),
            }))),
        )
        .await;
    }
    log(display_id, way, reason, code, bytes);
}

/// One line per relay: where its CDP ran, how many bytes it carried, and
/// why it ended. A relay is one `hide browser` command, so this is no
/// high-frequency path.
fn log(display_id: &str, way: Way, reason: &str, code: u16, bytes: usize) {
    herdr_core::diagnostic!(json!({
        "component": "browser_relay",
        "kind": "relay.ended",
        "display_id": display_id,
        "way": way.name(),
        "reason": reason,
        "code": code,
        "bytes": bytes,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::ws::WebSocketUpgrade;
    use axum::routing::get;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    /// A gateway that echoes each short text frame, as a CDP endpoint
    /// answers, and drops anything else, so only the relay's own checks can
    /// close the connection.
    async fn echo_gateway() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if let GatewayMessage::Text(text) = message
                    && text.len() < 64
                    && socket.send(GatewayMessage::Text(text)).await.is_err()
                {
                    break;
                }
            }
        });
        format!("ws://{address}/")
    }

    /// A daemon route that relays one client to the gateway.
    async fn relay_to(gateway: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    let gateway = connect(&gateway).await.unwrap();
                    pump(
                        &mut socket,
                        gateway,
                        "d1",
                        Way::Core,
                        std::future::pending(),
                    )
                    .await;
                })
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("ws://{address}/")
    }

    async fn close_code(message: GatewayMessage) -> u16 {
        let relay = relay_to(echo_gateway().await).await;
        let (mut client, _) = tokio_tungstenite::connect_async(relay.as_str())
            .await
            .unwrap();
        client
            .send(GatewayMessage::Text(r#"{"id":1}"#.into()))
            .await
            .unwrap();
        assert_eq!(
            client.next().await.unwrap().unwrap(),
            GatewayMessage::Text(r#"{"id":1}"#.into()),
            "a CDP frame crosses the relay both ways"
        );
        client.send(message).await.unwrap();
        loop {
            match client.next().await {
                Some(Ok(GatewayMessage::Close(Some(frame)))) => return u16::from(frame.code),
                Some(Ok(_)) => continue,
                other => panic!("the relay ended without a close frame: {other:?}"),
            }
        }
    }

    /// The link a node's relay was handed out over ends mid-command: the
    /// caller reads the lost gateway's close code, as when a gateway goes.
    #[tokio::test]
    async fn a_relay_whose_link_ends_closes_as_a_lost_gateway() {
        let gateway = echo_gateway().await;
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        let ended = std::sync::Arc::new(tokio::sync::Mutex::new(Some(ended)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    let gateway = connect(&gateway).await.unwrap();
                    let ended = ended.lock().await.take().unwrap();
                    pump(&mut socket, gateway, "d1", Way::Node, async {
                        let _ = ended.await;
                    })
                    .await;
                })
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        client
            .send(GatewayMessage::Text(r#"{"id":1}"#.into()))
            .await
            .unwrap();
        assert!(matches!(
            client.next().await,
            Some(Ok(GatewayMessage::Text(_)))
        ));
        end.send(()).unwrap();
        loop {
            match client.next().await {
                Some(Ok(GatewayMessage::Close(Some(frame)))) => {
                    assert_eq!(u16::from(frame.code), CLOSE_GATEWAY_LOST);
                    assert_eq!(frame.reason, "browser_relay_gateway_lost");
                    return;
                }
                Some(Ok(_)) => continue,
                other => panic!("the relay ended without a close frame: {other:?}"),
            }
        }
    }

    /// A refused upgrade tells the caller which cap it met: the gateway's
    /// own clients, or the relays a node runs at once.
    #[test]
    fn a_refused_dial_names_the_cap_it_met() {
        assert_eq!(refused(429).0, "browser_control_busy");
        assert_eq!(refused(503).0, "browser_relay_limit");
        assert_eq!(refused(404).0, "browser_control_unavailable");
        assert_eq!(refused(403).0, "browser_control_unavailable");
    }

    #[tokio::test]
    async fn a_frame_over_the_cap_closes_the_relay_with_its_reason() {
        let oversized = "x".repeat(MAX_MESSAGE_BYTES + 1);
        assert_eq!(
            close_code(GatewayMessage::Text(oversized.into())).await,
            CLOSE_MESSAGE_LIMIT
        );
    }

    #[tokio::test]
    async fn a_binary_frame_closes_the_relay_with_its_reason() {
        assert_eq!(
            close_code(GatewayMessage::Binary(vec![1, 2, 3].into())).await,
            CLOSE_MESSAGE_LIMIT
        );
        assert_ne!(CLOSE_MESSAGE_LIMIT, u16::from(CloseCode::Normal));
    }
}
