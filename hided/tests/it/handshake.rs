//! The daemon end to end: its state file, the WebSocket handshake, the
//! registration and Explorer lines, against a running hided.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hided::env::Env;
use hided::state_file;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

fn test_env(keep_alive: bool) -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().unwrap();
    let env = Env {
        home: dir.path().to_path_buf(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        login_shell: None,
        state_dir: dir.path().to_path_buf(),
        legacy_state_dir: None,
        account_state_folder: false,
        keep_alive,
        vite_origin: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        idle_secs: 600,
        build: None,
        starter_program: None,
        open_command: None,
        host_helper_root: None,
        host_cli_dir: None,
        pane_id: None,
        // A missing path: no test here reaches a real Tailscale.
        tailscale_bin: Some(dir.path().join("no-tailscale")),
        search_path: None,
    };
    (dir, env)
}

async fn start() -> (tempfile::TempDir, hided::RunningDaemon) {
    let (dir, env) = test_env(true);
    let running = hided::start_daemon(env).await.expect("start daemon");
    (dir, running)
}

async fn connect(
    port: u16,
    origin: Option<&str>,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    let origin = origin
        .map(str::to_owned)
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    request
        .headers_mut()
        .insert(ORIGIN, origin.parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect");
    socket
}

fn handshake(token: &str, schema: u32) -> Message {
    Message::Text(
        json!({"token": token, "schema_version": schema})
            .to_string()
            .into(),
    )
}

#[tokio::test]
async fn health_and_unknown_http_do_not_dispatch() {
    let (_dir, running) = start().await;
    let url = format!("http://127.0.0.1:{}/health", running.port);
    let body = reqwest_get(&url).await;
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["schema_version"], 2);
    let post = reqwest_post(&format!("http://127.0.0.1:{}/dispatch", running.port)).await;
    assert_eq!(post, 404, "unknown HTTP paths must be 404");
    let missing = reqwest_status(&format!("http://127.0.0.1:{}/nope", running.port)).await;
    assert_eq!(missing, 404);
    let health_after = reqwest_get(&url).await;
    let after: Value = serde_json::from_str(&health_after).unwrap();
    assert_eq!(after["clients"], 0);
    running.stop();
}

#[tokio::test]
async fn state_file_is_private() {
    let (dir, running) = start().await;
    let path = state_file::state_path(dir.path());
    assert!(hide_platform::fs::private::is_private(&path).unwrap());
    running.stop();
}

/// The `hide` CLI reaches the daemon's pane bootstrap and the daemon itself
/// answers: this test process names no pane and sits in no registered
/// checkout, so the answer is a refusal, and never `hide_unavailable`, which
/// is what a caller that could not reach the listener says.
#[tokio::test]
async fn the_pane_bootstrap_answers_a_caller_on_the_local_stream() {
    let (dir, mut env) = test_env(true);
    let running = hided::start_daemon(env.clone())
        .await
        .expect("start daemon");
    env.pane_id = None;
    let answer = tokio::task::spawn_blocking(move || hided::workspace_cli::bootstrap(&env, true))
        .await
        .unwrap();
    // Unix reads the caller's working directory and finds no checkout there;
    // Windows cannot read it, so the caller is unavailable to bind.
    let expected = if cfg!(unix) {
        "checkout_not_registered"
    } else {
        "caller_unavailable"
    };
    assert_eq!(answer.unwrap_err(), expected);
    running.stop();
    drop(dir);
}

/// A command whose `HIDE_CAP_REF` names a reference the daemon no longer
/// holds answers what a bare `hide` answers, because it runs on a bare
/// bootstrap: the daemon removed the file of a reference that expired; a
/// daemon that crashed left one pointing at a port nothing listens on; and
/// one a revoked or restarted daemon on the same port never issued. This
/// process sits in no checkout the daemon can reach, so the bare answer is
/// a refusal, which also shows the fallback lets in nobody a bare command
/// would not.
#[tokio::test]
async fn a_command_whose_reference_is_gone_answers_as_a_bare_command() {
    let (dir, mut env) = test_env(true);
    let running = hided::start_daemon(env.clone())
        .await
        .expect("start daemon");
    env.pane_id = None;
    let expired = dir.path().join("expired.json");
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let crashed = dir.path().join("crashed.json");
    let unknown = dir.path().join("unknown.json");
    for (path, port) in [(&crashed, closed_port), (&unknown, running.port)] {
        let mut file = hide_platform::fs::private::create_new_file(path).unwrap();
        std::io::Write::write_all(
            &mut file,
            json!({"token": "ab".repeat(32), "port": port, "origin_port": port})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    }
    let (bare, answers) = tokio::task::spawn_blocking(move || {
        let bare = hided::workspace_cli::bootstrap(&env, true).unwrap_err();
        let answers = [expired, crashed, unknown].map(|path| {
            let mut credential = hided::workspace_cli::Credential::named(&env, path);
            hided::workspace_cli::request(&mut credential, "info")
        });
        (bare, answers)
    })
    .await
    .unwrap();
    assert_ne!(bare, "credential_expired");
    for answer in answers {
        assert_eq!(answer.unwrap_err(), bare);
    }
    running.stop();
    drop(dir);
}

#[tokio::test]
async fn invalid_token_is_refused() {
    let (_dir, running) = start().await;
    let mut socket = connect(running.port, None).await;
    socket.send(handshake("nope", 2)).await.unwrap();
    let close = wait_close(&mut socket).await;
    assert_eq!(close, Some(4001));
    running.stop();
}

/// A command sends its request right behind its handshake, before the
/// daemon has answered. A refusal of that handshake still reaches the
/// client, and the connection then ends rather than resets: a reset can
/// discard the close frame before the client reads it, which Windows does,
/// and the command would report a lost daemon instead (issue 785).
#[tokio::test]
async fn a_refused_client_that_already_sent_its_request_reads_the_refusal() {
    let (_dir, running) = start().await;
    for _ in 0..10 {
        let mut socket = connect(running.port, None).await;
        socket.send(handshake(&"ab".repeat(32), 2)).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type": "workspace_query", "request_id": "r", "query": "info"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        match socket.next().await {
            Some(Ok(Message::Close(Some(frame)))) => assert_eq!(u16::from(frame.code), 4001),
            other => panic!("expected the refusal, read {other:?}"),
        }
        // Reading on sends the client's own close; the daemon waited for it.
        assert!(
            socket.next().await.is_none(),
            "the connection ends after the closing handshake"
        );
    }
    running.stop();
}

#[tokio::test]
async fn schema_mismatch_is_refused() {
    let (_dir, running) = start().await;
    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 1)).await.unwrap();
    let close = wait_close(&mut socket).await;
    assert_eq!(close, Some(4003));
    running.stop();
}

#[tokio::test]
async fn origin_not_allowed_is_refused() {
    let (_dir, running) = start().await;
    let mut socket = connect(running.port, Some("http://example.com")).await;
    let close = wait_close(&mut socket).await;
    assert_eq!(close, Some(4002));
    running.stop();
}

