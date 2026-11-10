//! The caller boundary: a real hided dials a private WebSocket listener, which
//! speaks shared admission/routing messages. The phone executes the real Noise
//! initiator. This fixture stores no room state or server policy of its own.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{SinkExt, StreamExt};
use hide_relay_protocol::{
    admission::{ClientMessage, PendingAdmission, Refusal, ServerMessage},
    noise::Session,
    qr::PairingQr,
};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};

type Device = WebSocketStream<TcpStream>;
type Shell = WebSocketStream<MaybeTlsStream<TcpStream>>;
const GUARD: Duration = Duration::from_secs(20);

fn env(at: &Path, port: u16) -> hided::env::Env {
    let mut env = hided::env::load_from(|key| match key {
        "HOME" | "USERPROFILE" => Some(at.to_string_lossy().into_owned()),
        "HIDE_STATE_DIR" => Some(at.join("state").to_string_lossy().into_owned()),
        "HIDE_RELAY_URL" => Some(format!("ws://127.0.0.1:{port}/ws")),
        "HIDE_TAILSCALE_BIN" => Some(at.join("no-tailscale").to_string_lossy().into_owned()),
        _ => None,
    })
    .unwrap();
    env.bind = "127.0.0.1:0".parse().unwrap();
    env.keep_alive = true;
    env
}

