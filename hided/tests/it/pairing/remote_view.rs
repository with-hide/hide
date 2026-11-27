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
    open_view_saying(port, token, host, key, json!({})).await
}

/// `open_view` with `more` added to the handshake.
async fn open_view_saying(port: u16, token: &str, host: &str, key: &str, more: Value) -> Socket {
    let mut socket = view_socket(port, token, host, key)
        .await
        .unwrap_or_else(|status| panic!("the view socket was refused: {status}"));
    let mut handshake = json!({"token": "", "schema_version": 2, "client_kind": "desktop",
        "snapshot_format": "semantic-rest-v1"});
    for (name, value) in more.as_object().unwrap() {
        handshake[name] = value.clone();
    }
    socket
        .send(Message::Text(handshake.to_string().into()))
        .await
        .unwrap();
    socket
}

/// What the host's daemon counts as drawing its browser displays now (`/health`).
async fn page_renderers(running: &hided::RunningDaemon) -> u64 {
    let (_, body) = get(running.port, "/health", None).await;
    serde_json::from_str::<Value>(&body).unwrap()["page_renderers"]
        .as_u64()
        .unwrap()
}

/// Waits until the host counts `wanted` page renderers.
async fn page_renderers_become(running: &hided::RunningDaemon, wanted: u64) {
    let deadline = Instant::now() + WITHIN;
    loop {
        let now = page_renderers(running).await;
        if now == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "page renderers stayed {now}, not {wanted}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await; // wait-ok: the host counts a view once its handshake lands and uncounts it once its socket closes
    }
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

/// A POST on the guest daemon, with its token.
async fn post(port: u16, path: &str, token: &str) -> (u16, Value) {
    use http_body_util::{BodyExt, Empty};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Empty<hyper::body::Bytes>>();
    let request = hyper::Request::post(format!("http://127.0.0.1:{port}{path}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Empty::new())
        .unwrap();
    let response = client.request(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// An origin on this machine's loopback that answers `ok` and keeps, for
/// each request, its path and the local port it came from.
async fn origin() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<(String, u16)>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = std::sync::Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut stream, from)) = listener.accept().await {
            let kept = std::sync::Arc::clone(&kept);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let Ok(read) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    head.extend_from_slice(&buffer[..read]);
                    while let Some(end) = head.windows(4).position(|window| window == b"\r\n\r\n") {
                        let text = String::from_utf8_lossy(&head[..end]).into_owned();
                        head.drain(..end + 4);
                        let path = text.split(' ').nth(1).unwrap_or_default().to_owned();
                        kept.lock().unwrap().push((path, from.port()));
                        if stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            });
        }
    });
    (port, seen)
}

/// One request a page's browser sends through the guest's listener, and the
/// status line of its answer; `secret` is None for a request on a connection
/// that already carried it.
async fn through(
    stream: &mut tokio::net::TcpStream,
    origin: u16,
    path: &str,
    secret: Option<&str>,
) -> String {
    use base64::Engine as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let auth = secret.map_or_else(String::new, |secret| {
        format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(format!("hide:{secret}"))
        )
    });
    stream
        .write_all(format!("GET http://127.0.0.1:{origin}{path} HTTP/1.1\r\nHost: 127.0.0.1:{origin}\r\n{auth}\r\n").as_bytes())
        .await
        .unwrap();
    let mut answer = Vec::new();
    let mut byte = [0_u8; 1];
    let deadline = Instant::now() + WITHIN;
    while !answer.ends_with(b"\r\n\r\n") {
        let left = deadline.saturating_duration_since(Instant::now());
        let read = tokio::time::timeout(left, stream.read(&mut byte))
            .await
            .expect("an answer")
            .unwrap();
        assert_eq!(
            read,
            1,
            "the connection closed: {}",
            String::from_utf8_lossy(&answer)
        );
        answer.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&answer).into_owned();
    if text.to_ascii_lowercase().contains("content-length: 2") {
        let mut body = [0_u8; 2];
        stream.read_exact(&mut body).await.unwrap();
    }
    text.lines().next().unwrap_or_default().to_owned()
}

/// The local ports the host's proxy logged dialing from.
async fn dialed(state: &std::path::Path, wanted: usize) -> Vec<u16> {
    let deadline = Instant::now() + WITHIN;
    loop {
        let log = std::fs::read_to_string(state.join("Logs/core.jsonl")).unwrap_or_default();
        let ports: Vec<u16> = log
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| row["kind"] == "screen_net.dialed")
            .filter_map(|row| {
                row["local_port"]
                    .as_u64()
                    .and_then(|port| u16::try_from(port).ok())
            })
            .collect();
        if ports.len() >= wanted || Instant::now() > deadline {
            return ports;
        }
        tokio::time::sleep(Duration::from_millis(50)).await; // wait-ok: the host's log has a dialed row for each request
    }
}

