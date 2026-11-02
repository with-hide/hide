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
    control(&mut device, ServerMessage::Registered).await;
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
        if let Some(plain) = phone.recv_message(&bytes, 0).unwrap() {
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
