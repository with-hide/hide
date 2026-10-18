//! hided against the pinned Herdr itself, on the operating system the test
//! runs on: the daemon starts, answers `/health`, opens a Workspace through
//! the shell's own `/ws` events, and a line typed into its pane comes back on
//! the terminal stream. The pane is attached the way the shell attaches it,
//! through `herdr terminal session control`.
//!
//! It needs the pinned binary, so it is ignored by default; the Windows check
//! runs it with `--ignored` and names the binary in `HIDE_E2E_HERDR_BIN`.
//! The Herdr server it starts is private: its own socket, session, config and
//! home, none of the operator's.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hide_platform::process::OwnedChild;
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

use crate::private_herdr::{PrivateHerdr, herdr_command};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Runs the control session the core runs for `pane` once more, by hand,
/// and says what it answered: only a failure calls it, so the exact command,
/// exit and output are in the report.
fn control_attempt(herdr: &PrivateHerdr, pane: &str) -> String {
    let arguments = [
        "terminal", "session", "control", pane, "--cols", "100", "--rows", "30",
    ];
    let mut command = herdr_command(&herdr.bin, &herdr.home, &herdr.socket, &herdr.config);
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match OwnedChild::spawn(&mut command) {
        Ok(child) => child,
        Err(error) => return format!("`herdr {}` did not start: {error}", arguments.join(" ")),
    };
    // Given ten seconds to answer, then ended, so a session that only waits
    // still reports what it wrote.
    let (ended, stdout, stderr) =
        match child.capture_until(Instant::now() + Duration::from_secs(10), 64 * 1024) {
            Ok(output) => (
                format!("exited {:?}", output.status.code()),
                output.stdout,
                output.stderr,
            ),
            Err(error) => (format!("was ended ({error})"), error.stdout, error.stderr),
        };
    let stdout = String::from_utf8_lossy(&stdout);
    format!(
        "`herdr {}` {ended}; stdout: {:?}; stderr: {:?}",
        arguments.join(" "),
        &stdout[..stdout.len().min(2000)],
        String::from_utf8_lossy(&stderr)
    )
}

fn daemon_env(root: &Path, herdr: &PrivateHerdr) -> Env {
    Env {
        home: herdr.home.clone(),
        herdr_socket_path: Some(herdr.socket.display().to_string()),
        herdr_bin_path: Some(herdr.bin.clone()),
        login_shell: None,
        state_dir: root.join("state"),
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
        // A missing path: this test reaches no Tailscale.
        tailscale_bin: Some(root.join("no-tailscale")),
        search_path: None,
    }
}

