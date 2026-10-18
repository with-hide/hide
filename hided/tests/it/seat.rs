//! A process keeps one address through a change of role (PRD
//! core-host-node-move Q12): the window's port and token stay, and
//! `/health` answers on every poll while the core stops, a move screen
//! stands in, and a core starts again on the same seat.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use hided::env::Env;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn env(dir: &std::path::Path) -> Env {
    Env {
        home: dir.to_path_buf(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        login_shell: None,
        state_dir: dir.to_path_buf(),
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
        tailscale_bin: Some(dir.join("no-tailscale")),
        search_path: None,
    }
}

/// The status and body of one `GET /health` on a fresh connection, as the
/// desktop host asks it.
async fn health(port: u16) -> Option<(u16, Value)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut answer = Vec::new();
    tokio::time::timeout(Duration::from_millis(1500), stream.read_to_end(&mut answer))
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&answer);
    let status = text.split_whitespace().nth(1)?.parse().ok()?;
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default();
    Some((status, serde_json::from_str(body).unwrap_or(Value::Null)))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_role_change_keeps_the_port_and_token_and_health_answers_throughout() {
    let dir = tempfile::tempdir().unwrap();
    let _lock = hided::state_file::acquire_lock(dir.path()).unwrap();
    let listener = hided::server::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let seat = hided::seat::Seat::serve(
        listener,
        "seat-token".to_owned(),
        hided::ending::Ending::new(),
    )
    .unwrap();
    let parts = seat.parts();

    let core = hided::start_core_role(env(dir.path()), parts.clone())
        .await
        .unwrap();
    assert_eq!((core.port, core.token.as_str()), (seat.port, "seat-token"));
    let (status, first) = health(seat.port).await.expect("the core answers");
    assert_eq!(status, 200);
    assert_eq!(first["instance"], 1);

    let stop = Arc::new(AtomicBool::new(false));
    let missed = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let poller = tokio::spawn({
        let (stop, missed, polls, port) = (stop.clone(), missed.clone(), polls.clone(), seat.port);
        async move {
            while !stop.load(Ordering::SeqCst) {
                match health(port).await {
                    Some((200, _)) => {}
                    _ => {
                        missed.fetch_add(1, Ordering::SeqCst);
                    }
                }
                polls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    });

    // A move screen stands in while the core stops, as the move does.
    parts.mount(Router::new().route(
        "/health",
        get(|| async { axum::Json(serde_json::json!({"role": "moving"})) }),
    ));
    drop(core);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_, moving) = health(seat.port).await.expect("the move screen answers");
    assert_eq!(moving["role"], "moving");

    // A rollback starts the core again on the same seat.
    let core = hided::start_core_role(env(dir.path()), parts.clone())
        .await
        .unwrap();
    assert_eq!((core.port, core.token.as_str()), (seat.port, "seat-token"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.store(true, Ordering::SeqCst);
    poller.await.unwrap();
    let (status, again) = health(seat.port).await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(again["instance"], 3, "{again}");
    assert!(polls.load(Ordering::SeqCst) > 5, "the poll ran");
    assert_eq!(
        missed.load(Ordering::SeqCst),
        0,
        "a health poll went unanswered"
    );
    drop(core);
    let port = seat.port;
    seat.close().await;
    assert!(health(port).await.is_none(), "a closed seat stops serving");
}