#[tokio::test]
async fn traversal_of_the_ui_dir_is_404() {
    let (dir, running) = start().await;
    let code = reqwest_status(&format!(
        "http://127.0.0.1:{}/assets/../../hided.json",
        running.port
    ))
    .await;
    assert_eq!(code, 404);
    // The state folder's own path after the climb; a URI spells it with `/`.
    let escaped = reqwest_status(&format!(
        "http://127.0.0.1:{}/assets/../../{}",
        running.port,
        dir.path()
            .join("hided.json")
            .display()
            .to_string()
            .replace('\\', "/")
            .trim_start_matches('/')
    ))
    .await;
    assert_eq!(escaped, 404);
    running.stop();
}

#[tokio::test]
async fn ninth_client_is_refused() {
    let (_dir, running) = start().await;
    let mut held = Vec::new();
    for _ in 0..8 {
        let mut socket = connect(running.port, None).await;
        socket.send(handshake(&running.token, 2)).await.unwrap();
        let _ = socket.next().await;
        held.push(socket);
    }
    let mut extra = connect(running.port, None).await;
    extra.send(handshake(&running.token, 2)).await.unwrap();
    let close = wait_close(&mut extra).await;
    assert_eq!(close, Some(4004));
    running.stop();
}

#[tokio::test]
async fn valid_handshake_receives_snapshot() {
    let (_dir, running) = start().await;
    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 2)).await.unwrap();
    let value = first_frame(&mut socket).await;
    assert_eq!(value["type"], "snapshot");
    assert_eq!(value["payload"]["schema_version"], 2);
    running.stop();
}

#[tokio::test]
async fn semantic_snapshot_negotiation_keeps_legacy_complete_and_refuses_unknown_formats() {
    let (_dir, running) = start().await;
    let mut legacy = connect(running.port, None).await;
    legacy.send(handshake(&running.token, 2)).await.unwrap();
    let initial = first_frame(&mut legacy).await;
    assert!(initial["payload"].get("rest_format").is_none());
    let mut modern = connect(running.port, None).await;
    modern
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2,
        "snapshot_format": "semantic-rest-v1"})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let full = first_frame(&mut modern).await;
    assert_eq!(full["type"], "snapshot");
    assert_eq!(full["payload"]["rest_format"], "semantic-rest-v1");
    assert!(full["payload"]["rest"]["navigator"]["workspaces"].is_array());
    assert!(
        full["payload"]["rest"]
            .get("workspace_view")
            .unwrap()
            .is_null()
    );
    let mut ui = full["payload"]["rest"]["ui_state"].clone();
    ui["sidebar_width"] = json!(401);
    modern
        .send(Message::Text(
            json!({"schema_version": 2, "kind": "ui_state_update",
        "payload": ui})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    for (socket, partial) in [(&mut modern, true), (&mut legacy, false)] {
        let delta = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let frame = first_frame(socket).await;
                if frame["payload"]["rest"]["ui_state"]["sidebar_width"] == 401 {
                    break frame;
                }
            }
        })
        .await
        .expect("the UI update reaches both independent readers");
        assert_eq!(delta["type"], "delta");
        if partial {
            assert!(delta["payload"]["rest"].get("navigator").is_none());
        } else {
            assert!(delta["payload"]["rest"]["navigator"]["workspaces"].is_array());
        }
    }
    let mut unknown = connect(running.port, None).await;
    unknown
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2,
        "snapshot_format": "future-format"})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(wait_close(&mut unknown).await, Some(4003));
    running.stop();
}

/// Settings reads the daemon from this frame: what it is, where its state
/// lives and which Herdr it attaches with. The token never travels back.
#[tokio::test]
async fn the_daemon_describes_itself_before_the_first_snapshot() {
    let (dir, running) = start().await;
    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 2)).await.unwrap();
    let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
        panic!("expected a text frame");
    };
    let daemon: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(daemon["type"], "daemon");
    assert_eq!(daemon["payload"]["schema_version"], 2);
    assert_eq!(daemon["payload"]["pid"], std::process::id());
    let host_id = std::fs::read_to_string(dir.path().join("host-id")).unwrap();
    assert_eq!(daemon["payload"]["host_id"], host_id.trim());
    // Settings names this machine as the owner of what the daemon stores (S5.5 B35).
    let host_name = daemon["payload"]["host_name"]
        .as_str()
        .expect("the daemon names its machine");
    assert!(!host_name.is_empty());
    assert_eq!(
        daemon["payload"]["core_state_path"],
        dir.path().join("core-state.json").display().to_string()
    );
    assert!(!text.contains(&running.token), "the token is never echoed");
    assert_eq!(first_frame(&mut socket).await["type"], "snapshot");
    running.stop();
}

fn handshake_from(token: &str, have_revision: u64, have_terminal_sequence: u64) -> Message {
    Message::Text(
        json!({
            "token": token,
            "schema_version": 2,
            "have_revision": have_revision,
            "have_terminal_sequence": have_terminal_sequence,
        })
        .to_string()
        .into(),
    )
}

async fn first_frame(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    loop {
        let message = socket.next().await.unwrap().unwrap();
        let Message::Text(text) = message else {
            panic!("expected a text frame");
        };
        let frame: Value = serde_json::from_str(&text).unwrap();
        // The daemon's own description precedes the first state frame; the
        // state tests read past it and `the_daemon_describes_itself_*` checks it.
        if frame["type"] != "daemon" {
            return frame;
        }
    }
}

/// The newest revision a client holds: state frames already waiting are
/// applied, and the one at or past `at_least` is waited for.
async fn applied(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut revision: u64,
    at_least: u64,
) -> u64 {
    loop {
        let waiting = if revision < at_least {
            Some(first_frame(socket).await)
        } else {
            tokio::time::timeout(Duration::ZERO, first_frame(socket))
                .await
                .ok()
        };
        let Some(frame) = waiting else {
            return revision;
        };
        if let Some(at) = frame["payload"]["revision"].as_u64() {
            revision = at;
        }
    }
}

#[tokio::test]
async fn a_reconnect_resumes_from_the_client_cursor() {
    let (_dir, running) = start().await;
    let mut fresh = connect(running.port, None).await;
    fresh.send(handshake(&running.token, 2)).await.unwrap();
    let full = first_frame(&mut fresh).await;
    assert_eq!(full["type"], "snapshot");
    assert!(
        full["payload"]["rest"].is_object(),
        "a snapshot carries rest"
    );
    let mut cursor = full["payload"]["revision"].as_u64().unwrap();
    // Terminal output and its cursor ride `terminal` frames of their own
    // (PRD core-host-node-terminal D-15); a snapshot carries neither, and
    // this client was sent none, so its terminal cursor is still 0.
    for field in [
        "terminal_sequence",
        "chunks",
        "chunks_dropped",
        "input_generation",
    ] {
        assert!(
            full["payload"].get(field).is_none(),
            "{field} left the snapshot"
        );
    }
    let sequence = 0;

    // Same cursor the first client applied: nothing changed, so a delta
    // without the rest section, not a second full snapshot. A starting
    // daemon still publishes what its workers find, so a resume that
    // crossed one of those resumes again from the cursor the first client
    // applied with it.
    let revision = loop {
        let mut resumed = connect(running.port, None).await;
        resumed
            .send(handshake_from(&running.token, cursor, sequence))
            .await
            .unwrap();
        let delta = first_frame(&mut resumed).await;
        assert_eq!(
            delta["type"], "delta",
            "a known cursor resumes with a delta"
        );
        let current = delta["payload"]["revision"].as_u64().unwrap();
        assert!(current >= cursor, "a delta never goes back");
        if current == cursor {
            assert!(
                delta["payload"]["rest"].is_null(),
                "a delta at the current revision has no rest"
            );
            break current;
        }
        cursor = applied(&mut fresh, cursor, current).await;
    };

    // A cursor ahead of the daemon (it restarted): the client state is not one
    // a delta applies to, so it gets a self-contained snapshot again.
    let mut ahead = connect(running.port, None).await;
    ahead
        .send(handshake_from(&running.token, revision + 1000, sequence))
        .await
        .unwrap();
    let resync = first_frame(&mut ahead).await;
    assert_eq!(resync["type"], "snapshot");
    assert!(resync["payload"]["rest"].is_object());
    let at = resync["payload"]["revision"].as_u64().unwrap();
    assert!(
        (revision..revision + 1000).contains(&at),
        "the snapshot is the daemon's state, not the client's cursor"
    );
    running.stop();
}

