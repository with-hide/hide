//! The daemon's idle exit beside automations (PRD automations B32): an
//! automation that is on keeps a daemon with no screen and no pane past its
//! idle window, so its time comes on time; once none is on, the daemon ends
//! after the window as before.
// A daemon starts only where it can listen for panes locally, which Windows
// cannot yet (#315).
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A daemon of its own state folder and home that ends one second after its
/// last screen left, when nothing holds it.
fn daemon_env(root: &Path) -> Env {
    let state = root.join("state");
    hide_platform::fs::private::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(root.join("operator")).unwrap();
    Env {
        home: root.join("operator"),
        state_dir: state,
        legacy_state_dir: None,
        account_state_folder: false,
        keep_alive: false,
        vite_origin: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        idle_secs: 1,
        build: None,
        starter_program: None,
        open_command: None,
        host_helper_root: None,
        host_cli_dir: None,
        pane_id: None,
        install: None,
        access: None,
        tailscale_bin: Some(root.join("no-tailscale")),
        relay_url: "ws://127.0.0.1:9/ws".to_owned(),
        search_path: None,
    }
}

/// The operator's window, the screen that makes and removes automations.
async fn window(running: &hided::RunningDaemon) -> Socket {
    let mut request = format!("ws://127.0.0.1:{}/ws", running.port)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        ORIGIN,
        format!("http://127.0.0.1:{}", running.port)
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let hello = json!({"token": running.token, "schema_version": 2, "client_kind": "web"});
    socket
        .send(Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    socket
}

async fn send(socket: &mut Socket, kind: &str, payload: Value) {
    let event =
        json!({"schema_version": hide_core::SCHEMA_VERSION, "kind": kind, "payload": payload});
    socket
        .send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}

/// The names in the `automations` section of the first frame carrying one.
fn automation_names(frame: &Value) -> Option<Vec<String>> {
    match frame {
        Value::Object(map) => map
            .get("automations")
            .and_then(|section| section["items"].as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .or_else(|| map.values().find_map(automation_names)),
        Value::Array(values) => values.iter().find_map(automation_names),
        _ => None,
    }
}

/// Reads frames until the automations section lists exactly `names`.
async fn listed(socket: &mut Socket, names: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = tokio::time::timeout(left, socket.next())
            .await
            .unwrap_or_else(|_| panic!("the section never listed {names:?}"))
            .expect("the connection closed")
            .unwrap();
        let Message::Text(text) = frame else { continue };
        let frame: Value = serde_json::from_str(&text).unwrap();
        if automation_names(&frame).is_some_and(|listed| listed == names) {
            return;
        }
    }
}

async fn serving(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_automation_that_is_on_keeps_an_idle_daemon_until_none_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(dir.path())).await.unwrap();

    let mut screen = window(&running).await;
    send(
        &mut screen,
        "automation_add",
        json!({
            "name": "Nightly", "task": "Sort the new issues",
            "schedule": {"kind": "daily", "time": "03:00"}, "agent": {"kind": "claude"},
        }),
    )
    .await;
    listed(&mut screen, &["Nightly"]).await;
    screen.close(None).await.unwrap();
    drop(screen);

    // Past the idle window with no screen and no pane: held by the
    // automation that is on.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        serving(running.port).await,
        "the daemon ended with an automation on"
    );

    let mut screen = window(&running).await;
    // An automation is named by its id or its name.
    send(&mut screen, "automation_remove", json!({"id": "Nightly"})).await;
    listed(&mut screen, &[]).await;
    screen.close(None).await.unwrap();
    drop(screen);

    let deadline = Instant::now() + Duration::from_secs(15);
    while serving(running.port).await {
        assert!(
            Instant::now() < deadline,
            "the daemon outlived its idle window with no automation on"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
