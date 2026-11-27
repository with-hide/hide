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

/// The host core's log records of `kind`, in order.
fn logged(state: &std::path::Path, kind: &str) -> Vec<Value> {
    std::fs::read_to_string(state.join("Logs/core.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["kind"] == kind)
        .collect()
}

/// What the host's core was told of the machines showing their view of it,
/// in order (`look.remote_views`, each machine by the start of its key).
fn views_told(state: &std::path::Path) -> Vec<Vec<String>> {
    logged(state, "look.remote_views")
        .iter()
        .map(|record| {
            record["machines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| key.as_str().unwrap().to_owned())
                .collect()
        })
        .collect()
}

/// The screens the host's core recorded the operator's look chosen on, in
/// order (`look.chosen_on`: `own`, or a viewing machine by the start of its
/// key).
fn chosen_on(state: &std::path::Path) -> Vec<String> {
    logged(state, "look.chosen_on")
        .iter()
        .map(|record| record["screen"].as_str().unwrap().to_owned())
        .collect()
}

/// Waits until `read` of the host's log ends with `wanted`, and answers all
/// of it.
async fn ends_with<T: PartialEq + std::fmt::Debug>(
    state: &std::path::Path,
    read: fn(&std::path::Path) -> Vec<T>,
    wanted: &T,
) -> Vec<T> {
    let deadline = Instant::now() + WITHIN;
    loop {
        let told = read(state);
        if told.last() == Some(wanted) {
            return told;
        }
        assert!(Instant::now() < deadline, "never {wanted:?}: {told:?}");
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the view's seat, its end and its events cross the connection; the record is the host core's own
    }
}

/// A look the operator chooses in the view counts only while the guest shows
/// it (operator report 2026-11-27, `docs/status-model.md`, Hide owns the read
/// axis): the host's core is told which machines show their view as each one
/// is seated and as it ends, so leaving the tile, which closes the view's
/// socket, ends the guest's look there, and showing the tile again starts
/// it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_host_s_core_hears_which_machines_show_their_view() {
    let root = crate::fixtures::fixture_root("hw");
    let running = hided::start_daemon(host_env(root.path())).await.unwrap();
    let host_state = root.path().join("host-state");
    let host = Terminal::new(root.path().join("host-home"), host_state.clone(), None);
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
    let guest_key = host_key(&host);
    let showing = vec![guest_key[..16].to_owned()];
    assert!(views_told(&host_state).is_empty(), "no view yet");

    let mut view = open_view(port, &token, &host_id, &key).await;
    until(&mut view, "the host's snapshot", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    ends_with(&host_state, views_told, &showing).await;

    // A choice made in the view is the guest's: the daemon dispatches the
    // view's event as sent from its machine, so it is chosen there.
    send(&mut view, "focus_device", json!({"device_id": &host_id})).await;
    ends_with(&host_state, chosen_on, &showing[0]).await;
    // ... and one made on the host's own window is the host's.
    let mut own = Window::open(&running, "web").await;
    own.frame("the host's snapshot", |frame| frame["type"] == "snapshot")
        .await;
    own.send("focus_device", json!({"device_id": &host_id}))
        .await;
    let chosen = ends_with(&host_state, chosen_on, &"own".to_owned()).await;
    assert_eq!(chosen, vec![showing[0].clone(), "own".to_owned()]);
    drop(own);

    // The tile hidden: the desktop closes the socket.
    drop(view);
    let told = ends_with(&host_state, views_told, &Vec::new()).await;
    assert_eq!(told, vec![showing.clone(), Vec::new()]);

    // The tile shown again.
    let mut shown = open_view(port, &token, &host_id, &key).await;
    until(&mut shown, "the host's snapshot again", |frame| {
        frame["type"] == "snapshot"
    })
    .await;
    let told = ends_with(&host_state, views_told, &showing).await;
    assert_eq!(told, vec![showing.clone(), Vec::new(), showing.clone()]);
}

/// What this machine's core was told its windows show, in order
/// (`look.window_screen`: `own` or `computer`).
fn screens_told(state: &std::path::Path) -> Vec<String> {
    logged(state, "look.window_screen")
        .iter()
        .map(|record| record["screen"].as_str().unwrap().to_owned())
        .collect()
}

/// The mirror of the view's rule on this machine (`docs/status-model.md`,
/// Hide owns the read axis): a look chosen on this machine's window counts
/// only while a window of it shows its own screen, so the core hears another
/// computer's screen while every window that reports shows one, this
/// machine's own again the moment one window does, and a window that goes
/// lets go of its report with its socket; the last one going leaves the core
/// where it was, and the app opened again opens on this machine's own screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn this_machine_s_core_hears_whether_a_window_shows_its_own_screen() {
    let root = crate::fixtures::fixture_root("ws");
    let running = hided::start_daemon(host_env(root.path())).await.unwrap();
    let state = root.path().join("host-state");
    let snapshot = |frame: &Value| frame["type"] == "snapshot";
    let before = host_clients(&running).await;

    let mut desktop = Window::open(&running, "desktop").await;
    desktop.frame("the desktop's snapshot", snapshot).await;
    desktop
        .send("window_screen", json!({"screen": "computer"}))
        .await;
    ends_with(&state, screens_told, &"computer".to_owned()).await;

    let mut tab = Window::open(&running, "web").await;
    tab.frame("the tab's snapshot", snapshot).await;
    tab.send("window_screen", json!({"screen": "own"})).await;
    ends_with(&state, screens_told, &"own".to_owned()).await;

    drop(tab);
    ends_with(&state, screens_told, &"computer".to_owned()).await;
    // The last window goes while it shows another computer's screen: its
    // report is let go once the daemon has counted it gone.
    drop(desktop);
    let deadline = Instant::now() + WITHIN;
    while host_clients(&running).await != before {
        assert!(
            Instant::now() < deadline,
            "the closed window stayed counted"
        );
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the closed socket's end crosses the connection; the count is the daemon's own, taken after it lets go of the window's reports (`client_gone`)
    }
    // The app opened again opens on this machine's own screen and says so.
    let mut reopened = Window::open(&running, "desktop").await;
    reopened.frame("the reopened snapshot", snapshot).await;
    reopened
        .send("window_screen", json!({"screen": "own"}))
        .await;
    let told = ends_with(&state, screens_told, &"own".to_owned()).await;
    assert_eq!(told, ["computer", "own", "computer", "own"]);
}