async fn health(port: u16) -> Value {
    use http_body_util::{BodyExt, Empty};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<hyper::body::Bytes>>();
    let response = client
        .get(format!("http://127.0.0.1:{port}/health").parse().unwrap())
        .await
        .expect("GET /health");
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

async fn connect(port: u16, token: &str) -> Socket {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(ORIGIN, format!("http://127.0.0.1:{port}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect");
    socket
        .send(text(json!({"token": token, "schema_version": 2})))
        .await
        .unwrap();
    socket
}

fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

async fn send(socket: &mut Socket, kind: &str, payload: Value) {
    socket
        .send(text(
            json!({"schema_version": 2, "kind": kind, "payload": payload}),
        ))
        .await
        .unwrap();
}

/// The next state frame, or `None` once the deadline passed.
async fn next_frame(socket: &mut Socket, deadline: Instant) -> Option<Value> {
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let message = tokio::time::timeout(left, socket.next()).await.ok()??;
        if let Message::Text(text) = message.expect("the socket stays open") {
            return Some(serde_json::from_str(&text).unwrap());
        }
    }
}

/// Keep a bounded record of state changes without another API request in the
/// input/echo path. A missing pane and a pane leaving its checkout have the
/// same empty terminal symptom, but different lifetime boundaries.
fn record_state(frame: &Value, states: &mut Vec<Value>) {
    let Some(rest) = frame["payload"]["rest"].as_object() else {
        return;
    };
    let state = json!({
        "focused": rest.get("focused"),
        "navigator": rest.get("navigator"),
        "terminal": rest.get("terminal"),
        "diagnostics": rest.get("status").map(|status| &status["diagnostics"]),
    });
    if states.last() != Some(&state) {
        if states.len() == 8 {
            // Preserve the initial selection alongside the latest changes.
            states.remove(1);
        }
        states.push(state);
    }
}

fn failure_snapshot(herdr: &PrivateHerdr) -> String {
    match hide_herdr_client::request_small_response(
        &hide_herdr_client::LocalSocketConnector::new(&herdr.socket),
        "session.snapshot",
        json!({}),
        Duration::from_secs(5),
    ) {
        Ok(response) => {
            let snapshot = &response["snapshot"];
            let panes = snapshot["panes"].as_array().map(|panes| {
                panes
                    .iter()
                    .map(|pane| {
                        json!({
                            "pane_id": pane["pane_id"],
                            "workspace_id": pane["workspace_id"],
                            "tab_id": pane["tab_id"],
                            "cwd": pane["cwd"],
                            "foreground_cwd": pane["foreground_cwd"],
                        })
                    })
                    .collect::<Vec<_>>()
            });
            json!({
                "focused_pane_id": snapshot["focused_pane_id"],
                "focused_workspace_id": snapshot["focused_workspace_id"],
                "panes": panes,
                "layouts": snapshot["layouts"],
            })
            .to_string()
        }
        Err(error) => format!("session.snapshot failed: {error}"),
    }
}

/// Every `{pane_id, bytes_base64}` chunk anywhere in a frame, decoded.
fn chunks(value: &Value, out: &mut Vec<(String, Vec<u8>)>) {
    match value {
        Value::Object(map) => {
            if let (Some(Value::String(pane)), Some(Value::String(bytes))) =
                (map.get("pane_id"), map.get("bytes_base64"))
            {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .unwrap();
                out.push((pane.clone(), bytes));
            }
            map.values().for_each(|value| chunks(value, out));
        }
        Value::Array(items) => items.iter().for_each(|value| chunks(value, out)),
        _ => {}
    }
}

/// The screen's text without its control sequences.
fn plain(screen: &str) -> String {
    let mut text = String::new();
    let mut chars = screen.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !c.is_control() || c == '\n' {
                text.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters, then one final byte in @..~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ST.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    text
}

async fn assert_private_pane_input() -> tempfile::TempDir {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    // A Unix socket path is limited to about a hundred bytes.
    let root = if cfg!(unix) {
        tempfile::Builder::new().prefix("hh").tempdir_in("/tmp")
    } else {
        tempfile::Builder::new().prefix("hh").tempdir()
    }
    .unwrap();
    let mut herdr = PrivateHerdr::start(bin, root.path());
    let running = hided::start_daemon(daemon_env(root.path(), &herdr))
        .await
        .expect("hided starts");

    let health = health(running.port).await;
    eprintln!("health: {health}");
    assert_eq!(health["schema_version"], 2);

    let project = herdr.home.join("proof");
    std::fs::create_dir_all(&project).unwrap();
    let mut socket = connect(running.port, &running.token).await;
    send(
        &mut socket,
        "create_workspace",
        json!({"path": project.display().to_string(), "label": "proof", "initialize_git": false}),
    )
    .await;
    // The Workspace's first pane, as the shell finds it: the focused one.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut states = Vec::new();
    let pane = loop {
        let frame = next_frame(&mut socket, deadline)
            .await
            .expect("the new Workspace's pane is focused within a minute");
        if let Some(pane) = frame["payload"]["rest"]["focused"]["pane_id"].as_str() {
            record_state(&frame, &mut states);
            break pane.to_owned();
        }
    };
    eprintln!("pane: {pane}");
    // A pane is attached once its view reports a size, as the shell's
    // terminal reports it when it first lays out.
    for kind in ["terminal_viewport", "terminal_resize"] {
        send(
            &mut socket,
            kind,
            json!({"pane_id": pane, "cols": 100, "rows": 30, "new_view": true}),
        )
        .await;
    }
    let mut screen = String::new();
    let mut typed = false;
    let deadline = Instant::now() + Duration::from_secs(90);
    // Typed as `"ok"` in quotes, which the shell (a POSIX shell, or
    // PowerShell on Windows) drops, so the joined word on the screen is the
    // shell's answer and not the echo of the keys.
    let found = loop {
        let Some(frame) = next_frame(&mut socket, deadline).await else {
            break false;
        };
        record_state(&frame, &mut states);
        let mut out = Vec::new();
        chunks(&frame, &mut out);
        for (from, bytes) in out {
            if from == pane {
                screen.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        if !typed && !plain(&screen).trim().is_empty() {
            // The prompt has drawn; give the shell a moment to read keys.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let line = base64::engine::general_purpose::STANDARD.encode("echo hide-proof-\"ok\"\r");
            send(
                &mut socket,
                "key",
                json!({"pane_id": pane, "bytes_base64": line}),
            )
            .await;
            typed = true;
            screen.clear();
        }
        if typed && plain(&screen).contains("hide-proof-ok") {
            eprintln!("screen: {:?}", plain(&screen));
            break true;
        }
    };
    eprintln!("pane lifecycle states: {states:?}");
    assert!(
        found,
        "the typed line never came back on the terminal stream (typed: {typed}); the screen read: {:?}; states: {states:?}; snapshot: {}; {}; herdr: {}",
        plain(&screen),
        failure_snapshot(&herdr),
        control_attempt(&herdr, &pane),
        std::fs::read_to_string(&herdr.log).unwrap_or_default()
    );
    // The core opened its letters' ledger on this system: startup writes it
    // back through the durable folder chain, and refuses it otherwise.
    let state = root.path().join("state");
    let ledger = std::fs::read(hide_kit::layout::delivery_ledger(&state))
        .expect("the daemon saved its delivery ledger at startup");
    serde_json::from_slice::<Value>(&ledger).expect("the saved ledger is JSON");
    let log = std::fs::read_to_string(state.join("Logs/core.jsonl")).unwrap_or_default();
    assert!(
        !log.contains("\"ledger.load_failed\""),
        "the daemon refused its ledger: {log}"
    );
    drop(socket);
    running.stop();
    drop(running);
    herdr.stop().expect("the private Herdr server exits");
    root
}

#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn hided_opens_a_pane_on_the_pinned_herdr_and_a_typed_line_echoes() {
    // Independent servers exercise startup ordering. An assertion failure
    // stops the test immediately; a later instance cannot erase its result.
    for instance in 1..=3 {
        eprintln!("private pane/input instance {instance}/3");
        let fixture_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("the private daemon runtime starts");
        let root = fixture_runtime.block_on(assert_private_pane_input());
        // End resident daemon tasks and their core before removing the
        // fixture, so none can publish into the next instance's diagnostics.
        drop(fixture_runtime);
        root.close().expect("the private fixture is removed");
    }
}
