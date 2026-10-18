//! Settings > Mobile and the phone socket against a real daemon and a fake
//! `tailscale` CLI (PRD mobile-companion B1-B10, B13-B18, B36, B37). The fake
//! keeps its whole state in files in the test's own folder, so nothing here
//! reaches the machine's Tailscale.
// A daemon starts only where it can listen for panes locally, which Windows
// cannot yet (#315).
#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

use crate::support::fake_tailscale::{DNS, FakeTailscale};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn env(dir: &Path, tailscale: &Path) -> Env {
    Env {
        home: dir.to_path_buf(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        login_shell: None,
        state_dir: dir.join("state"),
        legacy_state_dir: None,
        account_state_folder: false,
        keep_alive: true,
        vite_origin: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        idle_secs: 600,
        build: None,
        starter_program: None,
        open_command: None,
        host_helper_root: None,
        host_cli_dir: None,
        pane_id: None,
        tailscale_bin: Some(tailscale.to_path_buf()),
        search_path: None,
    }
}

async fn connect(port: u16, origin: &str) -> Socket {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(ORIGIN, origin.parse().unwrap());
    request.headers_mut().insert(
        "user-agent",
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)"
            .parse()
            .unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect")
        .0
}

fn loopback(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

async fn next_frame(socket: &mut Socket) -> Option<Value> {
    loop {
        match tokio::time::timeout(Duration::from_secs(15), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).ok(),
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Err(_) => return None,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) => return None,
        }
    }
}

/// A renderer (the desktop shell) on the shell token; returns its socket.
async fn renderer(running: &hided::RunningDaemon) -> Socket {
    let mut socket = connect(running.port, &loopback(running.port)).await;
    socket
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2, "client_kind": "desktop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
}

/// Reads frames until a `mobile` frame satisfies `accept`.
async fn mobile_frame(socket: &mut Socket, accept: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no matching mobile frame"
        );
        let frame = next_frame(socket).await.expect("socket open");
        if frame["type"] == "mobile" && accept(&frame["payload"]) {
            return frame["payload"].clone();
        }
    }
}

