//! Another machine's view of a Hide (PRD device-connect-iroh c1; D-53, D-67,
//! B17, B52, B63): the guest's desktop reaches its host's page and socket
//! through the guest's own daemon, over the screen streams of the one
//! connection the two paired with. The view draws the host, refuses on its
//! own socket what lets a machine in, and leaves nothing on the host once it
//! closes. It ends when the host pauses the guest's way in.

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

use super::{GUEST, Socket, StopOnDrop, Terminal, WITHIN, Window, host_env, pair_by_link};

/// The guest daemon's port and token, as its state file says.
fn daemon_of(terminal: &Terminal) -> (u16, String) {
    let state = hided::state_file::read_state(&terminal.state)
        .unwrap()
        .expect("the guest daemon is running");
    (state.port, state.token)
}

/// A GET on the guest daemon, with its token or none.
async fn get(port: u16, path: &str, token: Option<&str>) -> (u16, String) {
    use http_body_util::{BodyExt, Empty};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<hyper::body::Bytes>>();
    let mut request = hyper::Request::get(format!("http://127.0.0.1:{port}{path}"));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = client
        .request(request.body(Empty::new()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// The key the guest lists its one paired machine under.
fn host_key(guest: &Terminal) -> String {
    let peers = guest.peers();
    let [peer] = peers.as_slice() else {
        panic!("one paired machine: {peers:?}")
    };
    peer["connection_id"].as_str().unwrap().to_owned()
}

/// The desktop's view socket on the guest daemon, for the page opened under
/// `key`: the handshake, or the status the daemon refused it with.
async fn view_socket(port: u16, token: &str, host: &str, key: &str) -> Result<Socket, u16> {
    let mut request = format!("ws://127.0.0.1:{port}/peer-screen/{host}/ws?key={key}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(error) => panic!("the view socket: {error}"),
    }
}

/// The desktop's view socket on the guest daemon, its handshake sent as a
/// page that holds no token of the host sends it.
async fn open_view(port: u16, token: &str, host: &str, key: &str) -> Socket {
    let mut socket = view_socket(port, token, host, key)
        .await
        .unwrap_or_else(|status| panic!("the view socket was refused: {status}"));
    socket
        .send(Message::Text(
            json!({"token": "", "schema_version": 2, "client_kind": "desktop",
                "snapshot_format": "semantic-rest-v1"})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    socket
}

async fn send(socket: &mut Socket, kind: &str, payload: Value) {
    socket
        .send(Message::Text(
            json!({"schema_version": hide_core::SCHEMA_VERSION, "kind": kind, "payload": payload})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

/// The frames the view is sent until `wanted` takes one.
async fn until(socket: &mut Socket, what: &str, wanted: impl Fn(&Value) -> bool) -> Vec<Value> {
    let deadline = Instant::now() + WITHIN;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let frame = tokio::time::timeout(left, socket.next())
            .await
            .unwrap_or_else(|_| panic!("no {what} came: {seen:?}"))
            .unwrap_or_else(|| panic!("the view closed before {what}: {seen:?}"));
        let Ok(Message::Text(text)) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).unwrap();
        let done = wanted(&value);
        seen.push(value);
        if done {
            return seen;
        }
    }
}

/// The code the daemon closes `window` with; the frames before it are passed over.
async fn close_code(window: &mut Window) -> u16 {
    let deadline = Instant::now() + WITHIN;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, window.0.next()).await {
            Err(_) => panic!("the daemon did not close the screen"),
            Ok(Some(Ok(Message::Close(frame)))) => {
                return frame.map_or(1005, |frame| u16::from(frame.code));
            }
            Ok(Some(Ok(_))) => {}
            Ok(None | Some(Err(_))) => return 1006,
        }
    }
}

/// How many clients the host's daemon counts now (`/health`).
async fn host_clients(running: &hided::RunningDaemon) -> u64 {
    let (_, body) = get(running.port, "/health", None).await;
    serde_json::from_str::<Value>(&body).unwrap()["clients"]
        .as_u64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guest_views_its_host_and_the_view_lets_nothing_in() {
    let root = crate::fixtures::fixture_root("hv");
    let running = hided::start_daemon(host_env(root.path())).await.unwrap();
    let host = Terminal::new(
        root.path().join("host-home"),
        root.path().join("host-state"),
        None,
    );
    let guest = Terminal::new(
        root.path().join("guest-home"),
        root.path().join("guest-state"),
        Some(GUEST),
    );
    let _stop = StopOnDrop(&guest);
    let connected = guest.run(&["connect"]);
    assert!(connected.ok, "the guest daemon: {}", connected.text());
    let (window, host_id, _) = pair_by_link(&running, &host, &guest).await;
    drop(window);
    let (port, token) = daemon_of(&guest);
    let key = host_key(&guest);
    let before = host_clients(&running).await;

    // The host's page comes through the guest's daemon, for its token only;
    // the host answers nothing but its page, its assets and its socket.
    let page_path = format!("/peer-screen/{host_id}/?key={key}");
    let (status, page) = get(port, &page_path, Some(&token)).await;
    assert_eq!(status, 200, "{page}");
    let (status, _) = get(port, &page_path, None).await;
    assert_eq!(status, 401);
    let (status, _) = get(
        port,
        &format!("/peer-screen/{host_id}/health"),
        Some(&token),
    )
    .await;
    assert_eq!(status, 404);
    let (status, body) = get(
        port,
        &format!("/peer-screen/no-such-machine/?key={key}"),
        Some(&token),
    )
    .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["reason"],
        "not_paired"
    );

    // B17: the view is the host's screen: its daemon frame names the host,
    // and a snapshot follows. It is never sent the host's pairings.
    let mut view = open_view(port, &token, &host_id, &key).await;
    let opened = until(&mut view, "the host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    let daemon = opened
        .iter()
        .find(|frame| frame["type"] == "daemon")
        .expect("a daemon frame");
    assert_eq!(
        daemon["payload"]["state_dir"],
        root.path().join("host-state").display().to_string(),
        "{daemon}"
    );
    assert!(
        opened.iter().all(|frame| frame["type"] != "pairing"),
        "{opened:?}"
    );
    // It takes a seat of its own beside the host's windows.
    assert_eq!(host_clients(&running).await, before + 1);

    // D-53, B52: what lets a machine or a tool in, and a kind no one
    // classified, is refused on the view's socket alone.
    for kind in [
        "pairing_share",
        "access_approve",
        "mobile_show_code",
        "s3_unclassified_probe",
    ] {
        send(&mut view, kind, json!({"request_id": "v1"})).await;
        let seen = until(&mut view, "the refusal", |frame| frame["type"] == "refused").await;
        let refused = seen.last().unwrap();
        assert_eq!(refused["payload"]["kind"], kind, "{refused}");
    }
    // ... and never lands in what every screen of the host shows.
    let mut own = Window::open(&running, "web").await;
    let snapshot = own
        .frame("the host's snapshot", |frame| frame["type"] == "snapshot")
        .await;
    assert!(
        snapshot["payload"]["rest"]["status"]["last_error"].is_null(),
        "{}",
        snapshot["payload"]["rest"]["status"]
    );
    drop(own);

    // D-67: the view takes none of this machine's eight places. With it
    // open, seven windows and a phone are this machine's own: the phone is
    // answered as a phone (refused for its credential, not for room), an
    // eighth window draws, and only a ninth is refused `client_limit`, while
    // the view still answers.
    let mut windows = Vec::new();
    for _ in 0..7 {
        let mut window = Window::open(&running, "web").await;
        window
            .frame("a window's snapshot", |frame| frame["type"] == "snapshot")
            .await;
        windows.push(window);
    }
    let mut phone = Window::open(&running, "phone").await;
    let answer = phone
        .frame("the phone's answer", |frame| frame["type"] == "refused")
        .await;
    assert_ne!(answer["reason"], "client_limit", "{answer}");
    drop(phone);
    // ... and so is a `hide <path>` from this machine's terminal.
    let mut cli = Window::open(&running, "cli").await;
    cli.0
        .send(Message::Text(
            json!({"type": "project_add", "request_id": "c1",
                "path": root.path().join("no-such-folder").display().to_string()})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let answer = cli
        .frame("the command's answer", |frame| {
            frame["type"] == "workspace_result"
        })
        .await;
    assert_ne!(answer["reason"], "client_limit", "{answer}");
    drop(cli);
    let mut eighth = Window::open(&running, "web").await;
    eighth
        .frame("the eighth window's snapshot", |frame| {
            frame["type"] == "snapshot"
        })
        .await;
    windows.push(eighth);
    let mut ninth = Window::open(&running, "web").await;
    assert_eq!(close_code(&mut ninth).await, 4004, "client_limit");
    send(&mut view, "pairing_share", json!({"request_id": "v2"})).await;
    until(&mut view, "the view still answering", |frame| {
        frame["type"] == "refused"
    })
    .await;
    drop(windows);

    // B63: the view's end leaves nothing on the host.
    drop(view);
    let deadline = Instant::now() + WITHIN;
    while host_clients(&running).await != before {
        assert!(
            Instant::now() < deadline,
            "the view's seat stayed on the host"
        );
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the close crosses the connection; the count is the host daemon's own
    }

    // B21, B25: the host pauses the guest's way in; the guest's next view
    // says so instead of opening.
    let paused = host.run(&["device", "pause", GUEST, "--incoming", "--json"]);
    assert!(paused.ok, "{}", paused.text());
    guest
        .peer_becomes("the guest hears the pause", |peer| {
            peer["opens"] == "paused_there"
        })
        .await;
    let (status, body) = get(port, &page_path, Some(&token)).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["reason"],
        "paused_there"
    );
}

/// Security pre-review S1: a page is the screen of the key its machine was
/// paired under. When another Hide takes the same device id under a new key
/// (the machine lost its key and was set up again), every request that
/// names the earlier key, its page, its assets and its socket, is refused,
/// so a page the earlier host loaded and the desktop still holds never
/// reaches the later host; the later key's page and socket open on the
/// later host. A request that names no key is no request.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_view_asked_under_a_device_s_earlier_key_is_refused_once_another_host_takes_it() {
    let root = crate::fixtures::fixture_root("hk");
    let earlier = hided::start_daemon(host_env(root.path())).await.unwrap();
    let earlier_host = Terminal::new(
        root.path().join("host-home"),
        root.path().join("host-state"),
        None,
    );
    let guest = Terminal::new(
        root.path().join("guest-home"),
        root.path().join("guest-state"),
        Some(GUEST),
    );
    let _stop = StopOnDrop(&guest);
    let connected = guest.run(&["connect"]);
    assert!(connected.ok, "the guest daemon: {}", connected.text());
    let (window, device, _) = pair_by_link(&earlier, &earlier_host, &guest).await;
    drop(window);
    let (port, token) = daemon_of(&guest);
    let first = host_key(&guest);
    let (status, page) = get(
        port,
        &format!("/peer-screen/{device}/?key={first}"),
        Some(&token),
    )
    .await;
    assert_eq!(status, 200, "{page}");
    for query in ["", "?key=", "?key=xyz", "?other=1"] {
        let (status, _) = get(
            port,
            &format!("/peer-screen/{device}/{query}"),
            Some(&token),
        )
        .await;
        assert_eq!(status, 400, "{query}");
    }

    // The same machine id, a state folder of its own: another key. The
    // earlier host keeps running, so a refusal below is the key's, not a
    // lost connection's.
    let other = crate::fixtures::fixture_root("hl");
    let later = hided::start_daemon(host_env(other.path())).await.unwrap();
    let later_host = Terminal::new(
        other.path().join("host-home"),
        other.path().join("host-state"),
        None,
    );
    let (window, later_device, _) = pair_by_link(&later, &later_host, &guest).await;
    drop(window);
    assert_eq!(later_device, device, "one device id");
    let second = host_key(&guest);
    assert_ne!(second, first);

    for path in ["", "assets/app.js"] {
        let (status, body) = get(
            port,
            &format!("/peer-screen/{device}/{path}?key={first}"),
            Some(&token),
        )
        .await;
        assert_eq!(status, 503, "{path}: {body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["reason"],
            "not_paired",
            "{path}"
        );
    }
    assert_eq!(
        view_socket(port, &token, &device, &first).await.err(),
        Some(503),
        "the earlier key's socket"
    );

    let (status, page) = get(
        port,
        &format!("/peer-screen/{device}/?key={second}"),
        Some(&token),
    )
    .await;
    assert_eq!(status, 200, "{page}");
    let mut view = open_view(port, &token, &device, &second).await;
    let opened = until(&mut view, "the later host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    let daemon = opened
        .iter()
        .find(|frame| frame["type"] == "daemon")
        .expect("a daemon frame");
    assert_eq!(
        daemon["payload"]["state_dir"],
        other.path().join("host-state").display().to_string(),
        "{daemon}"
    );
}