/// A revision counts only on the core that numbered it (PRD
/// core-host-node-move Q12): a page that names another core, which a move,
/// a rollback or an update replaced, is drawn whole even when its revision
/// is one this core has.
#[tokio::test]
async fn a_revision_from_another_core_is_answered_with_a_whole_snapshot() {
    let (_dir, running) = start().await;
    let mut fresh = connect(running.port, None).await;
    fresh.send(handshake(&running.token, 2)).await.unwrap();
    let Some(Ok(Message::Text(daemon))) = fresh.next().await else {
        panic!("expected the daemon frame");
    };
    let daemon: Value = serde_json::from_str(&daemon).unwrap();
    let own = daemon["payload"]["core_instance"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(own.len(), 16);
    let mut cursor = first_frame(&mut fresh).await["payload"]["revision"]
        .as_u64()
        .unwrap();
    let resume = async |core: &str, cursor: u64| {
        let mut socket = connect(running.port, None).await;
        let mut handshake =
            json!({"token": running.token, "schema_version": 2, "have_revision": cursor});
        handshake["have_core"] = json!(core);
        socket
            .send(Message::Text(handshake.to_string().into()))
            .await
            .unwrap();
        first_frame(&mut socket).await
    };
    // A starting daemon still publishes what its workers find, so the
    // positive control waits for a quiet cursor first.
    loop {
        let delta = resume(&own, cursor).await;
        assert_eq!(delta["type"], "delta", "this core's own cursor resumes");
        let current = delta["payload"]["revision"].as_u64().unwrap();
        if current == cursor {
            break;
        }
        cursor = applied(&mut fresh, cursor, current).await;
    }
    let other = resume("0123456789abcdef", cursor).await;
    assert_eq!(other["type"], "snapshot");
    assert!(other["payload"]["rest"].is_object());
    running.stop();
}

#[tokio::test]
async fn the_daemon_exits_after_its_last_client_leaves() {
    let (dir, mut env) = test_env(false);
    env.idle_secs = 1;
    let running = std::sync::Arc::new(hided::start_daemon(env).await.expect("start daemon"));
    let state_path = state_file::state_path(dir.path());
    assert!(state_path.exists());
    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 2)).await.unwrap();
    let _ = first_frame(&mut socket).await;
    // Held past the idle window: a connected client keeps the daemon up.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        state_path.exists(),
        "a connected client keeps the daemon alive"
    );
    // Registered before the client leaves so the announcement cannot be missed.
    let announced = tokio::spawn({
        let running = std::sync::Arc::clone(&running);
        async move { hided::wait_shutdown(&running).await }
    });
    socket.close(None).await.unwrap();
    drop(socket);
    let stopped = tokio::time::timeout(Duration::from_secs(5), announced).await;
    assert!(
        stopped.is_ok(),
        "the daemon did not announce shutdown after its last client left"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let listening = tokio::net::TcpStream::connect(("127.0.0.1", running.port))
            .await
            .is_ok();
        if !listening && !state_path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the listener or the state file outlived the daemon");
}