async fn next<S>(socket: &mut WebSocketStream<S>) -> Option<Message>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(GUARD, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Ping(bytes))) => {
                    socket.send(Message::Pong(bytes)).await.unwrap();
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(message)) => return Some(message),
                _ => return None,
            }
        }
    })
    .await
    .expect("socket progress")
}
async fn event(shell: &mut Shell, kind: &str, payload: Value) {
    shell
        .send(Message::Text(
            json!({"schema_version":2,"kind":kind,"payload":payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}
async fn state(shell: &mut Shell, accept: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(GUARD, async {
        loop {
            if let Some(Message::Text(text)) = next(shell).await {
                let frame: Value = serde_json::from_str(&text).unwrap();
                if frame["type"] == "mobile" && accept(&frame["payload"]) {
                    return frame["payload"].clone();
                }
            } else {
                panic!("renderer closed");
            }
        }
    })
    .await
    .expect("mobile state")
}
async fn shell(running: &hided::RunningDaemon) -> Shell {
    let mut request = format!("ws://127.0.0.1:{}/ws", running.port)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "origin",
        format!("http://127.0.0.1:{}", running.port)
            .parse()
            .unwrap(),
    );
    let mut shell = tokio_tungstenite::connect_async(request).await.unwrap().0;
    shell
        .send(Message::Text(
            json!({"token":running.token,"schema_version":2,"client_kind":"desktop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    shell
}
async fn control(device: &mut Device, message: ServerMessage) {
    device
        .send(Message::Text(message.encode().unwrap().into()))
        .await
        .unwrap();
}
async fn accept(listener: &TcpListener) -> Device {
    accept_answering(
        listener,
        ServerMessage::Registered { push: false }.encode().unwrap(),
    )
    .await
}
/// Admits hided's device socket and confirms it with `registered`, spelled as
/// the relay under test spells it: an older relay sends `{"type":"registered"}`.
async fn accept_answering(listener: &TcpListener, registered: String) -> Device {
    let (stream, _) = tokio::time::timeout(GUARD, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut device = tokio_tungstenite::accept_async(stream).await.unwrap();
    let Some(Message::Text(text)) = next(&mut device).await else {
        panic!("device hello");
    };
    let ClientMessage::Device { hello } = ClientMessage::decode(&text).unwrap() else {
        panic!("device hello");
    };
    let pending = PendingAdmission::new(hello, 0).unwrap();
    control(
        &mut device,
        ServerMessage::Challenge {
            challenge: pending.challenge().clone(),
        },
    )
    .await;
    let Some(Message::Text(text)) = next(&mut device).await else {
        panic!("device proof");
    };
    let ClientMessage::Proof { signature } = ClientMessage::decode(&text).unwrap() else {
        panic!("device proof");
    };
    pending.verify(&signature, 1).unwrap();
    device.send(Message::Text(registered.into())).await.unwrap();
    device
}
async fn phone(device: &mut Device, qr: &PairingQr, connection: u32, auth: Value) -> Session {
    let mut session = handshake(device, qr, connection).await;
    send(device, &mut session, auth).await;
    session
}
async fn handshake(device: &mut Device, qr: &PairingQr, connection: u32) -> Session {
    control(device, ServerMessage::PeerJoined { connection }).await;
    let public: [u8; 32] = URL_SAFE_NO_PAD
        .decode(&qr.noise_key)
        .unwrap()
        .try_into()
        .unwrap();
    let mut session = Session::initiator(&public, connection).unwrap();
    device
        .send(Message::Binary(session.write_handshake().unwrap().into()))
        .await
        .unwrap();
    let Some(Message::Binary(bytes)) = next(device).await else {
        panic!("Noise response");
    };
    session.read_handshake(&bytes).unwrap();
    session
}
async fn send(device: &mut Device, phone: &mut Session, message: Value) {
    for frame in phone.send_message(message.to_string().as_bytes()).unwrap() {
        device.send(Message::Binary(frame.into())).await.unwrap();
    }
}
async fn receive(device: &mut Device, phone: &mut Session, kind: &str) -> Value {
    loop {
        let Some(Message::Binary(bytes)) = next(device).await else {
            panic!("encrypted {kind}");
        };
        let received = phone
            .recv_message(&bytes, 0)
            .unwrap_or_else(|error| panic!("waiting for {kind}: {error:?}"));
        if let Some(plain) = received {
            let frame: Value = serde_json::from_slice(&plain).unwrap();
            if frame["type"] == kind {
                return frame;
            }
        }
    }
}
async fn closed(device: &mut Device, connection: u32) {
    loop {
        match next(device).await {
            Some(Message::Text(text))
                if ClientMessage::decode(&text).unwrap()
                    == (ClientMessage::Close { connection }) =>
            {
                return;
            }
            Some(Message::Binary(_)) => {}
            other => panic!("expected peer close, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn relay_pairs_encrypts_reconnects_scopes_revokes_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    let off = state(&mut shell, |_| true).await;
    assert_eq!(off["method"], "relay");
    assert_eq!(off["enabled"], false);
    event(&mut shell, "mobile_enable", json!({"enabled":true})).await;
    let mut device = accept(&listener).await;
    let connected = state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    assert!(connected["checklist"].is_null());
    event(&mut shell, "mobile_show_code", json!({})).await;
    let code = state(&mut shell, |s| s["qr"].is_string()).await;
    let qr = PairingQr::decode(code["qr"].as_str().unwrap()).unwrap();
    assert_eq!(code["url"], "https://app.withhide.dev/");
    assert!(hide_platform::fs::private::is_private(&dir.path().join("state/mobile.json")).unwrap());
    let mut first = phone(
        &mut device,
        &qr,
        1,
        json!({"client_kind":"phone","pair":qr.code,"name":"iPhone"}),
    )
    .await;
    let paired = receive(&mut device, &mut first, "paired").await;
    receive(&mut device, &mut first, "agents").await;
    let listed = state(&mut shell, |s| {
        s["phones"].as_array().is_some_and(|p| p.len() == 1)
    })
    .await;
    assert!(listed["qr"].is_null(), "single use code");
    // Retry must not cancel a healthy link; the new QR fences the renderer
    // command before the original encrypted session answers the next ping.
    event(&mut shell, "mobile_retry", json!({})).await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    state(&mut shell, |s| s["qr"].is_string()).await;
    send(&mut device, &mut first, json!({"type":"ping","id":1})).await;
    receive(&mut device, &mut first, "pong").await;
    // Replay is rejected by hided, without bringing the relay link down.
    let replay = first
        .send_message(br#"{"type":"ping","id":1}"#)
        .unwrap()
        .remove(0);
    device
        .send(Message::Binary(replay.clone().into()))
        .await
        .unwrap();
    receive(&mut device, &mut first, "pong").await;
    device.send(Message::Binary(replay.into())).await.unwrap();
    closed(&mut device, 1).await;
    let mut tampered = phone(
        &mut device,
        &qr,
        2,
        json!({"client_kind":"phone","token":paired["credential"]}),
    )
    .await;
    receive(&mut device, &mut tampered, "agents").await;
    let mut bytes = tampered
        .send_message(br#"{"type":"ping","id":2}"#)
        .unwrap()
        .remove(0);
    *bytes.last_mut().unwrap() ^= 1;
    device.send(Message::Binary(bytes.into())).await.unwrap();
    closed(&mut device, 2).await;
    let mut incomplete = phone(
        &mut device,
        &qr,
        3,
        json!({"client_kind":"phone","token":paired["credential"]}),
    )
    .await;
    receive(&mut device, &mut incomplete, "agents").await;
    let chunk = incomplete
        .send_message(&vec![b'x'; 128_000])
        .unwrap()
        .remove(0);
    device.send(Message::Binary(chunk.into())).await.unwrap();
    // No more bytes arrive: hided must poll the Noise reassembly deadline.
    closed(&mut device, 3).await;
    let phone_id = paired["phone_id"].as_str().unwrap();
    assert!(matches!(
        running.mobile.reserve_input(phone_id, "lost-answer"),
        hided::mobile::Reservation::Write
    ));
    running.mobile.settle_input(
        phone_id,
        "lost-answer",
        Some(hided::mobile::InputState::Written),
    );
    // Losing all relay state is sufficient: a new signed admission and fresh
    // Noise state restore the same phone credential and device identity.
    drop(device);
    state(&mut shell, |s| s["relay"]["state"] == "retrying").await;
    let mut device = accept(&listener).await;
    state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    let mut second = phone(
        &mut device,
        &qr,
        1,
        json!({"client_kind":"phone","token":paired["credential"]}),
    )
    .await;
    receive(&mut device, &mut second, "agents").await;
    // A connection's first list is followed by the missions and pull request frames, so they are read before the next message is judged.
    receive(&mut device, &mut second, "missions").await;
    receive(&mut device, &mut second, "prs").await;
    // Socket teardown must retain the already applied reply's request record.
    assert!(matches!(
        running.mobile.reserve_input(phone_id, "lost-answer"),
        hided::mobile::Reservation::Seen(hided::mobile::InputState::Written)
    ));
    event(&mut shell, "mobile_method", json!({"method":"tailscale"})).await;
    let direct = state(&mut shell, |s| s["method"] == "tailscale").await;
    assert_eq!(direct["phones"], json!([]));
    assert!(matches!(
        next(&mut device).await,
        None | Some(Message::Close(_))
    ));
    event(&mut shell, "mobile_method", json!({"method":"relay"})).await;
    let mut device = accept(&listener).await;
    let back = state(&mut shell, |s| {
        s["relay"]["state"] == "connected" && s["method"] == "relay"
    })
    .await;
    assert_eq!(back["phones"][0]["id"], paired["phone_id"]);
    let mut third = phone(
        &mut device,
        &qr,
        1,
        json!({"client_kind":"phone","token":paired["credential"]}),
    )
    .await;
    receive(&mut device, &mut third, "agents").await;
    event(
        &mut shell,
        "mobile_revoke",
        json!({"phone_id":paired["phone_id"]}),
    )
    .await;
    closed(&mut device, 1).await;
    let mut revoked = phone(
        &mut device,
        &qr,
        2,
        json!({"client_kind":"phone","token":paired["credential"]}),
    )
    .await;
    assert_eq!(
        receive(&mut device, &mut revoked, "refused").await["reason"],
        "revoked"
    );
    closed(&mut device, 2).await;
    event(&mut shell, "mobile_enable", json!({"enabled":false})).await;
    state(&mut shell, |s| s["enabled"] == false).await;
    assert!(matches!(
        next(&mut device).await,
        None | Some(Message::Close(_))
    ));
    // Explicit retry while disabled cannot open a link. The renderer answer
    // fences the command; the interval covers the first reconnect ceiling.
    event(&mut shell, "mobile_retry", json!({})).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), listener.accept())
            .await
            .is_err()
    );
    running.stop();
}

#[tokio::test]
async fn an_upgrade_keeps_a_direct_setup_and_a_new_setup_chooses_relay() {
    for (settings, expected) in [
        (json!({}), "relay"),
        (json!({"last_tailnet":"fixture.ts.net"}), "tailscale"),
        (
            json!({"serve":{"dns_name":"fixture.ts.net","port":1234,"added_at":1}}),
            "tailscale",
        ),
        (
            json!({"method":"relay","last_tailnet":"fixture.ts.net"}),
            "relay",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("state")).unwrap();
        std::fs::write(dir.path().join("state/mobile.json"), settings.to_string()).unwrap();
        let running = hided::start_daemon(env(dir.path(), 9)).await.unwrap();
        let mut shell = shell(&running).await;
        assert_eq!(state(&mut shell, |_| true).await["method"], expected);
        running.stop();
    }
}

#[tokio::test]
async fn a_missing_tailscale_cli_does_not_block_relay_and_keeps_the_cleanup_warning() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state")).unwrap();
    std::fs::write(
        dir.path().join("state/mobile.json"),
        json!({
            "enabled":true,"method":"relay",
            "serve":{"dns_name":"fixture.ts.net","port":1234,"added_at":1},
        })
        .to_string(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    let _device = accept(&listener).await;
    let connected = state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    assert_eq!(connected["failure"]["step"], "remove");
    event(&mut shell, "mobile_show_code", json!({})).await;
    assert!(state(&mut shell, |s| s["qr"].is_string()).await["qr"].is_string());
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("state/mobile.json")).unwrap())
            .unwrap();
    assert!(saved["serve"].is_null());
    assert_eq!(saved["last_tailnet"], "fixture.ts.net");
    running.stop();
}

#[tokio::test]
async fn switching_back_restores_direct_phones_without_a_live_tailscale_or_serve_entry() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    std::fs::create_dir(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("mobile.json"),
        json!({"enabled":false,"method":"relay","last_tailnet":"fixture.ts.net"}).to_string(),
    )
    .unwrap();
    let now = hided::mobile::now_ms();
    let record = |id: &str, origin: Value| {
        json!({
            "id":id,"name":"Phone","credential_sha256":"0".repeat(64),
            "paired_at_ms":now,"last_seen_ms":now,"origin":origin,
        })
    };
    std::fs::write(
        state_dir.join("phones.json"),
        json!({"phones":[record("direct", json!({"paired":"fixture.ts.net"})), record("relay", json!("relay"))]}).to_string(),
    )
    .unwrap();
    let running = hided::start_daemon(env(dir.path(), 9)).await.unwrap();
    let mut shell = shell(&running).await;
    let relay = state(&mut shell, |s| s["method"] == "relay").await;
    assert_eq!(relay["phones"].as_array().unwrap().len(), 1);
    assert_eq!(relay["phones"][0]["id"], "relay");
    for method in ["tailscale", "relay", "tailscale"] {
        event(&mut shell, "mobile_method", json!({"method":method})).await;
        let shown = state(&mut shell, |s| s["method"] == method).await;
        assert_eq!(shown["phones"].as_array().unwrap().len(), 1);
        assert_eq!(
            shown["phones"][0]["id"],
            if method == "relay" { "relay" } else { "direct" }
        );
        assert!(shown["qr"].is_null());
    }
    running.stop();
}

#[tokio::test]
async fn unauthenticated_peers_expire_without_using_paired_phone_slots() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    state(&mut shell, |_| true).await;
    event(&mut shell, "mobile_enable", json!({"enabled":true})).await;
    let mut device = accept(&listener).await;
    state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let code = state(&mut shell, |s| s["qr"].is_string()).await;
    let qr = PairingQr::decode(code["qr"].as_str().unwrap()).unwrap();
    // Two peers stop before Noise; two finish Noise but never send credentials.
    // These four connections must not consume the four paired-phone slots.
    for connection in [1, 2] {
        control(&mut device, ServerMessage::PeerJoined { connection }).await;
    }
    for connection in [3, 4] {
        let _ = handshake(&mut device, &qr, connection).await;
    }
    let mut paired = phone(
        &mut device,
        &qr,
        5,
        json!({"client_kind":"phone","pair":qr.code,"name":"Phone"}),
    )
    .await;
    receive(&mut device, &mut paired, "paired").await;
    receive(&mut device, &mut paired, "agents").await;
    let listed = state(&mut shell, |s| {
        s["phones"].as_array().is_some_and(|p| p.len() == 1)
    })
    .await;
    assert_eq!(listed["phones"].as_array().unwrap().len(), 1);
    // The wait observes the product's ten-second deadline, not a settling delay.
    let mut closed_peers = std::collections::BTreeSet::new();
    while closed_peers.len() < 4 {
        match next(&mut device).await {
            Some(Message::Text(text)) => {
                let ClientMessage::Close { connection } = ClientMessage::decode(&text).unwrap()
                else {
                    panic!("expected peer close");
                };
                assert!((1..=4).contains(&connection));
                assert!(closed_peers.insert(connection));
            }
            Some(Message::Binary(bytes)) => {
                // Consume status updates just as a real phone does, preserving
                // the authenticated receive sequence while its peers expire.
                paired.recv_message(&bytes, 0).unwrap();
            }
            other => panic!("expected peer close, got {other:?}"),
        }
    }
    send(&mut device, &mut paired, json!({"type":"ping","id":9})).await;
    assert_eq!(receive(&mut device, &mut paired, "pong").await["id"], 9);
    running.stop();
}

#[tokio::test]
async fn relay_reports_version_and_capacity_refusals_in_the_connection_line() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    state(&mut shell, |_| true).await;
    event(&mut shell, "mobile_enable", json!({"enabled":true})).await;
    let mut device = accept(&listener).await;
    control(
        &mut device,
        ServerMessage::Refused {
            reason: Refusal::UnsupportedVersion,
        },
    )
    .await;
    assert_eq!(
        state(&mut shell, |s| s["relay"]["state"] == "refused").await["relay"]["reason"],
        "unsupported_version"
    );
    event(&mut shell, "mobile_retry", json!({})).await;
    let mut device = accept(&listener).await;
    control(
        &mut device,
        ServerMessage::Refused {
            reason: Refusal::DailyLimit,
        },
    )
    .await;
    assert_eq!(
        state(&mut shell, |s| s["relay"]["reason"] == "daily_limit").await["relay"]["state"],
        "refused"
    );
    running.stop();
}

/// A phone's push key pair, as the iOS app makes one per computer: the
/// private half stays here to decrypt what Apple would carry.
struct AppKey {
    private: ring::agreement::EphemeralPrivateKey,
    public: Vec<u8>,
    auth: [u8; 16],
}
impl AppKey {
    fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let private =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let public = private.compute_public_key().unwrap().as_ref().to_vec();
        Self {
            private,
            public,
            auth: [7; 16],
        }
    }
    fn registration(&self, token: &str) -> Value {
        json!({
            "type": "apns_registration",
            "token": token,
            "environment": "sandbox",
            "key": {
                "p256dh": URL_SAFE_NO_PAD.encode(&self.public),
                "auth": URL_SAFE_NO_PAD.encode(self.auth),
            },
        })
    }
    /// RFC 8291 decryption of one aes128gcm body, as the phone's
    /// notification extension does it (`docs/RELAY_PROTOCOL.md`, The
    /// notification body).
    fn decrypt(self, body: &[u8]) -> Vec<u8> {
        let hmac = |key: &[u8], parts: &[&[u8]]| {
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
            let mut context = ring::hmac::Context::with_key(&key);
            for part in parts {
                context.update(part);
            }
            context.sign().as_ref().to_vec()
        };
        let (salt, rest) = body.split_at(16);
        assert_eq!(&rest[..4], 4096_u32.to_be_bytes(), "record size");
        assert_eq!(rest[4], 65, "the sender key is an uncompressed P-256 point");
        let sender = &rest[5..70];
        let ciphertext = &rest[70..];
        let peer = ring::agreement::UnparsedPublicKey::new(&ring::agreement::ECDH_P256, sender);
        let shared =
            ring::agreement::agree_ephemeral(self.private, &peer, |secret| secret.to_vec())
                .unwrap();
        let prk_key = hmac(&self.auth, &[&shared]);
        let ikm = hmac(&prk_key, &[b"WebPush: info\0", &self.public, sender, &[1]]);
        let prk = hmac(salt, &[&ikm]);
        let cek = hmac(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
        let nonce = hmac(&prk, &[b"Content-Encoding: nonce\0", &[1]]);
        let key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek[..16]).unwrap(),
        );
        let mut buffer = ciphertext.to_vec();
        let plain = key
            .open_in_place(
                ring::aead::Nonce::try_assume_unique_for_key(&nonce[..12]).unwrap(),
                ring::aead::Aad::empty(),
                &mut buffer,
            )
            .unwrap();
        // One record: the plaintext, then the last-record delimiter.
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }
}

/// The push request hided sends over its device socket, read by the relay.
/// Phone frames that arrive meanwhile still pass through the phone's
/// session, whose order every later frame depends on.
async fn push_request(
    device: &mut Device,
    phone: &mut Session,
) -> hide_relay_protocol::admission::PushRequest {
    loop {
        match next(device).await {
            Some(Message::Text(text)) => {
                let ClientMessage::Push(request) = ClientMessage::decode(&text).unwrap() else {
                    panic!("expected push_request, got {text}");
                };
                return request;
            }
            Some(Message::Binary(bytes)) => {
                phone.recv_message(&bytes, 0).unwrap();
            }
            other => panic!("expected push_request, got {other:?}"),
        }
    }
}

fn phones_file(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("state/phones.json")).unwrap()
}

const APP_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn an_app_phone_is_notified_through_the_relay_and_a_gone_token_is_dropped() {
    use hide_relay_protocol::admission::{PushEnvironment, PushOutcome, PushResult};
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    event(&mut shell, "mobile_enable", json!({"enabled":true})).await;
    let mut device = accept_answering(
        &listener,
        ServerMessage::Registered { push: true }.encode().unwrap(),
    )
    .await;
    state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let code = state(&mut shell, |s| s["qr"].is_string()).await;
    let qr = PairingQr::decode(code["qr"].as_str().unwrap()).unwrap();
    let mut session = phone(
        &mut device,
        &qr,
        1,
        json!({"client_kind":"phone","pair":qr.code,"name":"iPhone"}),
    )
    .await;
    let paired = receive(&mut device, &mut session, "paired").await;
    receive(&mut device, &mut session, "agents").await;
    let phone_id = paired["phone_id"].as_str().unwrap().to_owned();

    // A malformed registration is refused and stores nothing.
    let key = AppKey::new();
    let mut wrong = key.registration("../not-hex");
    send(&mut device, &mut session, wrong.take()).await;
    let refused = receive(&mut device, &mut session, "push_state").await;
    assert_eq!(refused["notifications"], "refused");
    send(&mut device, &mut session, key.registration(APP_TOKEN)).await;
    let stored = receive(&mut device, &mut session, "push_state").await;
    assert_eq!(stored["notifications"], "on");
    assert!(phones_file(dir.path()).contains(APP_TOKEN));
    let listed = state(&mut shell, |s| s["phones"][0]["notifications"] == "on").await;
    assert_eq!(listed["phones"][0]["id"], phone_id);

    // A notice goes out as one push request: the token, the environment, an
    // opaque collapse value and the ciphertext, and nothing readable.
    let notice = json!({
        "title": "a secret task title",
        "state": "needs_you",
        "tag": "device-1:w1:p1",
        "device_id": "device-1",
        "pane_id": "w1:p1",
        "clear": [],
    });
    let mobile = std::sync::Arc::clone(&running.mobile);
    let (id, payload) = (phone_id.clone(), notice.clone());
    let sent = tokio::task::spawn_blocking(move || mobile.push_to_app(&id, &payload));
    let request = push_request(&mut device, &mut session).await;
    assert_eq!(request.token, APP_TOKEN);
    assert_eq!(request.environment, PushEnvironment::Sandbox);
    assert!(!request.collapse.contains("w1") && !request.collapse.contains("device"));
    let body = URL_SAFE_NO_PAD.decode(&request.payload).unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("secret"));
    let text = ClientMessage::Push(request.clone()).encode().unwrap();
    assert!(!text.contains("a secret task title") && !text.contains("w1:p1"));
    control(
        &mut device,
        ServerMessage::PushResult(PushResult {
            id: request.id,
            result: PushOutcome::Delivered,
        }),
    )
    .await;
    assert_eq!(sent.await.unwrap(), Ok(()));
    // The phone's key opens it to the notice JSON hided sends Web Push.
    let plain: Value = serde_json::from_slice(&key.decrypt(&body)).unwrap();
    assert_eq!(plain, notice);

    // The same agent collapses to the same value on this phone.
    let mobile = std::sync::Arc::clone(&running.mobile);
    let (id, payload) = (phone_id.clone(), notice.clone());
    let sent = tokio::task::spawn_blocking(move || mobile.push_to_app(&id, &payload));
    let again = push_request(&mut device, &mut session).await;
    assert_eq!(again.collapse, request.collapse);
    assert_ne!(again.id, request.id);
    // Apple called the token gone: hided drops it and does not retry.
    control(
        &mut device,
        ServerMessage::PushResult(PushResult {
            id: again.id,
            result: PushOutcome::TokenGone,
        }),
    )
    .await;
    assert_eq!(sent.await.unwrap(), Err("token_gone"));
    assert!(!phones_file(dir.path()).contains(APP_TOKEN));
    state(&mut shell, |s| s["phones"][0]["notifications"] == "unasked").await;
    let mobile = std::sync::Arc::clone(&running.mobile);
    let (id, payload) = (phone_id.clone(), notice.clone());
    assert_eq!(
        tokio::task::spawn_blocking(move || mobile.push_to_app(&id, &payload))
            .await
            .unwrap(),
        Err("no_registration")
    );

    // Unpairing removes a registration with the phone.
    send(
        &mut device,
        &mut session,
        AppKey::new().registration(APP_TOKEN),
    )
    .await;
    receive(&mut device, &mut session, "push_state").await;
    assert!(phones_file(dir.path()).contains(APP_TOKEN));
    event(&mut shell, "mobile_revoke", json!({"phone_id": phone_id})).await;
    closed(&mut device, 1).await;
    assert!(!phones_file(dir.path()).contains(APP_TOKEN));
    running.stop();
}

#[tokio::test]
async fn an_older_relay_keeps_the_link_and_is_never_sent_a_push_request() {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut shell = shell(&running).await;
    event(&mut shell, "mobile_enable", json!({"enabled":true})).await;
    // Exactly what a relay from before push answers.
    let mut device = accept_answering(&listener, r#"{"type":"registered"}"#.to_owned()).await;
    state(&mut shell, |s| s["relay"]["state"] == "connected").await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let code = state(&mut shell, |s| s["qr"].is_string()).await;
    let qr = PairingQr::decode(code["qr"].as_str().unwrap()).unwrap();
    let mut session = phone(
        &mut device,
        &qr,
        1,
        json!({"client_kind":"phone","pair":qr.code,"name":"iPhone"}),
    )
    .await;
    let paired = receive(&mut device, &mut session, "paired").await;
    receive(&mut device, &mut session, "agents").await;
    send(
        &mut device,
        &mut session,
        AppKey::new().registration(APP_TOKEN),
    )
    .await;
    assert_eq!(
        receive(&mut device, &mut session, "push_state").await["notifications"],
        "on"
    );
    let mobile = std::sync::Arc::clone(&running.mobile);
    let id = paired["phone_id"].as_str().unwrap().to_owned();
    let sent = tokio::task::spawn_blocking(move || {
        mobile.push_to_app(&id, &json!({"title":"t","tag":"d:p","clear":[]}))
    });
    assert_eq!(sent.await.unwrap(), Err("relay_without_push"));
    // The link stays up and carries the phone; `receive` fails on any text
    // message, so no push request reached this relay.
    send(&mut device, &mut session, json!({"type":"ping","id":1})).await;
    receive(&mut device, &mut session, "pong").await;
    running.stop();
}

/// A real notice reaches an iOS app phone through the relay: a letter that
/// missed its delivery deadline in a seeded ledger becomes the operator
/// notice, and hided's delivery pass takes the app branch, encrypting it for
/// the registered key and handing it to the relay. Nothing calls the send
/// directly. The pass runs once a minute, so this waits for it.
#[cfg(unix)]
#[tokio::test]
async fn an_operator_notice_reaches_an_app_phone_through_the_relay() {
    use hide_relay_protocol::admission::{PushOutcome, PushResult};
    use std::os::unix::fs::PermissionsExt;
    const BODY: &str = "a letter body no push may carry";
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let private = |name: &str, value: Value| {
        let path = state_dir.join(name);
        std::fs::write(&path, value.to_string()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let actor = |name: &str| {
        json!({"pane_id": name, "name": name, "kind": "claude", "device_id": "local",
            "session": format!("{name}-session")})
    };
    private(
        "delivery-ledger.json",
        json!({"version": 1, "next_id": 100, "watches": [], "letters": [{
            "id": "letter-7", "intent": "held", "sender": actor("sender"),
            "recipient": actor("lead"), "kind": "report", "body": BODY,
            "state": "undelivered", "waiting_answer": false, "reply_to": null,
            "created_at_unix_ms": now - 3_600_000, "finished_at_unix_ms": now,
            "bell_errors": 0, "bell_sent": false, "human_notified": false,
        }]}),
    );
    private(
        "mobile.json",
        json!({"enabled": true, "method": "relay", "push_mode": "always"}),
    );
    let key = AppKey::new();
    let registration = key.registration(APP_TOKEN);
    private(
        "phones.json",
        json!({"phones": [{
            "id": "phone-1", "name": "iPhone", "credential_sha256": "0".repeat(64),
            "paired_at_ms": now, "last_seen_ms": now, "notifications": "on",
            "origin": "relay",
            "apns": {
                "token": APP_TOKEN, "environment": "sandbox",
                "p256dh": registration["key"]["p256dh"], "auth": registration["key"]["auth"],
            },
        }]}),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = hided::start_daemon(env(dir.path(), listener.local_addr().unwrap().port()))
        .await
        .unwrap();
    let mut device = accept_answering(
        &listener,
        ServerMessage::Registered { push: true }.encode().unwrap(),
    )
    .await;
    // The delivery pass starts a minute after the daemon; pings are answered
    // meanwhile so the link stays up.
    let request = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match device.next().await {
                Some(Ok(Message::Ping(bytes))) => {
                    device.send(Message::Pong(bytes)).await.unwrap();
                }
                Some(Ok(Message::Text(text))) => {
                    let ClientMessage::Push(request) = ClientMessage::decode(&text).unwrap() else {
                        panic!("expected push_request, got {text}");
                    };
                    return request;
                }
                Some(Ok(_)) => {}
                other => panic!("relay link ended: {other:?}"),
            }
        }
    })
    .await
    .expect("the delivery pass sent the notice to the relay");
    assert_eq!(request.token, APP_TOKEN);
    control(
        &mut device,
        ServerMessage::PushResult(PushResult {
            id: request.id,
            result: PushOutcome::Delivered,
        }),
    )
    .await;
    let body = URL_SAFE_NO_PAD.decode(&request.payload).unwrap();
    let plain: Value = serde_json::from_slice(&key.decrypt(&body)).unwrap();
    // The notice names the letter's place and the computer's room, and never
    // carries the letter's text; Apple sees neither.
    assert_eq!(plain["state"], "letter_undelivered");
    assert_eq!(plain["place"], "letter-7");
    assert!(plain["room"].is_string());
    assert!(!plain.to_string().contains(BODY));
    let wire = ClientMessage::Push(request).encode().unwrap();
    assert!(!wire.contains("letter-7"));
    running.stop();
}