// docs/BROWSER_DISPLAYS.md, A page in another computer's screen: a page the
// guest draws of its host's screen reaches the host's network through the
// host. Two daemons on one machine share a loopback, so the proof is the
// connection: every request the origin records came from a local port the
// host's proxy logged dialing, the second of a kept-alive connection too,
// whose credentials only its first request carried. A pause refuses the
// next request with no stream and nothing reaching the origin, and the same
// listener carries again once the host resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_page_request_comes_from_a_port_the_host_dialed() {
    let root = crate::fixtures::fixture_root("hn");
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
    let net = format!(
        "/peer-screen/{host_id}/net?key={key}&owner={}",
        std::process::id()
    );
    let (status, listener) = post(port, &net, &token).await;
    assert_eq!(status, 200, "{listener}");
    assert_eq!(listener["state"], "ready", "{listener}");
    let listen = u16::try_from(listener["port"].as_u64().unwrap()).unwrap();
    let secret = listener["secret"].as_str().unwrap().to_owned();
    let (status, _) = post(
        port,
        &format!("/peer-screen/{host_id}/net?key={key}&owner=0"),
        &token,
    )
    .await;
    assert_eq!(status, 400, "no owner is no listener");
    let (status, refused) = post(
        port,
        &format!(
            "/peer-screen/no-such-machine/net?key={key}&owner={}",
            std::process::id()
        ),
        &token,
    )
    .await;
    assert_eq!(
        (status, refused["reason"].as_str()),
        (503, Some("not_paired"))
    );

    let (origin_port, seen) = origin().await;
    let mut page = tokio::net::TcpStream::connect(("127.0.0.1", listen))
        .await
        .unwrap();
    assert_eq!(
        through(&mut page, origin_port, "/one", Some(&secret)).await,
        "HTTP/1.1 200 OK"
    );
    assert_eq!(
        through(&mut page, origin_port, "/two", None).await,
        "HTTP/1.1 200 OK"
    );
    let arrived = seen.lock().unwrap().clone();
    assert_eq!(
        arrived
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["/one", "/two"]
    );
    // A `CONNECT` tunnel, as Chromium opens for `https` and WebSockets:
    // the host dials the origin and joins the two, and what the page then
    // sends inside it reaches the origin from that dialed port.
    {
        use base64::Engine as _;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut tunnel = tokio::net::TcpStream::connect(("127.0.0.1", listen))
            .await
            .unwrap();
        tunnel
            .write_all(
                format!(
                    "CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\nHost: 127.0.0.1:{origin_port}\r\nProxy-Authorization: Basic {}\r\n\r\n",
                    base64::engine::general_purpose::STANDARD.encode(format!("hide:{secret}"))
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut answer = Vec::new();
        let mut byte = [0_u8; 1];
        while !answer.ends_with(b"\r\n\r\n") {
            let read = tokio::time::timeout(WITHIN, tunnel.read(&mut byte))
                .await
                .expect("the tunnel's answer")
                .unwrap();
            assert_eq!(read, 1, "{}", String::from_utf8_lossy(&answer));
            answer.push(byte[0]);
        }
        assert!(
            String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 200"),
            "{}",
            String::from_utf8_lossy(&answer)
        );
        tunnel
            .write_all(b"GET /tunneled HTTP/1.1\r\nHost: origin\r\n\r\n")
            .await
            .unwrap();
        let mut ok = [0_u8; 40];
        let read = tokio::time::timeout(WITHIN, tunnel.read(&mut ok))
            .await
            .expect("the origin's answer")
            .unwrap();
        assert!(ok[..read].starts_with(b"HTTP/1.1 200 OK"));
    }
    let arrived = seen.lock().unwrap().clone();
    assert_eq!(
        arrived.last().map(|(path, _)| path.as_str()),
        Some("/tunneled")
    );
    let ports = dialed(&root.path().join("host-state"), arrived.len()).await;
    for (path, from) in &arrived {
        assert!(
            ports.contains(from),
            "{path} came from port {from}, which the host's proxy never dialed: {ports:?}"
        );
    }

    let paused = host.run(&["device", "pause", GUEST, "--incoming"]);
    assert!(paused.ok, "{}", paused.text());
    let (_, during) = post(port, &net, &token).await;
    assert_eq!(during["state"], "paused_there", "{during}");
    assert_eq!(
        during["port"], listener["port"],
        "a pause keeps the listener"
    );
    assert_eq!(
        during["pairing"], listener["pairing"],
        "a pause keeps the pairing its sign-ins are stored under"
    );
    assert_eq!(listener["pairing"].as_str().map(str::len), Some(32));
    let mut later = tokio::net::TcpStream::connect(("127.0.0.1", listen))
        .await
        .unwrap();
    assert_eq!(
        through(&mut later, origin_port, "/paused", Some(&secret)).await,
        "HTTP/1.1 503 Service Unavailable"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "nothing reached the origin while paused"
    );

    let resumed = host.run(&["device", "resume", GUEST, "--incoming"]);
    assert!(resumed.ok, "{}", resumed.text());
    let deadline = Instant::now() + WITHIN;
    loop {
        let (_, now) = post(port, &net, &token).await;
        if now["state"] == "ready" {
            assert_eq!(
                (now["port"].clone(), now["secret"].clone()),
                (listener["port"].clone(), listener["secret"].clone())
            );
            break;
        }
        assert!(Instant::now() < deadline, "the grant came back: {now}");
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the guest's listener reads ready once the resumed grant reaches it
    }
    let mut again = tokio::net::TcpStream::connect(("127.0.0.1", listen))
        .await
        .unwrap();
    assert_eq!(
        through(&mut again, origin_port, "/resumed", Some(&secret)).await,
        "HTTP/1.1 200 OK"
    );
    drop(running);
}

/// A POST of `body` on the guest daemon, with its token.
async fn post_json(port: u16, path: &str, token: &str, body: &Value) -> (u16, Value) {
    use http_body_util::{BodyExt, Full};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    let client = Client::builder(TokioExecutor::new()).build_http::<Full<hyper::body::Bytes>>();
    let request = hyper::Request::post(format!("http://127.0.0.1:{port}{path}"))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Full::new(hyper::body::Bytes::from(body.to_string())))
        .unwrap();
    let response = client.request(request).await.unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

// docs/BROWSER_DISPLAYS.md, A page in another computer's screen: the guest
// asks the host where a display of the host's screen loads from, over that
// screen's own stream, so the host's core is the one that decides. Nothing is
// asked while the guest shows no screen of the host; once it shows one, the
// host reads the question (one out of shape is refused, which it does only
// for a live view of this guest) and asks its core, which answers a display
// it does not have as gone. The core's answers for a display that is there,
// the host's own and a third machine's, are `remote_screen::tests`' (a
// display needs a pane, which this fixture's daemon has no holder for).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screen_asks_its_host_where_a_display_loads_from() {
    let root = crate::fixtures::fixture_root("hr");
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
    let route = format!("/peer-screen/{host_id}/page-route?key={key}");
    let ask = json!({"device": host_id, "checkout": "/web", "view": "d999", "load": 1});

    let (status, answer) = post_json(port, &route, &token, &ask).await;
    assert_eq!(
        (status, answer["reason"].as_str()),
        (503, Some("not_shown")),
        "{answer}"
    );
    let mut view = open_view(port, &token, &host_id, &key).await;
    until(&mut view, "the host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    let (status, answer) = post_json(port, &route, &token, &json!({"device": ""})).await;
    assert_eq!(
        status, 400,
        "the host read the question of a live view: {answer}"
    );
    let (status, answer) = post_json(port, &route, &token, &ask).await;
    assert_eq!(
        (status, answer["reason"].as_str()),
        (410, Some("view_unavailable")),
        "{answer}"
    );
    drop(view);
    drop(running);
}

// docs/BROWSER_DISPLAYS.md, A machine with no window of its own: a view of
// the host whose app draws pages counts on the host beside its desktop
// windows, which is what lets an agent there open a page with no window of
// the host's own (`page_renderers`, the gate of `hide browser open`); a view
// that does not say so, and a window of the host that says so, count
// nothing; the count goes with the view.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guest_view_that_draws_pages_counts_on_its_host_while_it_lives() {
    let root = crate::fixtures::fixture_root("hd");
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
    page_renderers_become(&running, 0).await;

    // A window of the host that says it draws pages is not one of its
    // desktop windows, and counts nothing.
    let mut own = Window::open_saying(
        running.port,
        &running.token,
        json!({"client_kind": "web", "draws_pages": true}),
    )
    .await;
    own.frame("the host's snapshot", |frame| frame["type"] == "snapshot")
        .await;
    assert_eq!(
        page_renderers(&running).await,
        0,
        "a window of the host saying it draws pages"
    );
    let mut older = open_view(port, &token, &host_id, &key).await;
    until(&mut older, "the host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    assert_eq!(
        page_renderers(&running).await,
        0,
        "a view that does not say it draws pages"
    );
    drop(older);

    let mut drawing =
        open_view_saying(port, &token, &host_id, &key, json!({"draws_pages": true})).await;
    until(&mut drawing, "the host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    page_renderers_become(&running, 1).await;
    drop(drawing);
    page_renderers_become(&running, 0).await;
    drop(own);
    drop(running);
}