async fn wait_close(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Option<u16> {
    tokio::time::timeout(Duration::from_secs(2), async {
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

/// One request's status and body. The path is sent as written, `..`
/// included, as `curl --path-as-is` would, and no test needs a `curl` on the
/// runner.
async fn request(method: &str, url: &str) -> (u16, String) {
    use http_body_util::{BodyExt, Empty};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<hyper::body::Bytes>>();
    let request = hyper::Request::builder()
        .method(method)
        .uri(url)
        .body(Empty::new())
        .unwrap();
    let response = client.request(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn reqwest_get(url: &str) -> String {
    let (status, body) = request("GET", url).await;
    assert_eq!(status, 200, "{url}: {body}");
    body
}

async fn reqwest_post(url: &str) -> u16 {
    request("POST", url).await.0
}

async fn reqwest_status(url: &str) -> u16 {
    request("GET", url).await.0
}

// --- $HOME boundary over the socket (PRD S2 B10) -------------------------

async fn live_socket(
    running: &hided::RunningDaemon,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 2)).await.unwrap();
    let first = first_frame(&mut socket).await;
    assert_eq!(first["type"], "snapshot");
    socket
}

async fn send_event(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    kind: &str,
    payload: Value,
) -> Value {
    socket
        .send(Message::Text(
            json!({"schema_version": 2, "kind": kind, "payload": payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let frame = first_frame(socket).await;
        // Snapshot deltas and terminal frames keep flowing on the same
        // socket; the reply to a boundary event is the first frame that is
        // not one of them.
        if !["snapshot", "delta", "terminal"].contains(&frame["type"].as_str().unwrap_or("")) {
            return frame;
        }
    }
}

// The escapes are Unix symbolic links. A Windows folder link is a junction,
// which the boundary's final-path check judges; that is not proved here.
#[cfg(unix)]
#[tokio::test]
async fn registration_refusals_are_answered_by_hided() {
    let (dir, running) = start().await;
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    std::fs::create_dir_all(home.join("projects/alpha")).unwrap();
    std::fs::create_dir_all(home.join("projects/.hidden")).unwrap();
    std::fs::write(home.join("projects/file.txt"), "x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), home.join("projects/escape")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("nope"), home.join("projects/dangling"))
        .unwrap();
    let mut socket = live_socket(&running).await;

    let cases = [
        (wire(&home.join("projects/escape")), "outside_home"),
        (wire(&home.join("projects/escape/nope")), "outside_home"),
        (wire(&home.join("projects/dangling")), "outside_home"),
        (wire(&home.join("projects/dangling/child")), "outside_home"),
        (format!("{}/projects/../..", home.display()), "invalid_path"),
        (
            format!("{}/projects/%2e%2e/%2e%2e", home.display()),
            "not_found",
        ),
        (wire(&home.join("projects/file.txt")), "not_a_directory"),
        // Written outside home, a folder asks for the operator's
        // confirmation before anything there is read (PRD wsl-device B15).
        (wire(outside.path()), "needs_confirmation"),
        (wire(&outside.path().join("nope")), "needs_confirmation"),
        (wire(&home), "home_root"),
        ("relative/path".to_owned(), "invalid_path"),
    ];
    for (path, reason) in cases {
        let refused = send_event(
            &mut socket,
            "create_workspace",
            json!({"path": path, "label": "x", "initialize_git": false}),
        )
        .await;
        assert_eq!(refused["type"], "path_refused", "{path}");
        assert_eq!(refused["payload"]["reason"], reason, "{path}");
        assert_eq!(refused["payload"]["kind"], "create_workspace");
    }
    // A confirmation names one path: another path's is no confirmation, and
    // a confirmed path is then answered by what is there (B18). A confirmed
    // symlink out of home stays refused.
    let elsewhere = wire(&outside.path().join("elsewhere"));
    let missing = wire(&outside.path().join("nope"));
    let escape = wire(&home.join("projects/escape"));
    for (path, confirmed, reason) in [
        (
            wire(outside.path()),
            elsewhere.clone(),
            "needs_confirmation",
        ),
        (missing.clone(), missing.clone(), "not_found"),
        (escape.clone(), escape.clone(), "outside_home"),
    ] {
        let refused = send_event(
            &mut socket,
            "create_workspace",
            json!({"path": path, "label": "x", "initialize_git": false,
                "confirmed_outside_home": confirmed}),
        )
        .await;
        assert_eq!(refused["type"], "path_refused", "{path}");
        assert_eq!(refused["payload"]["reason"], reason, "{path}");
    }
    running.stop();
}

#[tokio::test]
async fn a_remote_listing_is_forwarded_untouched() {
    let (_dir, running) = start().await;
    let outside = tempfile::tempdir().unwrap();
    let mut socket = live_socket(&running).await;
    // The path is on a remote machine; locally it is outside home, so a
    // boundary that wrongly ran would answer `path_refused`. The core has no
    // such target and records that instead, which shows the event arrived.
    // `local` is no exception: hided lists no folder of this machine for
    // registration, since Add a project picks it with the native picker.
    for target in ["mini", "local"] {
        socket
            .send(Message::Text(
                json!({
                    "schema_version": 2,
                    "kind": "remote_file_list",
                    "payload": {"target_id": target, "root_path": wire(outside.path())},
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let reaction = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let frame = first_frame(&mut socket).await;
                assert_ne!(
                    frame["type"], "path_refused",
                    "a listing must not meet the local boundary"
                );
                assert_ne!(
                    frame["type"], "directory_list",
                    "hided lists nothing for {target}"
                );
                if frame
                    .to_string()
                    .contains(&format!("unconfigured target {target}"))
                {
                    return frame;
                }
            }
        })
        .await
        .expect("the core's reaction to the forwarded event");
        assert!(matches!(
            reaction["type"].as_str(),
            Some("delta" | "snapshot")
        ));
    }
    running.stop();
}

// --- checkout-root boundary over the socket (PRD S3 B7) ------------------

/// A client socket, as `connect` builds it.
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Sends an event and returns the first frame that answers it: a refusal from
/// hided, or the core's own reaction to a forwarded event. `answered` names
/// the reaction, so a delta the core emitted for something else does not count
/// as the answer.
async fn send_event_expecting(
    socket: &mut Socket,
    kind: &str,
    payload: Value,
    answered: impl Fn(&Value) -> bool,
) -> Value {
    socket
        .send(Message::Text(
            json!({"schema_version": 2, "kind": kind, "payload": payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    // What the core last said about the editor and its status, for the
    // report when no answer comes.
    let mut editor = Value::Null;
    let mut status = Value::Null;
    let answer = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = first_frame(socket).await;
            if frame["type"] == "path_refused" || answered(&frame) {
                return frame;
            }
            if !frame["payload"]["editor"].is_null() {
                editor = frame["payload"]["editor"].clone();
            }
            if !frame["payload"]["rest"]["status"].is_null() {
                status = frame["payload"]["rest"]["status"].clone();
            }
        }
    })
    .await;
    answer.unwrap_or_else(|_| {
        panic!("an answer to the {kind} event; the core's last editor: {editor}; its last status: {status}")
    })
}

/// `path` as a client spells it on the wire (`hide_platform::path`), which on
/// macOS and Linux is the path as written.
fn wire(path: &std::path::Path) -> String {
    hide_platform::path::to_wire(path).expect("a test path has a wire spelling")
}

/// The reaction predicate of a step whose only expected answer is the refusal
/// itself, which the loop returns before it asks.
fn never(_: &Value) -> bool {
    false
}

/// Writes the core's own state file with one registration, which is what makes
/// a directory a checkout and so what gives hided a root to check against. The
/// file is the shape herdr-core persists: flat, schema 1, no nesting.
fn seed_registration(state_dir: &std::path::Path, id: &str, path: &std::path::Path) {
    std::fs::write(
        state_dir.join("core-state.json"),
        json!({
            "schema_version": 1,
            "expanded_paths": [],
            "selected_path": Value::Null,
            "selected_pane_id": Value::Null,
            "workspace_registrations": [{
                "id": id,
                "label": "alpha",
                "path": wire(path),
            }],
        })
        .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn explorer_paths_are_checked_against_the_registered_checkout() {
    let (dir, env) = test_env(true);
    // The short spelling, as the daemon and a client use it; Windows writes
    // `std::fs::canonicalize`'s answer with a `\\?\` prefix after which `/`
    // is not a separator.
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects").join("alpha");
    let main = checkout.join("src").join("main.rs");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::write(&main, "fn main() {}\n").unwrap();
    // A directory under home that no registration covers: the registration
    // line would accept it, the Explorer line must not.
    let notes = home.join("projects").join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("todo.md"), "- [ ] x\n").unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");

    let mut socket = connect(running.port, None).await;
    socket.send(handshake(&running.token, 2)).await.unwrap();
    let snapshot = first_frame(&mut socket).await;
    assert_eq!(snapshot["type"], "snapshot");
    let workspace = snapshot["payload"]["rest"]["navigator"]["workspaces"]
        .as_array()
        .expect("the snapshot carries the navigator")
        .iter()
        .find(|workspace| workspace["path"] == json!(wire(&checkout)))
        .cloned()
        .expect("the seeded registration is a workspace");
    let workspace_id = workspace["id"].as_str().unwrap().to_owned();
    let checkout_id = workspace["checkouts"][0]["id"].as_str().unwrap().to_owned();
    let checkout_path = wire(&checkout);
    let notes_path = wire(&notes.join("todo.md"));
    let file_path = wire(&main);

    // A path under home that no checkout covers is refused as such, for an
    // open and for a change alike.
    let refused = send_event_expecting(
        &mut socket,
        "file_open",
        json!({
            "path": notes_path,
            "workspace_id": workspace_id,
            "checkout_id": checkout_id,
            "preview": false,
        }),
        never,
    )
    .await;
    assert_eq!(refused["type"], "path_refused");
    assert_eq!(refused["payload"]["kind"], "file_open");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");
    assert_eq!(refused["payload"]["path"], notes_path);

    let refused = send_event_expecting(
        &mut socket,
        "path_trash",
        json!({
            "root": checkout_path,
            "path": notes_path,
            "select_after": checkout_path,
            "inode": Value::Null,
        }),
        never,
    )
    .await;
    assert_eq!(refused["payload"]["kind"], "path_trash");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");

    // A copy names the item and the folder it lands in on the same line a move
    // does: a folder outside every checkout is refused.
    let refused = send_event_expecting(
        &mut socket,
        "path_copy",
        json!({"root": checkout_path, "path": file_path, "destination": notes_path}),
        never,
    )
    .await;
    assert_eq!(refused["payload"]["kind"], "path_copy");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");

    // A drop from the file manager is judged on the same line, before any
    // upload is read: its destination outside every checkout is refused.
    let refused = send_event_expecting(
        &mut socket,
        "path_import",
        json!({
            "request_id": "01234567-0123-0123-0123-0123456789ab",
            "root": checkout_path,
            "destination": notes_path,
            "items": [{"path": "dir", "directory": true}],
        }),
        never,
    )
    .await;
    assert_eq!(refused["payload"]["kind"], "path_import");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");

    // The root a change names has to be one of the registered ones.
    let unregistered_root = wire(&home.join("projects"));
    let refused = send_event_expecting(
        &mut socket,
        "path_trash",
        json!({
            "root": unregistered_root,
            "path": wire(&checkout.join("src")),
            "select_after": checkout_path,
            "inode": Value::Null,
        }),
        never,
    )
    .await;
    assert_eq!(refused["payload"]["kind"], "path_trash");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");
    assert_eq!(refused["payload"]["path"], unregistered_root);

    // A name that is not one component never reaches the core.
    let refused = send_event_expecting(
        &mut socket,
        "path_rename",
        json!({"root": checkout_path, "path": file_path, "name": "../escape.rs"}),
        never,
    )
    .await;
    assert_eq!(refused["payload"]["kind"], "path_rename");
    assert_eq!(refused["payload"]["reason"], "invalid_path");
    assert_eq!(refused["payload"]["path"], "../escape.rs");

    // A path inside the registered checkout is the Explorer's: the core opens
    // it, and the tab it makes carries the file's own name.
    let opened = send_event_expecting(
        &mut socket,
        "file_open",
        json!({
            "path": file_path,
            "workspace_id": workspace_id,
            "checkout_id": checkout_id,
            "preview": false,
        }),
        |frame| frame["payload"]["editor"]["tabs"][0]["label"] == "main.rs",
    )
    .await;
    assert_eq!(opened["type"], "delta", "the open has to reach the core");
    assert_eq!(
        opened["payload"]["editor"]["tabs"][0]["label"], "main.rs",
        "the tab the core makes carries the file's own name"
    );

    running.stop();
}

// The escapes are Unix symbolic links. A Windows folder link is a junction,
// which the boundary's final-path check judges; that is not proved here.
#[cfg(unix)]
#[tokio::test]
async fn the_explorer_listing_shows_a_checkout_folder_in_the_explorer_order() {
    let (dir, env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects/alpha");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::create_dir_all(checkout.join(".github")).unwrap();
    std::fs::create_dir_all(checkout.join(".git/objects")).unwrap();
    std::fs::write(checkout.join("README.md"), "# alpha\n").unwrap();
    std::fs::write(checkout.join("file2.txt"), "x").unwrap();
    std::fs::write(checkout.join("file10.txt"), "x").unwrap();
    std::fs::write(checkout.join(".env"), "x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), checkout.join("escape")).unwrap();
    std::os::unix::fs::symlink(checkout.join("src"), checkout.join("alias")).unwrap();
    // A folder under home that no registration covers: the registration line
    // reads it, the Explorer line must not.
    let notes = home.join("projects/notes");
    std::fs::create_dir_all(&notes).unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut socket = live_socket(&running).await;
    let checkout_path = wire(&checkout);
    let notes_path = wire(&notes);

    let listing = send_event_expecting(
        &mut socket,
        "file_list",
        json!({"root": checkout_path, "path": checkout_path}),
        |frame| frame["type"] == "directory_list",
    )
    .await;
    assert_eq!(listing["type"], "directory_list");
    assert_eq!(listing["payload"]["kind"], "file_list");
    assert_eq!(listing["payload"]["root_path"], json!(checkout_path));
    assert_eq!(listing["payload"]["truncated"], false);
    let rows: Vec<(&str, bool)> = listing["payload"]["entries"]
        .as_array()
        .expect("a listing carries entries")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap(),
                entry["is_directory"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (".github", true),
            ("alias", true),
            ("src", true),
            (".env", false),
            ("file2.txt", false),
            ("file10.txt", false),
            ("README.md", false),
        ],
        "directories first and then the natural order; .git and the symlink that \
         leaves the root are not rows"
    );

    // A folder inside the root is the Explorer's at any depth.
    let nested = send_event_expecting(
        &mut socket,
        "file_list",
        json!({"root": checkout_path, "path": wire(&checkout.join("src"))}),
        |frame| frame["type"] == "directory_list",
    )
    .await;
    assert_eq!(
        nested["payload"]["root_path"],
        json!(wire(&checkout.join("src")))
    );
    assert!(nested["payload"]["entries"].as_array().unwrap().is_empty());

    // The daemon resolves an absolute in-checkout link before the core's
    // capability operation sees the path. The created file must land in the
    // linked folder, while the outside link above remains excluded.
    socket
        .send(Message::Text(
            json!({
                "schema_version": 2,
                "kind": "file_create",
                "payload": {
                    "root": checkout_path,
                    "parent": wire(&checkout.join("alias")),
                    "name": "through-link.txt"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !checkout.join("src/through-link.txt").is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("an in-checkout link allows file creation through the daemon");
    assert!(!outside.path().join("through-link.txt").exists());

    // The root the listing names has to be a registered one, and the folder has
    // to be under it: a folder under home that no checkout covers, and a
    // symlink out of the checkout, are both refused as outside_checkout.
    for (root, path) in [
        (checkout_path.clone(), notes_path.clone()),
        (notes_path.clone(), notes_path.clone()),
        (checkout_path.clone(), wire(&checkout.join("escape"))),
    ] {
        let refused = send_event_expecting(
            &mut socket,
            "file_list",
            json!({"root": root, "path": path}),
            never,
        )
        .await;
        assert_eq!(refused["type"], "path_refused", "{root} {path}");
        assert_eq!(refused["payload"]["kind"], "file_list");
        assert_eq!(refused["payload"]["reason"], "outside_checkout");
    }

    // The snapshot still registers this spelling after the directory is
    // replaced. Neither the new outside rows nor a write through it may pass.
    std::fs::write(outside.path().join("outside-secret.txt"), "secret").unwrap();
    std::fs::rename(&checkout, home.join("projects/alpha-old")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &checkout).unwrap();
    for (kind, payload) in [
        (
            "file_list",
            json!({"root": checkout_path, "path": checkout_path}),
        ),
        (
            "file_create",
            json!({"root": checkout_path, "parent": checkout_path, "name": "written.txt"}),
        ),
        (
            "file_save",
            json!({"path": wire(&checkout.join("outside-secret.txt"))}),
        ),
        (
            "file_index",
            json!({"root": checkout_path, "query": "outside"}),
        ),
    ] {
        let refused = send_event_expecting(&mut socket, kind, payload, never).await;
        assert_eq!(refused["type"], "path_refused", "{kind}");
        assert_eq!(refused["payload"]["reason"], "outside_checkout", "{kind}");
    }
    assert!(!outside.path().join("written.txt").exists());
    running.stop();
}

/// Reads the next binary frame and splits it into its header and bytes, the
/// way a viewer does: a 4-byte big-endian header length, the header JSON, then
/// the payload. Text frames (snapshot deltas) are skipped.
async fn recv_binary(socket: &mut Socket) -> (Value, Vec<u8>) {
    loop {
        let message = socket.next().await.unwrap().unwrap();
        let Message::Binary(bytes) = message else {
            continue;
        };
        let header_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let header: Value = serde_json::from_slice(&bytes[4..4 + header_len]).unwrap();
        return (header, bytes[4 + header_len..].to_vec());
    }
}

#[tokio::test]
async fn file_bytes_streams_a_checkout_file_and_refuses_a_path_outside_it() {
    let (dir, env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects/alpha");
    std::fs::create_dir_all(&checkout).unwrap();
    let contents: Vec<u8> = (0u8..64).collect();
    std::fs::write(checkout.join("blob.bin"), &contents).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.bin"), b"x").unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut socket = live_socket(&running).await;
    let path = wire(&checkout.join("blob.bin"));

    // A whole-file read: one binary frame carrying the header and every byte.
    socket
        .send(Message::Text(
            json!({
                "schema_version": 2,
                "kind": "file_bytes",
                "payload": {"request_id": "r1", "path": path, "offset": 0, "length": Value::Null},
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let (header, payload) = recv_binary(&mut socket).await;
    assert_eq!(header["type"], "file_bytes");
    assert_eq!(header["request_id"], "r1");
    assert_eq!(header["path"], json!(path));
    assert_eq!(header["offset"], 0);
    assert_eq!(header["total"], 64);
    assert_eq!(header["eof"], true);
    assert_eq!(payload, contents);

    // A range read names its own offset and returns only those bytes; the
    // total still reports the whole file, which is what a seek needs.
    socket
        .send(Message::Text(
            json!({
                "schema_version": 2,
                "kind": "file_bytes",
                "payload": {"request_id": "r2", "path": path, "offset": 16, "length": 16},
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let (header, payload) = recv_binary(&mut socket).await;
    assert_eq!(header["request_id"], "r2");
    assert_eq!(header["offset"], 16);
    assert_eq!(header["total"], 64);
    assert_eq!(header["eof"], true);
    assert_eq!(payload, contents[16..32]);

    // A file outside every registered root is the same path_refused frame the
    // rest of the boundary answers with.
    let refused = send_event_expecting(
        &mut socket,
        "file_bytes",
        json!({
            "request_id": "r3",
            "path": wire(&outside.path().join("secret.bin")),
            "offset": 0,
            "length": Value::Null,
        }),
        never,
    )
    .await;
    assert_eq!(refused["type"], "path_refused");
    assert_eq!(refused["payload"]["kind"], "file_bytes");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");

    // A directory under the root is not a byte source.
    let refused = send_event_expecting(
        &mut socket,
        "file_bytes",
        json!({
            "request_id": "r4",
            "path": wire(&checkout),
            "offset": 0,
            "length": Value::Null,
        }),
        never,
    )
    .await;
    assert_eq!(refused["type"], "path_refused");
    assert_eq!(refused["payload"]["reason"], "not_a_file");

    // A read past the cap is answered as a one-line failure, not streamed.
    let refused = send_event_expecting(
        &mut socket,
        "file_bytes",
        json!({
            "request_id": "r5",
            "path": path,
            "offset": 0,
            "length": hided::boundary::MAX_FILE_BYTES + 1,
        }),
        |frame| frame["type"] == "file_bytes_error",
    )
    .await;
    assert_eq!(refused["type"], "file_bytes_error");
    assert_eq!(refused["payload"]["reason"], "too_large");
    running.stop();
}

#[cfg(unix)]
#[tokio::test]
async fn a_fifo_byte_request_is_refused_and_another_socket_stays_responsive() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let (dir, env) = test_env(true);
    let checkout = hide_platform::fs::identity::canonical(dir.path())
        .unwrap()
        .join("projects/alpha");
    std::fs::create_dir_all(&checkout).unwrap();
    let pipe = checkout.join("pipe");
    let name = CString::new(pipe.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut socket = live_socket(&running).await;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        send_event_expecting(
            &mut socket,
            "file_bytes",
            json!({"request_id": "fifo", "path": wire(&pipe), "offset": 0, "length": Value::Null}),
            never,
        ),
    )
    .await;
    if result.is_err() {
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe);
    }
    let refused = result.expect("a FIFO must not block the WebSocket");
    assert_eq!(refused["payload"]["reason"], "not_a_file");
    let mut second = live_socket(&running).await;
    let listing = send_event_expecting(
        &mut second,
        "file_list",
        json!({"root": wire(&checkout), "path": wire(&checkout)}),
        |frame| frame["type"] == "directory_list",
    )
    .await;
    assert_eq!(listing["type"], "directory_list");
    running.stop();
}

#[tokio::test]
async fn a_change_in_a_watched_checkout_is_announced() {
    let (dir, env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects/alpha");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    // A window that draws, as the web shell names itself: the Explorer watch
    // runs only while one does.
    let mut socket = connect(running.port, None).await;
    socket
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2, "client_kind": "web"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    // The watcher arms after the daemon's first snapshot read; let it settle
    // so the change below is not missed by a watcher that was not up yet.
    tokio::time::sleep(Duration::from_millis(750)).await;
    std::fs::write(checkout.join("appeared.txt"), "x").unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = first_frame(&mut socket).await;
            if frame["type"] == "directory_changed" {
                return frame;
            }
        }
    })
    .await
    .expect("a directory_changed frame for the changed checkout");
    assert_eq!(frame["payload"]["path"], json!(wire(&checkout)));
    running.stop();
}

// The relative paths are the wire's, `/` between names on every system.
#[tokio::test]
async fn the_file_index_answers_the_ranked_matches_for_a_checkout() {
    let (dir, env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects/alpha");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::create_dir_all(checkout.join("docs")).unwrap();
    std::fs::write(checkout.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(checkout.join("docs/readme.md"), "# a\n").unwrap();
    std::fs::write(checkout.join("debug.log"), "x").unwrap();
    std::fs::write(checkout.join(".gitignore"), "*.log\n").unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut socket = live_socket(&running).await;
    let root = wire(&checkout);

    // The first query starts the walk; the daemon answers `indexing`, and the
    // next query has the list.
    let mut result = send_event_expecting(
        &mut socket,
        "file_index",
        json!({"root": root, "query": "main"}),
        |frame| frame["type"] == "file_index_result",
    )
    .await;
    for _ in 0..40 {
        if result["payload"]["indexing"] == json!(false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        result = send_event_expecting(
            &mut socket,
            "file_index",
            json!({"root": root, "query": "main"}),
            |frame| frame["type"] == "file_index_result",
        )
        .await;
    }
    assert_eq!(result["payload"]["indexing"], json!(false));
    assert_eq!(result["payload"]["truncated"], json!(false));
    let entries: Vec<&str> = result["payload"]["files"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| entry["relative_path"].as_str().unwrap())
        .collect();
    assert_eq!(
        entries,
        vec!["src/main.rs"],
        "only the match, and not the ignored file"
    );
    assert_eq!(
        result["payload"]["files"][0]["path"],
        json!(wire(&checkout.join("src/main.rs"))),
        "a palette entry carries the absolute path to open"
    );

    // A root that is not a registered checkout is refused.
    let refused = send_event_expecting(
        &mut socket,
        "file_index",
        json!({"root": wire(&home.join("projects")), "query": "x"}),
        never,
    )
    .await;
    assert_eq!(refused["type"], "path_refused");
    assert_eq!(refused["payload"]["kind"], "file_index");
    assert_eq!(refused["payload"]["reason"], "outside_checkout");
    running.stop();
}

#[tokio::test]
async fn browser_socket_cannot_start_a_host_file_handler() {
    let (dir, mut env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects/alpha");
    std::fs::create_dir_all(&checkout).unwrap();
    let file = checkout.join("notes.txt");
    std::fs::write(&file, "safe data").unwrap();
    // A helper that leaves the marker if anything ever starts it, at the
    // temporary folder's own spelling, which `cmd` writes on Windows.
    let marker = dir.path().join("handler-started");
    #[cfg(unix)]
    let opener = {
        use std::os::unix::fs::PermissionsExt;
        let opener = home.join("fake-opener");
        std::fs::write(
            &opener,
            format!("#!/bin/sh\nprintf x > '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o700)).unwrap();
        opener
    };
    #[cfg(windows)]
    let opener = {
        let opener = home.join("fake-opener.cmd");
        std::fs::write(&opener, format!("@echo x> \"{}\"\r\n", marker.display())).unwrap();
        opener
    };
    env.open_command = Some(opener);
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut socket = live_socket(&running).await;

    let answer = send_event_expecting(
        &mut socket,
        "open_external",
        json!({"path": wire(&file)}),
        |frame| frame["type"] == "open_external_result",
    )
    .await;
    assert_eq!(answer["payload"]["ok"], false);
    assert_eq!(answer["payload"]["reason"], "untrusted_client");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!marker.exists(), "browser frame started the host handler");
    running.stop();
}

#[tokio::test]
async fn a_client_cannot_send_the_shell_attachment_events() {
    let (_dir, running) = start().await;
    let mut socket = live_socket(&running).await;
    // The browser stages bytes through attachment_*; a client that sends the
    // shell's own event would name an arbitrary path for the core to read.
    for kind in [
        "terminal_attachment",
        "terminal_attachment_ready",
        "terminal_attachment_action",
    ] {
        let refused = send_event_expecting(
            &mut socket,
            kind,
            json!({
                "request_id": "01234567-0123-0123-0123-0123456789ab",
                "pane_id": "p1",
                "paths": ["/etc/hosts"],
                "error": Value::Null,
                "action": "cancel",
            }),
            |frame| frame["type"] == "error",
        )
        .await;
        assert_eq!(refused["type"], "error", "{kind}");
    }
    running.stop();
}

#[tokio::test]
async fn attachment_stages_over_the_cap_and_unknown_commits_are_refused() {
    let (_dir, running) = start().await;
    let mut socket = live_socket(&running).await;

    let refused = send_event_expecting(
        &mut socket,
        "attachment_stage",
        json!({
            "request_id": "r1",
            "name": "big.bin",
            "size": (20u64 * 1024 * 1024) + 1,
            "clipboard": false,
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["type"], "attachment_refused");
    assert_eq!(refused["payload"]["reason"], "too_large");

    // A stage id that names a path is refused before anything is written.
    let refused = send_event_expecting(
        &mut socket,
        "attachment_stage",
        json!({
            "request_id": "../escape",
            "name": "shot.png",
            "size": 1,
            "clipboard": false,
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["payload"]["reason"], "invalid_request_id");

    let refused = send_event_expecting(
        &mut socket,
        "attachment_commit",
        json!({
            "request_id": "01234567-0123-0123-0123-0123456789ab",
            "pane_id": "p1",
            "bracketed_paste": true,
            "clipboard": false,
            "stages": ["never-staged"],
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["payload"]["reason"], "unknown_stage");

    // A commit id the core would refuse is refused before any stage is consumed.
    let refused = send_event_expecting(
        &mut socket,
        "attachment_commit",
        json!({
            "request_id": "not-a-uuid",
            "pane_id": "p1",
            "bracketed_paste": true,
            "clipboard": false,
            "stages": ["never-staged"],
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["payload"]["reason"], "invalid_request_id");

    // A clipboard commit must name its own stage; a different id is refused
    // (the mode binding itself is covered by the attachments unit tests).
    let refused = send_event_expecting(
        &mut socket,
        "attachment_commit",
        json!({
            "request_id": "b2",
            "pane_id": "p1",
            "bracketed_paste": true,
            "clipboard": true,
            "stages": ["never-staged"],
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["payload"]["reason"], "invalid_request_id");

    // A clipboard commit must name the one stage that wrote its image, because
    // the core reads the file its own request id derives.
    let refused = send_event_expecting(
        &mut socket,
        "attachment_commit",
        json!({
            "request_id": "01234567-0123-0123-0123-0123456789ab",
            "pane_id": "p1",
            "bracketed_paste": true,
            "clipboard": true,
            "stages": [],
        }),
        |frame| frame["type"] == "attachment_refused",
    )
    .await;
    assert_eq!(refused["payload"]["reason"], "invalid_request_id");
    running.stop();
}

#[tokio::test]
async fn a_drop_names_only_uploads_hided_staged_and_shapes_it_can_forward() {
    let (_dir, running) = start().await;
    let mut socket = live_socket(&running).await;
    let request_id = "01234567-0123-0123-0123-0123456789ab";
    let refusal = |frame: &Value| frame["type"] == "attachment_refused";
    let drop = |items: Value| json!({"request_id": request_id, "root": "/nowhere", "destination": "/nowhere", "items": items});

    // An upload id that was never opened names no file.
    let refused = send_event_expecting(
        &mut socket,
        "path_import",
        drop(json!([{"path": "a.txt", "directory": false, "stage": "never-staged"}])),
        refusal,
    )
    .await;
    // The destination is judged first, so a daemon with no such checkout
    // refuses it; either way nothing reaches the core.
    assert!(
        refused["type"] == "path_refused" || refused["payload"]["reason"] == "unknown_stage",
        "{refused}"
    );

    // Shapes the page never sends are refused before anything else is read.
    for items in [
        json!([]),
        json!([{"path": "../escape", "directory": true}]),
        json!([{"path": "a/../b", "directory": true}]),
        json!([{"path": "", "directory": true}]),
        json!([{"path": "dir", "directory": true, "stage": "s1"}]),
        json!([{"path": "a.txt", "directory": false}]),
    ] {
        let refused =
            send_event_expecting(&mut socket, "path_import", drop(items.clone()), refusal).await;
        assert_eq!(refused["type"], "attachment_refused", "{items}");
        assert!(
            ["invalid_item", "too_many_files"]
                .contains(&refused["payload"]["reason"].as_str().unwrap()),
            "{items}: {refused}"
        );
    }
    running.stop();
}

/// A Herdr server that answers every request on `socket`, refusing all but
/// `ping`; the daemon's session sync gets nothing it can use and keeps
/// retrying, which is all this needs. It answers until dropped.
struct AnsweringHerdr {
    closer: hide_platform::ipc::ListenerCloser,
}

impl Drop for AnsweringHerdr {
    fn drop(&mut self) {
        self.closer.close();
    }
}

fn answering_herdr(socket: &std::path::Path) -> AnsweringHerdr {
    use std::io::{BufRead, Write};
    let listener = hide_platform::ipc::LocalListener::bind(socket).unwrap();
    let closer = listener.closer();
    std::thread::spawn(move || {
        while let Ok(mut stream) = listener.accept() {
            std::thread::spawn(move || {
                let mut line = String::new();
                if std::io::BufReader::new(&mut stream)
                    .read_line(&mut line)
                    .is_err()
                {
                    return;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    return;
                };
                let response = if request["method"] == "ping" {
                    json!({"id": request["id"], "result": {"type": "pong"}})
                } else {
                    json!({"id": request["id"], "error": {"code": "unavailable", "message": "fake"}})
                };
                let _ = writeln!(stream, "{response}");
            });
        }
    });
    AnsweringHerdr { closer }
}

#[tokio::test]
async fn the_daemon_outlives_the_idle_window_while_its_herdr_answers() {
    let (dir, mut env) = test_env(false);
    env.idle_secs = 1;
    // A Unix socket path has a hard length limit; a tempdir may be too deep.
    let base = if cfg!(unix) {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let herdr_dir = base.join(format!("hided-idle-{}", std::process::id()));
    std::fs::create_dir_all(&herdr_dir).unwrap();
    let socket = herdr_dir.join("herdr.sock");
    let herdr = answering_herdr(&socket);
    env.herdr_socket_path = Some(socket.display().to_string());
    let running = std::sync::Arc::new(hided::start_daemon(env).await.expect("start daemon"));
    let announced = tokio::spawn({
        let running = std::sync::Arc::clone(&running);
        async move { hided::wait_shutdown(&running).await }
    });
    // No client ever connects; three idle windows pass.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !announced.is_finished(),
        "a daemon whose Herdr answers keeps making labels with no window open"
    );
    assert!(state_file::state_path(dir.path()).exists());
    drop(herdr);
    let _ = std::fs::remove_dir_all(&herdr_dir);
    announced.abort();
}

#[tokio::test]
async fn the_daemon_exits_after_the_idle_window_when_its_herdr_is_gone() {
    let (dir, mut env) = test_env(false);
    env.idle_secs = 1;
    env.herdr_socket_path = Some(dir.path().join("gone.sock").display().to_string());
    let running = std::sync::Arc::new(hided::start_daemon(env).await.expect("start daemon"));
    let stopped =
        tokio::time::timeout(Duration::from_secs(5), hided::wait_shutdown(&running)).await;
    assert!(
        stopped.is_ok(),
        "with no client and no Herdr the daemon ends after the idle window"
    );
}

#[tokio::test]
async fn a_window_cannot_say_whether_windows_are_attached() {
    let (_dir, running) = start().await;
    let mut socket = live_socket(&running).await;
    let answer = send_event(&mut socket, "ui_attached", json!({"attached": false})).await;
    assert_eq!(answer["type"], "error");
    assert_eq!(answer["message"], "ui_attached is sent by the daemon only");
}

async fn post_json(url: &str, token: Option<&str>, body: &str) -> u16 {
    use http_body_util::Full;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<hyper::body::Bytes>>();
    let mut request = hyper::Request::builder()
        .method("POST")
        .uri(url)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = request
        .body(Full::new(hyper::body::Bytes::from(body.to_owned())))
        .unwrap();
    client.request(request).await.unwrap().status().as_u16()
}

/// The desktop host's app-open, wake and unlock reach the core as one event
/// with the daemon's token only (PRD hide-ai-suggestions B6).
#[tokio::test]
async fn a_wake_is_taken_only_with_the_token_and_a_known_cause() {
    let (_dir, running) = start().await;
    let url = format!("http://127.0.0.1:{}/hide-ai-wake", running.port);
    let wake = r#"{"cause":"unlock"}"#;
    assert_eq!(post_json(&url, None, wake).await, 401);
    assert_eq!(post_json(&url, Some("not-the-token"), wake).await, 401);
    assert_eq!(
        post_json(&url, Some(&running.token), r#"{"cause":"lunch"}"#).await,
        400
    );
    assert_eq!(post_json(&url, Some(&running.token), wake).await, 204);
    running.stop();
}

/// `hide <path>` on the core's machine: the command asks with the daemon's
/// token over one `/ws` connection that is never a screen. A folder a
/// registered checkout holds comes forward and the windows are called; any
/// other folder meets the Add a project dialog's `$HOME` line and the core's
/// own registration, whose refusal is the command's answer.
#[tokio::test]
async fn hide_path_opens_a_registered_folder_and_answers_why_another_was_not_added() {
    let (dir, env) = test_env(true);
    let home = hide_platform::fs::identity::canonical(dir.path()).unwrap();
    let checkout = home.join("projects").join("alpha");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::create_dir_all(home.join("projects").join("beta")).unwrap();
    seed_registration(&env.state_dir, "w-alpha", &checkout);
    let running = hided::start_daemon(env).await.expect("start daemon");
    let mut window = connect(running.port, None).await;
    window
        .send(Message::Text(
            json!({"token": running.token, "schema_version": 2, "client_kind": "desktop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    // A renderer is told where a core move stands before its snapshot.
    while first_frame(&mut window).await["type"] != "snapshot" {}

    let open = |path: String| {
        let (port, token) = (running.port, running.token.clone());
        tokio::task::spawn_blocking(move || {
            hided::workspace_cli::request_project_open_with_token(port, &token, &path)
        })
    };
    let answer = open(wire(&checkout.join("src"))).await.unwrap().unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["result"]["path"], json!(wire(&checkout)));
    assert_eq!(answer["result"]["added"], false);
    assert_eq!(answer["result"]["window"], true);
    let called = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if first_frame(&mut window).await["type"] == "window_call" {
                return;
            }
        }
    })
    .await;
    assert!(called.is_ok(), "the open window is called forward");

    let outside = tempfile::tempdir().unwrap();
    for (path, reason) in [
        (wire(&home.join("projects").join("gone")), "not_found"),
        (wire(outside.path()), "needs_confirmation"),
        (wire(&home), "home_root"),
        // The dialog's own registration, refused by the core: no Herdr runs.
        (
            wire(&home.join("projects").join("beta")),
            "workspace_control_unavailable",
        ),
    ] {
        let answer = open(path.clone()).await.unwrap().unwrap();
        assert_eq!(answer["ok"], false, "{path}: {answer}");
        assert_eq!(answer["reason"], reason, "{path}");
        assert!(answer["next_action"].as_str().is_some_and(|next| !next.is_empty()));
    }
    let health: Value = serde_json::from_str(
        &reqwest_get(&format!("http://127.0.0.1:{}/health", running.port)).await,
    )
    .unwrap();
    assert_eq!(health["clients"], 1, "each command's connection left; the window stays");
    running.stop();
}