async fn event(socket: &mut Socket, kind: &str, payload: Value) {
    socket
        .send(Message::Text(
            json!({"schema_version": 2, "kind": kind, "payload": payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

fn pair_code(qr: &str) -> String {
    pair_code_at(qr, DNS)
}

fn pair_code_at(qr: &str, dns: &str) -> String {
    use base64::Engine;
    let encoded = qr.split("#pair=").nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["endpoint"], format!("https://{dns}"));
    value["code"].as_str().unwrap().to_owned()
}

async fn pair(port: u16, origin: &str, code: &str) -> (Socket, Value) {
    let mut socket = connect(port, origin).await;
    socket
        .send(Message::Text(
            json!({"client_kind": "phone", "pair": code, "name": "Mozilla/5.0 (iPhone)"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let first = next_frame(&mut socket).await.expect("an answer");
    (socket, first)
}

async fn phone(port: u16, credential: &str) -> (Socket, Value) {
    phone_at(port, DNS, credential).await
}

async fn phone_at(port: u16, dns: &str, credential: &str) -> (Socket, Value) {
    let mut socket = connect(port, &format!("https://{dns}")).await;
    socket
        .send(Message::Text(
            json!({"client_kind": "phone", "token": credential, "schema_version": 2})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let first = next_frame(&mut socket).await.expect("an answer");
    (socket, first)
}

async fn close_code(socket: &mut Socket) -> Option<u16> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            if let Ok(Message::Close(Some(close))) = frame {
                return Some(close.code.into());
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn mobile_is_off_by_default_and_never_runs_tailscale() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let frame = mobile_frame(&mut shell, |_| true).await;
    assert_eq!(frame["enabled"], false);
    assert_eq!(frame["exposure"], "off");
    assert!(frame["qr"].is_null());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.calls(), "", "off never runs tailscale (B1)");
    // The tailnet origin is not allowed while off (B18).
    let mut outside = connect(running.port, &format!("https://{DNS}")).await;
    outside
        .send(Message::Text(
            json!({"client_kind": "phone", "pair": "x"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(close_code(&mut outside).await, Some(4002));
    running.stop();
}

#[tokio::test]
async fn the_checklist_advances_and_the_qr_follows_a_confirmed_serve_entry() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.logged_out();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    mobile_frame(&mut shell, |_| true).await;
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    let blocked = mobile_frame(&mut shell, |frame| frame["exposure"] == "blocked").await;
    assert_eq!(blocked["checklist"]["installed"], "ok");
    assert_eq!(blocked["checklist"]["logged_in"], "failed");
    assert_eq!(blocked["checklist"]["https"], "waiting");
    assert!(blocked["qr"].is_null());
    // Logging in without reopening Settings moves the list on (B3).
    fake.https_off();
    let https = mobile_frame(&mut shell, |frame| frame["checklist"]["https"] == "failed").await;
    assert_eq!(https["checklist"]["logged_in"], "ok");
    assert_eq!(https["checklist"]["host_name"], "mac");
    assert!(
        fake.proxy().is_none(),
        "no serve entry before every check passes"
    );
    fake.ready();
    let exposed = mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    let port = running.port;
    assert_eq!(fake.proxy(), Some(format!("http://127.0.0.1:{port}")));
    assert_eq!(exposed["url"], format!("https://{DNS}"));
    // Opening Settings > Mobile and exposing issue no code (B58).
    assert!(exposed["qr"].is_null());
    assert!(exposed["code_expires_at_ms"].is_null());
    event(&mut shell, "mobile_show_code", json!({})).await;
    let shown = mobile_frame(&mut shell, |frame| frame["qr"].is_string()).await;
    let qr = shown["qr"].as_str().expect("a QR once shown (B4)");
    assert!(qr.starts_with(&format!("https://{DNS}/m/#pair=")));
    assert!(shown["code_expires_at_ms"].as_u64().unwrap() > shown["now_ms"].as_u64().unwrap());
    // Observing again keeps the shown code and the last page closing takes it.
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    let kept = mobile_frame(&mut shell, |_| true).await;
    assert_eq!(kept["qr"], shown["qr"]);
    event(&mut shell, "mobile_hide_code", json!({})).await;
    mobile_frame(&mut shell, |frame| frame["qr"].is_null()).await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let qr_frame = mobile_frame(&mut shell, |frame| frame["qr"].is_string()).await;
    let qr = qr_frame["qr"].as_str().unwrap();
    // Enabling again converges on the same single entry (B9).
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(fake.calls().matches("serve --bg").count(), 1);
    // A new code voids the one in the QR (B10).
    let old = pair_code(qr);
    event(&mut shell, "mobile_show_code", json!({})).await;
    let renewed = mobile_frame(&mut shell, |frame| {
        frame["qr"].as_str().is_some_and(|next| next != qr)
    })
    .await;
    let (mut stale, answer) = pair(port, &format!("https://{DNS}"), &old).await;
    assert_eq!(answer, json!({"type": "refused", "reason": "code_expired"}));
    assert_eq!(close_code(&mut stale).await, Some(4001));
    // The shell token is never accepted from the tailnet origin.
    let mut shell_from_tailnet = connect(port, &format!("https://{DNS}")).await;
    shell_from_tailnet
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(close_code(&mut shell_from_tailnet).await, Some(4002));
    let code = pair_code(renewed["qr"].as_str().unwrap());
    let (_paired, answer) = pair(port, &format!("https://{DNS}"), &code).await;
    assert_eq!(answer["type"], "paired");
    assert_eq!(answer["name"], "iPhone");
    let listed = mobile_frame(&mut shell, |frame| {
        frame["phones"]
            .as_array()
            .is_some_and(|phones| phones.len() == 1)
    })
    .await;
    assert_eq!(listed["phones"][0]["name"], "iPhone");
    assert_eq!(listed["phones"][0]["connected"], true);
    // Turning it off removes only hide's entry and forgets the origin (B7).
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "off").await;
    fake.wait_until_removed().await;
    let mut refused = connect(port, &format!("https://{DNS}")).await;
    refused
        .send(Message::Text(
            json!({"client_kind": "phone", "token": "x"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(close_code(&mut refused).await, Some(4002));
    running.stop();
}

#[tokio::test]
async fn a_foreign_entry_or_a_failed_command_leaves_mobile_unexposed() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let theirs = json!({"TCP": {"443": {"HTTPS": true}}, "Web": {format!("{DNS}:443"): {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}}}});
    fake.set_serve(theirs.clone());
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    let foreign = mobile_frame(&mut shell, |frame| frame["exposure"] == "foreign").await;
    assert!(
        foreign["foreign_target"]
            .as_str()
            .unwrap()
            .contains("http://127.0.0.1:3000")
    );
    assert!(foreign["qr"].is_null());
    assert_eq!(
        fake.serve(),
        theirs,
        "hide never touches an entry it did not add (B5)"
    );
    assert!(!fake.calls().contains("off"));
    // A serve command that fails names its step and shows no QR (B6).
    fake.set_serve(json!({}));
    std::fs::write(fake.state.join("fail-serve"), "").unwrap();
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "off").await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    let failed = mobile_frame(&mut shell, |frame| frame["exposure"] == "failed").await;
    assert_eq!(failed["failure"]["step"], "add");
    assert_eq!(
        failed["failure"]["message"],
        "serve config denied: access denied"
    );
    assert!(failed["qr"].is_null());
    // Fixed, then switched on again, it goes on (B6).
    std::fs::remove_file(fake.state.join("fail-serve")).unwrap();
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "off").await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    running.stop();
}

#[tokio::test]
async fn a_restart_on_a_new_port_replaces_hides_entry_and_phones_stay_paired() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let first_port = running.port;
    let mut shell = renderer(&running).await;
    // Settings > Mobile is open, as it is whenever the switch is used.
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let exposed = mobile_frame(&mut shell, |frame| frame["qr"].is_string()).await;
    let code = pair_code(exposed["qr"].as_str().unwrap());
    let (mut paired, answer) = pair(first_port, &loopback(first_port), &code).await;
    let credential = answer["credential"].as_str().unwrap().to_owned();
    let phones = std::fs::read_to_string(dir.path().join("state/phones.json")).unwrap();
    assert!(
        !phones.contains(&credential),
        "only the credential's hash is stored"
    );
    assert!(hide_platform::fs::private::is_private(&dir.path().join("state/phones.json")).unwrap());
    // A crash: the process goes without its graceful stop.
    let _ = paired.close(None).await;
    drop(shell);
    running.stop();
    drop(running);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.proxy(), Some(format!("http://127.0.0.1:{first_port}")));
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let second_port = running.port;
    assert_ne!(first_port, second_port);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while fake.proxy() != Some(format!("http://127.0.0.1:{second_port}")) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the old port's entry was not replaced: {:?}",
            fake.serve()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The serve command writes its config before Mobile admits the new Origin.
    // The exposed frame confirms the daemon's admission state.
    let mut shell = renderer(&running).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    // The phone's credential outlives the restart and the port change (B8).
    let (_socket, hello) = phone(second_port, &credential).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["mac_name"], "mac");
    // Mobile on with a phone paired keeps the daemon from its idle exit (B37).
    assert!(running.mobile.keep_alive());
    // A graceful stop removes hide's entry (B8).
    running.mobile.shutdown().await;
    assert!(fake.proxy().is_none());
    running.stop();
}

/// Pairs a phone with the core `running` exposes at `dns`, where `paired`
/// phones are listed already, and answers its credential.
async fn pair_at(
    running: &hided::RunningDaemon,
    shell: &mut Socket,
    dns: &str,
    paired: usize,
) -> String {
    event(shell, "mobile_show_code", json!({})).await;
    // A frame published before the last pairing can still be unread and
    // carries the code that pairing spent; the pairing that spent it also
    // listed its phone, so the code shown for this one is the first beside
    // `paired` phones.
    let shown = mobile_frame(shell, |frame| {
        frame["qr"].is_string()
            && frame["phones"]
                .as_array()
                .is_some_and(|phones| phones.len() == paired)
    })
    .await;
    let code = pair_code_at(shown["qr"].as_str().unwrap(), dns);
    let (_paired, answer) = pair(running.port, &loopback(running.port), &code).await;
    answer["credential"].as_str().unwrap().to_owned()
}

/// The state folder moved to a machine with another tailnet name lets in
/// only the phones paired there: one paired where the core ran before is
/// told the core is unreachable and stays paired, the Settings list and the
/// four-phone limit leave it out, and a move back lets it in again (PRD
/// core-host-node-move B17).
#[tokio::test]
async fn a_moved_core_lets_in_only_the_phones_paired_at_its_address() {
    const MINI: &str = "mini.tailnet-name.ts.net";
    let dir = tempfile::tempdir().unwrap();
    let mac = FakeTailscale::new(&dir.path().join("mac"));
    let mini = FakeTailscale::named(&dir.path().join("mini"), MINI);
    mac.ready();
    mini.ready();
    let start = |fake: &FakeTailscale| hided::start_daemon(env(dir.path(), &fake.bin));
    let exposed = |frame: &Value| frame["exposure"] == "exposed";

    let running = start(&mac).await.unwrap();
    let mut shell = renderer(&running).await;
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(&mut shell, exposed).await;
    let at_mac = pair_at(&running, &mut shell, DNS, 0).await;
    running.mobile.shutdown().await;
    running.stop();
    drop((shell, running));

    let running = start(&mini).await.unwrap();
    let mut shell = renderer(&running).await;
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    let frame = mobile_frame(&mut shell, exposed).await;
    assert_eq!(frame["phones"], json!([]), "{frame}");
    let (_socket, answer) = phone_at(running.port, MINI, &at_mac).await;
    assert_eq!(answer, json!({"type": "refused", "reason": "mobile_off"}));
    for paired in 0..4 {
        pair_at(&running, &mut shell, MINI, paired).await;
    }
    let frame = mobile_frame(&mut shell, |frame| {
        frame["phones"]
            .as_array()
            .is_some_and(|phones| phones.len() == 4)
    })
    .await;
    assert_eq!(frame["max_phones"], 4);
    running.mobile.shutdown().await;
    running.stop();
    drop((shell, running));

    let running = start(&mac).await.unwrap();
    let mut shell = renderer(&running).await;
    mobile_frame(&mut shell, exposed).await;
    let (_socket, hello) = phone(running.port, &at_mac).await;
    assert_eq!(hello["type"], "hello", "{hello}");
    running.stop();
}

#[tokio::test]
async fn the_last_mobile_page_closing_takes_the_shown_code_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut page = renderer(&running).await;
    event(&mut page, "mobile_observe", json!({"observing": true})).await;
    event(&mut page, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(&mut page, |frame| frame["exposure"] == "exposed").await;
    event(&mut page, "mobile_show_code", json!({})).await;
    mobile_frame(&mut page, |frame| frame["qr"].is_string()).await;
    // Another window that never opened Settings > Mobile is not an observer.
    let mut other = renderer(&running).await;
    mobile_frame(&mut other, |frame| frame["qr"].is_string()).await;
    drop(page);
    mobile_frame(&mut other, |frame| frame["qr"].is_null()).await;
    running.mobile.shutdown().await;
    running.stop();
}

#[tokio::test]
async fn phones_are_capped_revoked_and_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let port = running.port;
    let mut shell = renderer(&running).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    let mut credentials = Vec::new();
    for _ in 0..4 {
        event(&mut shell, "mobile_show_code", json!({})).await;
        let paired = credentials.len();
        let frame = mobile_frame(&mut shell, |frame| {
            frame["qr"].is_string()
                && frame["phones"]
                    .as_array()
                    .is_some_and(|phones| phones.len() == paired)
        })
        .await;
        let (mut socket, answer) = pair(
            port,
            &format!("https://{DNS}"),
            &pair_code(frame["qr"].as_str().unwrap()),
        )
        .await;
        assert_eq!(answer["type"], "paired", "{answer}");
        credentials.push((
            answer["phone_id"].as_str().unwrap().to_owned(),
            answer["credential"].as_str().unwrap().to_owned(),
        ));
        let _ = socket.close(None).await;
    }
    event(&mut shell, "mobile_show_code", json!({})).await;
    let frame = mobile_frame(&mut shell, |frame| {
        frame["phones"]
            .as_array()
            .is_some_and(|phones| phones.len() == 4)
            && frame["qr"].is_string()
    })
    .await;
    assert_eq!(frame["max_phones"], 4);
    let (_fifth, answer) = pair(
        port,
        &format!("https://{DNS}"),
        &pair_code(frame["qr"].as_str().unwrap()),
    )
    .await;
    assert_eq!(
        answer,
        json!({"type": "refused", "reason": "phone_limit"}),
        "B14"
    );
    // A phone's vocabulary: everything else is refused (B17).
    let (mut socket, hello) = phone(port, &credentials[0].1).await;
    assert_eq!(hello["type"], "hello");
    for request in [
        json!({"type": "file_open", "path": "/etc/hosts"}),
        json!({"schema_version": 2, "kind": "ui_state_update", "payload": {}}),
        json!({"type": "snapshot"}),
        json!({"type": "open", "device_id": "local", "pane_id": "w9:p9"}),
    ] {
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
    }
    let mut refused = 0;
    let mut gone = 0;
    while refused + gone < 4 {
        let frame = next_frame(&mut socket).await.expect("an answer");
        match frame["type"].as_str() {
            Some("refused_request") => refused += 1,
            Some("rows") => {
                assert_eq!(
                    frame["state"], "gone",
                    "a pane that is no agent is not opened"
                );
                gone += 1;
            }
            _ => {}
        }
    }
    // Revoking closes its live connection and a second revoke is quiet (B15, B9).
    event(
        &mut shell,
        "mobile_revoke",
        json!({"phone_id": credentials[0].0}),
    )
    .await;
    event(
        &mut shell,
        "mobile_revoke",
        json!({"phone_id": credentials[0].0}),
    )
    .await;
    let mut revoked = None;
    while let Some(frame) = next_frame(&mut socket).await {
        if frame["type"] == "refused" {
            revoked = Some(frame);
        }
    }
    assert_eq!(
        revoked,
        Some(json!({"type": "refused", "reason": "revoked"}))
    );
    let (_again, answer) = phone(port, &credentials[0].1).await;
    assert_eq!(
        answer,
        json!({"type": "refused", "reason": "revoked"}),
        "B16"
    );
    let frame = mobile_frame(&mut shell, |frame| {
        frame["phones"]
            .as_array()
            .is_some_and(|phones| phones.len() == 3)
    })
    .await;
    assert!(
        frame["phones"]
            .as_array()
            .unwrap()
            .iter()
            .all(|phone| phone["id"] != credentials[0].0)
    );
    running.stop();
}

#[tokio::test]
async fn a_phone_unseen_for_seven_days_is_revoked_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let week = 7 * 24 * 60 * 60 * 1000_u64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let hash = |credential: &str| hided::mobile::phones::credential_hash(credential);
    std::fs::write(
        state.join("phones.json"),
        json!({"phones": [
            {"id": "old", "name": "iPad", "credential_sha256": hash("old-credential"), "paired_at_ms": 0, "last_seen_ms": now - week - 1000,
             "notifications": "on", "push": {"endpoint": "https://web.push.apple.com/x", "p256dh": "k", "auth": "a"}},
            {"id": "recent", "name": "iPhone", "credential_sha256": hash("recent-credential"), "paired_at_ms": 0, "last_seen_ms": now - 3 * 24 * 60 * 60 * 1000},
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        state.join("mobile.json"),
        json!({"enabled": true}).to_string(),
    )
    .unwrap();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let frame = mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    let phones = frame["phones"].as_array().unwrap();
    assert_eq!(phones.len(), 1);
    assert_eq!(phones[0]["id"], "recent");
    assert_eq!(
        phones[0]["revoke_at_ms"],
        phones[0]["last_seen_ms"].as_u64().unwrap() + week
    );
    let file = std::fs::read_to_string(state.join("phones.json")).unwrap();
    assert!(
        !file.contains("web.push.apple.com"),
        "the subscription went with it (B36)"
    );
    let (_socket, answer) = phone(running.port, "old-credential").await;
    assert_eq!(answer, json!({"type": "refused", "reason": "revoked"}));
    running.stop();
}

#[tokio::test]
async fn the_phone_app_is_served_under_m_and_nothing_else_leaks() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let status = |path: &str| {
        let url = format!("http://127.0.0.1:{}{path}", running.port);
        async move {
            let output = tokio::process::Command::new("/usr/bin/curl")
                .args([
                    "-s",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    "--path-as-is",
                    &url,
                ])
                .output()
                .await
                .unwrap();
            String::from_utf8(output.stdout).unwrap()
        }
    };
    assert_eq!(status("/m").await, "308");
    assert_eq!(status("/m/../hided.json").await, "404");
    assert_eq!(status("/m/missing.js").await, "404");
    running.stop();
}

async fn http_status(port: u16, path: &str, headers: &[&str]) -> String {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut args = vec!["-s", "-o", "/dev/null", "-w", "%{http_code}"];
    for header in headers {
        args.extend(["-H", header]);
    }
    args.push(&url);
    let output = tokio::process::Command::new("/usr/bin/curl")
        .args(&args)
        .output()
        .await
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn the_tailnet_reaches_only_the_phone_app_and_transport_trouble_stays_visible() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    // Funnel publishes the address to the internet: hide stays unexposed.
    fake.set_serve(json!({"AllowFunnel": {format!("{DNS}:443"): true}}));
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let port = running.port;
    let mut shell = renderer(&running).await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    let funnel = mobile_frame(&mut shell, |frame| frame["exposure"] == "failed").await;
    assert_eq!(funnel["failure"]["step"], "funnel");
    assert!(funnel["qr"].is_null());
    assert!(
        !fake.calls().contains("--bg"),
        "nothing is added under a Funnel"
    );

    // An add that took but then failed is still hide's own on the next pass.
    fake.set_serve(json!({}));
    std::fs::write(fake.state.join("apply-then-fail"), "").unwrap();
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    mobile_frame(&mut shell, |frame| frame["exposure"] == "off").await;
    event(&mut shell, "mobile_enable", json!({"enabled": true})).await;
    let failed = mobile_frame(&mut shell, |frame| frame["exposure"] == "failed").await;
    assert_eq!(failed["failure"]["step"], "add");
    std::fs::remove_file(fake.state.join("apply-then-fail")).unwrap();
    event(&mut shell, "mobile_observe", json!({"observing": true})).await;
    let exposed = mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;
    assert_eq!(
        fake.calls().matches("--bg").count(),
        1,
        "the entry it had added is recognised, not added again or called foreign"
    );

    // Through `tailscale serve` only the phone app and /ws answer, and /ws
    // takes only a phone, whatever Origin the caller claims.
    let tailnet_host = format!("Host: {DNS}");
    assert_eq!(http_status(port, "/health", &[&tailnet_host]).await, "404");
    assert_eq!(http_status(port, "/", &[&tailnet_host]).await, "404");
    // Tailscale routes by TLS name, so a tailnet caller can claim a loopback
    // Host; the forwarded-for header serve always adds still gives it away.
    let loopback_host = format!("Host: 127.0.0.1:{port}");
    assert_eq!(
        http_status(
            port,
            "/health",
            &[&loopback_host, "X-Forwarded-For: 100.64.0.7"]
        )
        .await,
        "404"
    );
    assert_eq!(http_status(port, "/health", &[]).await, "200");
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(ORIGIN, loopback(port).parse().unwrap());
    request.headers_mut().insert("host", DNS.parse().unwrap());
    let mut forged = tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect")
        .0;
    forged
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        close_code(&mut forged).await,
        Some(4002),
        "the shell token is refused through serve even with a loopback Origin"
    );
    let _ = exposed;
    event(&mut shell, "mobile_show_code", json!({})).await;
    let shown = mobile_frame(&mut shell, |frame| frame["qr"].is_string()).await;
    let code = pair_code(shown["qr"].as_str().unwrap());
    let (mut paired, answer) = pair(port, &format!("https://{DNS}"), &code).await;
    assert_eq!(answer["type"], "paired");

    // A Funnel turned on after exposure takes hide's entry down and closes
    // every phone; turned off again, the entry comes back.
    std::fs::write(fake.state.join("funnel"), "").unwrap();
    let mut serve = fake.serve();
    serve["AllowFunnel"] = json!({format!("{DNS}:443"): true});
    fake.set_serve(serve);
    let funnel = mobile_frame(&mut shell, |frame| frame["failure"]["step"] == "funnel").await;
    assert!(funnel["qr"].is_null());
    assert_eq!(close_code(&mut paired).await, Some(4001));
    fake.wait_until_removed().await;
    std::fs::remove_file(fake.state.join("funnel")).unwrap();
    fake.set_serve(json!({}));
    mobile_frame(&mut shell, |frame| frame["exposure"] == "exposed").await;

    // A removal that fails at switch-off stays on screen, and the next pass
    // finishes it.
    std::fs::write(fake.state.join("fail-remove"), "").unwrap();
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    let stuck = mobile_frame(&mut shell, |frame| {
        frame["exposure"] == "failed" && frame["enabled"] == false
    })
    .await;
    assert_eq!(stuck["failure"]["step"], "remove");
    assert!(fake.proxy().is_some());
    std::fs::remove_file(fake.state.join("fail-remove")).unwrap();
    event(&mut shell, "mobile_enable", json!({"enabled": false})).await;
    fake.wait_until_removed().await;
    running.stop();
}

/// Turns Mobile on and pairs one phone; returns its socket after `hello`,
/// its id and its credential.
async fn paired_phone(
    running: &hided::RunningDaemon,
    shell: &mut Socket,
) -> (Socket, String, String) {
    event(shell, "mobile_enable", json!({"enabled": true})).await;
    mobile_frame(shell, |frame| frame["exposure"] == "exposed").await;
    event(shell, "mobile_show_code", json!({})).await;
    let frame = mobile_frame(shell, |frame| frame["qr"].is_string()).await;
    let (_pairing, answer) = pair(
        running.port,
        &format!("https://{DNS}"),
        &pair_code(frame["qr"].as_str().unwrap()),
    )
    .await;
    assert_eq!(answer["type"], "paired", "{answer}");
    let phone_id = answer["phone_id"].as_str().unwrap().to_owned();
    let credential = answer["credential"].as_str().unwrap().to_owned();
    let (socket, hello) = phone(running.port, &credential).await;
    assert_eq!(hello["type"], "hello");
    (socket, phone_id, credential)
}

async fn send_phone(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

/// Opens the start sheet and returns the catalog hided sends for it.
async fn open_sheet(socket: &mut Socket) -> Value {
    send_phone(socket, json!({"type": "start_sheet", "open": true})).await;
    loop {
        let frame = next_frame(socket).await.expect("a start catalog");
        if frame["type"] == "start_catalog"
            && frame["targets"].as_array().is_some_and(|t| !t.is_empty())
        {
            return frame;
        }
    }
}

async fn start_result(socket: &mut Socket, request_id: &str) -> Value {
    loop {
        let frame = next_frame(socket).await.expect("a start result");
        if frame["type"] == "start_result" && frame["request_id"] == request_id {
            return frame;
        }
    }
}

/// Every distinct answer the core's snapshot carries for `request_id` within `window`:
/// its task slot and its last error, as the desktop shell would read them.
async fn core_answers(shell: &mut Socket, request_id: &str, window: Duration) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + window;
    let mut seen: Vec<Value> = Vec::new();
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, next_frame(shell)).await {
        for pointer in [
            "/payload/rest/status/last_error",
            "/payload/rest/task_operation",
        ] {
            if let Some(value) = frame.pointer(pointer)
                && value["request_id"] == request_id
                && !seen.contains(value)
            {
                seen.push(value.clone());
            }
        }
    }
    seen
}

#[tokio::test]
async fn a_start_from_an_unadmitted_phone_is_refused_and_nothing_reaches_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let (mut socket, phone_id, credential) = paired_phone(&running, &mut shell).await;
    let catalog = open_sheet(&mut socket).await;
    assert_eq!(catalog["targets"][0]["id"], home_target().as_str());
    let start = |request_id: &str| json!({"type": "start_agent", "request_id": request_id, "text": "테스트 고쳐줘", "target": home_target().as_str(), "kind": "claude"});
    // A connection with no credential never reaches the vocabulary (B45).
    let (mut stranger, refused) = phone(running.port, &"0".repeat(64)).await;
    assert_eq!(refused["type"], "refused");
    let _ = stranger
        .send(Message::Text(start("stranger-1").to_string().into()))
        .await;
    // A phone revoked after it connected is refused before anything is dispatched.
    event(&mut shell, "mobile_revoke", json!({"phone_id": phone_id})).await;
    mobile_frame(&mut shell, |frame| {
        frame["phones"].as_array().is_some_and(Vec::is_empty)
    })
    .await;
    let answer = running
        .mobile
        .start_agent(&phone_id, &start("revoked-1"))
        .await;
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["reason"], "revoked");
    let (_again, answer) = phone(running.port, &credential).await;
    assert_eq!(answer, json!({"type": "refused", "reason": "revoked"}));
    for request_id in ["stranger-1", "revoked-1"] {
        let answers = core_answers(&mut shell, request_id, Duration::from_millis(800)).await;
        assert!(answers.is_empty(), "the core saw {request_id}: {answers:?}");
    }
    running.stop();
}

/// The next `pong` the phone receives, with the other frames passed over.
async fn next_pong(socket: &mut Socket) -> Value {
    loop {
        let frame = next_frame(socket).await.expect("a pong");
        if frame["type"] == "pong" {
            return frame;
        }
    }
}

#[tokio::test]
async fn a_ping_gets_its_number_back_until_the_phone_is_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let (mut socket, phone_id, _credential) = paired_phone(&running, &mut shell).await;
    send_phone(&mut socket, json!({"type": "ping", "id": 41})).await;
    assert_eq!(
        next_pong(&mut socket).await,
        json!({"type": "pong", "id": 41})
    );
    // An input is written beside the socket loop and answers as a frame; the loop keeps answering after it.
    send_phone(
        &mut socket,
        json!({"type": "input", "request_id": "r-1", "device_id": "local", "pane_id": "w9:p9", "text": "hi"}),
    )
    .await;
    send_phone(&mut socket, json!({"type": "ping", "id": 40})).await;
    let mut result = None;
    let mut pong = None;
    while result.is_none() || pong.is_none() {
        let frame = next_frame(&mut socket).await.expect("an answer");
        match frame["type"].as_str() {
            Some("input_result") => result = Some(frame),
            Some("pong") => pong = Some(frame),
            _ => {}
        }
    }
    assert_eq!(result.unwrap()["request_id"], "r-1");
    assert_eq!(pong.unwrap(), json!({"type": "pong", "id": 40}));
    // A ping with no number is refused and recorded; the connection stays and the next one is answered.
    send_phone(&mut socket, json!({"type": "ping"})).await;
    send_phone(&mut socket, json!({"type": "ping", "id": 42})).await;
    assert_eq!(
        next_pong(&mut socket).await,
        json!({"type": "pong", "id": 42})
    );
    // A phone no longer let in is never answered with a pong.
    event(&mut shell, "mobile_revoke", json!({"phone_id": phone_id})).await;
    mobile_frame(&mut shell, |frame| {
        frame["phones"].as_array().is_some_and(Vec::is_empty)
    })
    .await;
    let _ = socket
        .send(Message::Text(
            json!({"type": "ping", "id": 43}).to_string().into(),
        ))
        .await;
    let mut refused = false;
    while let Some(frame) = next_frame(&mut socket).await {
        assert_ne!(frame["type"], "pong", "{frame}");
        refused |= frame == json!({"type": "refused", "reason": "revoked"});
    }
    assert!(refused, "a revoked phone is told why");
    running.stop();
}

#[tokio::test]
async fn a_repeated_start_request_id_starts_once_and_returns_the_first_answer() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let (mut socket, _phone_id, _credential) = paired_phone(&running, &mut shell).await;
    open_sheet(&mut socket).await;
    let start = json!({"type": "start_agent", "request_id": "retry-1", "text": "테스트 고쳐줘", "target": home_target().as_str(), "kind": "claude"});
    send_phone(&mut socket, start.clone()).await;
    let first = start_result(&mut socket, "retry-1").await;
    assert_ne!(first["reason"], "unknown_target", "{first}");
    // The core answered the request id once; a network retry of the same id
    // changes nothing and gets the same answer.
    let before = core_answers(&mut shell, "retry-1", Duration::from_millis(500)).await;
    assert!(!before.is_empty(), "the core never saw the start");
    send_phone(&mut socket, start).await;
    let second = start_result(&mut socket, "retry-1").await;
    assert_eq!(second, first);
    let after = core_answers(&mut shell, "retry-1", Duration::from_millis(800)).await;
    assert!(
        after.iter().all(|value| before.contains(value)),
        "a second start reached the core: {after:?}"
    );
    running.stop();
}

#[tokio::test]
async fn a_start_the_sheet_never_offered_is_refused_by_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut shell = renderer(&running).await;
    let (mut socket, _phone_id, _credential) = paired_phone(&running, &mut shell).await;
    open_sheet(&mut socket).await;
    for (request_id, target, kind, reason) in [
        ("bad-target", "/etc", "claude", "unknown_target"),
        (
            "bad-kind",
            home_target().as_str(),
            "terminal",
            "unknown_kind",
        ),
    ] {
        send_phone(
            &mut socket,
            json!({"type": "start_agent", "request_id": request_id, "text": "x", "target": target, "kind": kind}),
        )
        .await;
        let answer = start_result(&mut socket, request_id).await;
        assert_eq!(
            (answer["ok"].clone(), answer["reason"].clone()),
            (json!(false), json!(reason))
        );
        assert!(
            core_answers(&mut shell, request_id, Duration::from_millis(300))
                .await
                .is_empty()
        );
    }
    send_phone(
        &mut socket,
        json!({"type": "start_agent", "request_id": "not a valid id", "text": "x", "target": home_target().as_str(), "kind": "claude"}),
    )
    .await;
    loop {
        let frame = next_frame(&mut socket).await.expect("an answer");
        if frame["type"] == "start_result" {
            assert_eq!(frame["reason"], "invalid_request");
            break;
        }
    }
    running.stop();
}

/// The phone's start target for this machine's Home: `home:<node id>`.
fn home_target() -> String {
    format!("home:{}", hide_platform::host::machine_id().unwrap())
}

/// The desktop app's agent notifications share the phone's follower but not
/// its setting: only a desktop window can turn them on (PRD
/// first-run-onboarding D-14), and a browser tab asking is refused.
#[tokio::test]
async fn only_a_desktop_window_turns_agent_notifications_on() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTailscale::new(dir.path());
    fake.ready();
    let running = hided::start_daemon(env(dir.path(), &fake.bin))
        .await
        .unwrap();
    let mut tab = connect(running.port, &loopback(running.port)).await;
    tab.send(Message::Text(
        json!({"token": running.token, "schema_version": 2, "client_kind": "web"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    event(&mut tab, "desktop_notices", json!({"wanted": true})).await;
    loop {
        let frame = next_frame(&mut tab).await.expect("socket open");
        if frame["type"] == "error" {
            assert_eq!(frame["message"], "desktop_notices is a desktop window's");
            break;
        }
    }

    let mut window = renderer(&running).await;
    event(&mut window, "desktop_notices", json!({"wanted": true})).await;
    event(&mut window, "desktop_notices", json!({"wanted": "yes"})).await;
    loop {
        let frame = next_frame(&mut window).await.expect("socket open");
        if frame["type"] == "error" {
            assert_eq!(
                frame["message"], "desktop_notices.wanted must be a boolean",
                "the well-formed request before it was taken without an error"
            );
            break;
        }
    }
    running.stop();
}
