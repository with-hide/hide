//! Two Hides pair (PRD device-connect-iroh c1, c2): a host whose window
//! shares a link and answers, and a guest that is a second daemon with a
//! machine id of its own, driven by the built `hide` at a terminal. They
//! pair by the link and the same four words, the host asks to be let in the
//! other way, each direction is stopped and removed apart, and what is the
//! operator's alone is refused to a pane and to a screen that is not this
//! machine's own window (B52).
//!
//! The two endpoints bind loopback with no relay (`HIDE_PEER_DIRECT`), so
//! nothing leaves the machine.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

mod bootstrap_origin;
#[cfg(unix)]
mod core_letters;
#[cfg(unix)]
mod fault_run;
#[cfg(unix)]
mod guest_work;
#[cfg(unix)]
mod moved_core;
mod remote_view;

/// The machine id the guest daemon runs under, so one host is two machines.
const GUEST: &str = "pairing-guest";

const WITHIN: Duration = Duration::from_secs(40);

/// A relay nothing answers at: with `HIDE_PEER_RELAY_ONLY` an endpoint has
/// no IP path and reaches no relay, as two machines with no way to each
/// other but SSH.
const NO_RELAY: &str = "https://127.0.0.1:9";

fn host_env(root: &Path) -> Env {
    let state = root.join("host-state");
    hide_platform::fs::private::create_dir_all(&state).unwrap();
    crate::fixtures::fixture_home(&root.join("host-home"));
    Env {
        home: root.join("host-home"),
        state_dir: state,
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
        install: None,
        access: None,
        tailscale_bin: Some(root.join("no-tailscale")),
        relay_url: "ws://127.0.0.1:9/ws".to_owned(),
        peer_direct: Some("127.0.0.1:0".parse().unwrap()),
        peer_relay: None,
        peer_relay_only: false,
        search_path: None,
    }
}

/// A screen connection to the host daemon.
struct Window(Socket);

impl Window {
    async fn open(running: &hided::RunningDaemon, kind: &str) -> Self {
        Self::open_at(running.port, &running.token, kind).await
    }

    /// A screen of `kind` on the daemon that listens on `port`.
    async fn open_at(port: u16, token: &str, kind: &str) -> Self {
        let mut request = format!("ws://127.0.0.1:{port}/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(ORIGIN, format!("http://127.0.0.1:{port}").parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(Message::Text(
                json!({"token": token, "schema_version": 2, "client_kind": kind})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        Self(socket)
    }

    async fn send(&mut self, kind: &str, payload: Value) {
        self.0
            .send(Message::Text(
                json!({"schema_version": hide_core::SCHEMA_VERSION, "kind": kind,
                    "payload": payload})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
    }

    /// The next frame `wanted` takes, of those the daemon sends this screen.
    async fn frame(&mut self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WITHIN;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let frame = tokio::time::timeout(left, self.0.next())
                .await
                .unwrap_or_else(|_| panic!("no {what} came"))
                .expect("the connection closed")
                .unwrap();
            let Message::Text(text) = frame else { continue };
            let value: Value = serde_json::from_str(&text).unwrap();
            if wanted(&value) {
                return value;
            }
        }
    }

    /// The answer to the `pairing_<verb>` this window sent.
    async fn result(&mut self, verb: &str) -> Value {
        let frame = self
            .frame("pairing result", |frame| {
                frame["type"] == "pairing_result" && frame["payload"]["verb"] == verb
            })
            .await;
        frame["payload"].clone()
    }
}

/// The built `hide`, at a terminal of one of the two machines.
struct Terminal {
    home: PathBuf,
    state: PathBuf,
    machine: Option<&'static str>,
    /// The daemon this terminal starts has no IP path and an unreachable
    /// relay, so nothing but SSH reaches another machine from it.
    no_direct_path: bool,
}

impl Terminal {
    /// A terminal of the machine whose HOME is `home`, which is made a
    /// fixture's (`fixtures::fixture_home`).
    fn new(home: PathBuf, state: PathBuf, machine: Option<&'static str>) -> Self {
        crate::fixtures::fixture_home(&home);
        Self {
            home,
            state,
            machine,
            no_direct_path: false,
        }
    }

    /// This terminal, its daemon reaching nothing but over SSH.
    fn with_no_direct_path(mut self) -> Self {
        self.no_direct_path = true;
        self
    }

    fn command_of(&self, program: &Path, args: &[&str]) -> Command {
        // A `hide` here may start the machine's daemon, which under a HOME
        // that is no fixture's reaches the account's login items once it is
        // paired. A cleanup that runs while the test fails is let through.
        assert!(
            std::thread::panicking() || hide_kit::layout::fixture_home(&self.home),
            "{} is not a fixture HOME: make the terminal with `Terminal::new`",
            self.home.display()
        );
        let mut command = Command::new(program);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().to_uppercase().starts_with("HIDE_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .current_dir(&self.home)
            .env(hided::env::HOME, &self.home)
            .env("APPDATA", self.home.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", self.home.join("AppData").join("Local"))
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env_remove("XDG_STATE_HOME")
            .env("HIDE_STATE_DIR", &self.state)
            .env("HIDE_TAILSCALE_BIN", self.home.join("no-tailscale"))
            .env("HIDE_RELAY_URL", "ws://127.0.0.1:9/ws")
            .env("LANG", "en_US.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(machine) = self.machine {
            command.env(hided::env::HIDE_MACHINE_ID, machine);
        }
        match self.no_direct_path {
            true => command
                .env("HIDE_PEER_RELAY", NO_RELAY)
                .env("HIDE_PEER_RELAY_ONLY", "1"),
            false => command.env("HIDE_PEER_DIRECT", "127.0.0.1:0"),
        };
        command
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_of(Path::new(env!("CARGO_BIN_EXE_hide")), args)
    }

    fn run(&self, args: &[&str]) -> Ran {
        Ran::of(self.command(args).output().unwrap())
    }

    /// Runs `hide args` with `typed` waiting on its input, off the runtime.
    fn typing(&self, args: &[&str], typed: &'static str) -> tokio::task::JoinHandle<Ran> {
        let mut command = self.command(args);
        command.stdin(Stdio::piped());
        tokio::task::spawn_blocking(move || {
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(typed.as_bytes())
                .unwrap();
            Ran::of(child.wait_with_output().unwrap())
        })
    }

    /// The paired machines `hide device list` names.
    fn peers(&self) -> Vec<Value> {
        let ran = self.run(&["device", "list", "--json"]);
        assert!(ran.ok, "{}", ran.text());
        ran.json()["value"]["peers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Waits until this machine's one paired machine is as `wanted` says.
    async fn peer_becomes(&self, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WITHIN;
        loop {
            let peers = self.peers();
            if let [peer] = peers.as_slice()
                && wanted(peer)
            {
                return peer.clone();
            }
            assert!(Instant::now() < deadline, "{what}: {peers:?}");
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the other machine's answer crosses the connection; the list is the daemon's own account of it
        }
    }
}

/// B1 to B3: the host's window shares a link, the guest adds it at a
/// terminal, both show the same four words and the host's window allows it.
/// Returns that window, the host's device id as the guest names it and the
/// link, used once.
async fn pair_by_link(
    running: &hided::RunningDaemon,
    host: &Terminal,
    guest: &Terminal,
) -> (Window, String, String) {
    // B1: the host's window shares a link that names the words' language.
    let mut window = Window::open(running, "web").await;
    let idle = window
        .frame("pairing frame", |frame| frame["type"] == "pairing")
        .await;
    assert!(idle["payload"]["offer"].is_null(), "{idle}");
    window
        .send(
            "pairing_share",
            json!({"request_id": "s1", "language": "ko"}),
        )
        .await;
    let shared = window.result("share").await;
    assert_eq!(shared["ok"], true, "{shared}");
    assert_eq!(shared["request_id"], "s1", "{shared}");
    let link = shared["link"].as_str().unwrap().to_owned();
    assert!(link.starts_with("hide://pair#"), "{link}");

    // B2: the guest pastes it at a terminal; both sides show four words.
    let adding = guest.typing(&["device", "add", &link], "y\n");
    let asked = window
        .frame("the guest's ask", |frame| {
            frame["type"] == "pairing"
                && frame["payload"]["asks"]
                    .as_array()
                    .is_some_and(|asks| !asks.is_empty())
        })
        .await;
    let ask = &asked["payload"]["asks"][0];
    assert_eq!(ask["kind"], "link", "{ask}");
    assert_eq!(ask["device"], GUEST, "{ask}");
    let shown: Vec<String> = ask["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|word| word.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(shown.len(), 4, "{ask}");
    // Nothing is connected while the host's operator has not answered.
    assert!(host.peers().is_empty());
    window
        .send(
            "pairing_answer",
            json!({"request_id": "a1", "ask": ask["id"], "allow": true}),
        )
        .await;
    assert_eq!(window.result("answer").await["ok"], true);

    // B3: both said yes, so each lists the other, connected.
    let added = adding.await.unwrap();
    assert!(added.ok, "{}", added.text());
    assert_eq!(
        words_printed(&added),
        shown,
        "the same words, in the same order"
    );
    let on_guest = guest
        .peer_becomes("the guest opens the host", |peer| {
            peer["opens"] == "active" && peer["connected"] == true
        })
        .await;
    assert!(on_guest["opened_by"].is_null(), "{on_guest}");
    assert_eq!(on_guest["path"], "direct", "{on_guest}");
    let on_host = host
        .peer_becomes("the host is opened by the guest", |peer| {
            peer["opened_by"] == "active" && peer["connected"] == true
        })
        .await;
    assert_eq!(on_host["id"], GUEST, "{on_host}");
    assert!(on_host["opens"].is_null(), "{on_host}");
    let host_id = on_guest["id"].as_str().unwrap().to_owned();
    (window, host_id, link)
}

/// What one `hide` run answered.
struct Ran {
    ok: bool,
    out: String,
    err: String,
}

impl Ran {
    fn of(output: std::process::Output) -> Self {
        Self {
            ok: output.status.success(),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn text(&self) -> String {
        format!("{}{}", self.out, self.err)
    }

    fn json(&self) -> Value {
        let line = self.out.lines().rev().find(|line| line.starts_with('{'));
        serde_json::from_str(line.unwrap_or_else(|| panic!("no JSON line: {}", self.text())))
            .unwrap()
    }

    /// Asserts a refusal with `code` and what that code says to do.
    fn refused(&self, code: &str) {
        assert!(
            !self.ok,
            "{code} expected, the command ran: {}",
            self.text()
        );
        assert_eq!(self.json()["error"]["code"], code, "{}", self.text());
        assert_eq!(
            self.json()["error"]["message"],
            hided::pairing::next_action(code),
            "{}",
            self.text()
        );
    }
}

/// Stops the guest daemon however the test ends.
struct StopOnDrop<'a>(&'a Terminal);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.command(&["stop"]).status();
    }
}

/// `hide args` started by a shell the engine records as a pane's program,
/// with nothing in its environment that says so: what the daemon answers it.
#[cfg(unix)]
fn in_a_pane(terminal: &Terminal, args: &[&str]) -> Ran {
    let mut shell = vec![
        "-c",
        "read _ && \"$0\" \"$@\"; exit $?",
        env!("CARGO_BIN_EXE_hide"),
    ];
    shell.extend_from_slice(args);
    let mut child = terminal
        .command_of(Path::new("/bin/sh"), &shell)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let record =
        hide_kit::layout::pane_record(&terminal.state, &format!("in-a-pane-{}", child.id()));
    hide_platform::fs::private::create_dir_all(record.parent().unwrap()).unwrap();
    hide_holder_protocol::record::write(
        &record,
        &hide_holder_protocol::record::PaneRecord {
            format: hide_holder_protocol::record::RECORD_FORMAT,
            pane: "in-a-pane".to_owned(),
            owner: "fixture".to_owned(),
            incarnation: 1,
            phase: hide_holder_protocol::record::Phase::Running,
            protocol: hide_holder_protocol::PROTOCOL_VERSION,
            version_folder: "fixture".to_owned(),
            spec: "fixture".to_owned(),
            reservation: None,
            reserved_at_unix_ms: 0,
            boot: None,
            holder: None,
            program: Some(hide_holder_protocol::record::Identity::of(child.id()).unwrap()),
            end: None,
        },
    )
    .unwrap();
    child.stdin.take().unwrap().write_all(b"go\n").unwrap();
    let ran = Ran::of(child.wait_with_output().unwrap());
    std::fs::remove_file(&record).unwrap();
    ran
}

/// The words a terminal printed for its operator to compare.
fn words_printed(ran: &Ran) -> Vec<String> {
    let line = ran
        .err
        .lines()
        .find_map(|line| line.strip_prefix("Words: "))
        .unwrap_or_else(|| panic!("no words were shown: {}", ran.text()));
    line.split_whitespace().map(str::to_owned).collect()
}

// The host daemon runs on this test's runtime while a `hide` blocks a
// thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_hides_pair_by_a_link_and_stop_each_direction_apart() {
    let root = crate::fixtures::fixture_root("hp");
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

    // A machine paired with nothing lists nothing, and looking opens no
    // endpoint (D-83).
    assert!(host.peers().is_empty());
    assert!(!hided::peer_host::endpoint_wanted(&host.state).unwrap());

    // What is the operator's alone is refused where an agent runs: by this
    // command's own environment, and by what the kernel says of the caller.
    let mut from_pane = host.command(&["device", "share", "--json"]);
    from_pane.env("HIDE_PANE_ID", "w1:p1");
    Ran::of(from_pane.output().unwrap()).refused("device_from_pane");
    #[cfg(unix)]
    in_a_pane(&host, &["device", "share", "--json"]).refused("device_from_pane");

    // A screen that is not this machine's own window is refused, hears so
    // alone, and is never sent the pairings under way (D-67).
    let mut phone = Window::open(&running, "mobile").await;
    phone
        .send("pairing_share", json!({"request_id": "p1"}))
        .await;
    let refused = phone
        .frame("the refusal", |frame| frame["type"] == "refused")
        .await;
    assert_eq!(refused["payload"]["kind"], "pairing_share", "{refused}");
    assert_eq!(refused["payload"]["reason"], "not_here", "{refused}");

    let (mut window, host_id, link) = pair_by_link(&running, &host, &guest).await;

    // The link worked once (B4), and the same machine is not added twice
    // (B51).
    guest
        .run(&["device", "add", &link, "--json"])
        .refused("device_already_connected");

    // A client never names the paired machines.
    window.send("peer_machines", json!({"machines": []})).await;
    let refused = window
        .frame("a refusal", |frame| frame["type"] == "refused")
        .await;
    assert_eq!(refused["payload"]["kind"], "peer_machines", "{refused}");
    assert_eq!(host.peers().len(), 1);

    // B50: the host asks its guest to be let in the other way; the guest's
    // operator answers at a terminal, with no words to compare.
    window
        .send(
            "pairing_request",
            json!({"request_id": "r1", "device": GUEST}),
        )
        .await;
    assert_eq!(window.result("request").await["ok"], true);
    let deadline = Instant::now() + WITHIN;
    let allowed = loop {
        let ran = guest
            .typing(&["device", "allow", "--json"], "y\n")
            .await
            .unwrap();
        assert!(ran.ok, "{}", ran.text());
        if ran.json()["value"]["answered"]
            .as_array()
            .is_some_and(|answered| !answered.is_empty())
        {
            break ran;
        }
        assert!(Instant::now() < deadline, "the request never arrived");
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the request crosses the connection before the guest's terminal can list it
    };
    assert_eq!(
        allowed.json()["value"]["answered"][0]["allowed"],
        true,
        "{}",
        allowed.text()
    );
    assert!(
        !allowed.err.contains("Words:"),
        "a request over the connection compares no words: {}",
        allowed.text()
    );
    assert!(
        allowed.err.contains("Hide program and agent hooks"),
        "{}",
        allowed.text()
    );
    host.peer_becomes("each opens the other", |peer| {
        peer["opens"] == "active" && peer["opened_by"] == "active"
    })
    .await;
    guest
        .peer_becomes("each opens the other", |peer| {
            peer["opens"] == "active" && peer["opened_by"] == "active"
        })
        .await;

    // B51, B25: two directions are stopped apart, so a command that names
    // none stops nothing.
    host.run(&["device", "pause", GUEST, "--json"])
        .refused("device_direction_needed");
    let paused = host.run(&["device", "pause", GUEST, "--incoming", "--json"]);
    assert!(paused.ok, "{}", paused.text());
    host.peer_becomes("the guest's way in is paused", |peer| {
        peer["opened_by"] == "paused" && peer["opens"] == "active"
    })
    .await;
    // The guest hears it, and the other direction and the connection stay.
    guest
        .peer_becomes("the guest hears the pause", |peer| {
            peer["opens"] == "paused_there"
                && peer["opened_by"] == "active"
                && peer["connected"] == true
        })
        .await;
    let resumed = host.run(&["device", "resume", GUEST, "--incoming", "--json"]);
    assert!(resumed.ok, "{}", resumed.text());
    guest
        .peer_becomes("the guest's way in is back", |peer| {
            peer["opens"] == "active"
        })
        .await;

    // B26, B51: removing one direction leaves the other, and the connection.
    let removed = guest.run(&["device", "remove", &host_id, "--outgoing", "--json"]);
    assert!(removed.ok, "{}", removed.text());
    host.peer_becomes("the guest no longer opens the host", |peer| {
        peer["opened_by"].is_null() && peer["opens"] == "active" && peer["connected"] == true
    })
    .await;
    guest
        .peer_becomes("the host still opens the guest", |peer| {
            peer["opens"].is_null() && peer["opened_by"] == "active"
        })
        .await;

    // The guest takes the host's way in away too. The host hears that it
    // is let in nowhere, keeps the machine as one to pair again (B51), and
    // forgets it when its own operator removes it.
    let removed = guest.run(&["device", "remove", &host_id, "--json"]);
    assert!(removed.ok, "{}", removed.text());
    host.peer_becomes("the host is let in nowhere", |peer| {
        peer["opens"] == "unpaired" && peer["opened_by"].is_null()
    })
    .await;
    let removed = host.run(&["device", "remove", GUEST, "--json"]);
    assert!(removed.ok, "{}", removed.text());
    let deadline = Instant::now() + WITHIN;
    while !(host.peers().is_empty() && guest.peers().is_empty()) {
        assert!(
            Instant::now() < deadline,
            "{:?} {:?}",
            host.peers(),
            guest.peers()
        );
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the removal crosses the connection; each list is its daemon's own account
    }

    // The screen that was refused was never sent a pairing.
    drop(window);
    phone
        .send("pairing_share", json!({"request_id": "p2"}))
        .await;
    let mut pairing_frames = 0;
    loop {
        let frame = phone.frame("the refusal", |_| true).await;
        if frame["type"] == "pairing" {
            pairing_frames += 1;
        }
        if frame["type"] == "refused" {
            break;
        }
    }
    assert_eq!(pairing_frames, 0);
}

/// What a move before 1.2.0 left in the state of the core that was moved
/// to this machine: the machine it was moved off, `node`, registered as a
/// node that dials in.
fn moved_off_dials_in(state: &Path, node: &str) {
    std::fs::write(
        state.join("core-state.json"),
        json!({
            "schema_version": 1,
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "device_registrations": [{"id": node, "label": "MacBook", "inbound": true}],
        })
        .to_string(),
    )
    .unwrap();
}

/// The moved core's side of a split (PRD device-connect-iroh D-46, B45): a
/// core that a machine dials as its node takes that machine's grant in the
/// ask that gives its own, both settled at once with no question and no
/// confirm over a connection, and answers the same when asked again. The
/// same ask from a machine it runs nothing for is refused and saves
/// nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_core_takes_the_grant_of_the_node_that_dials_it_and_of_no_other_machine() {
    let root = crate::fixtures::fixture_root("hs");
    let env = host_env(root.path());
    moved_off_dials_in(&env.state_dir, "moved-off");
    let _running = hided::start_daemon(env).await.unwrap();
    let core = Terminal::new(
        root.path().join("host-home"),
        root.path().join("host-state"),
        None,
    );
    let key = hide_peer::MachineKey::generate().unwrap().public().to_hex();
    let ask = |device: &str| {
        core.run(&[
            "device",
            "accept-ssh",
            "--request",
            "split-1",
            "--key",
            &key,
            "--device",
            device,
            "--name",
            "MacBook",
            "--grant",
            "0123456789abcdef0123456789abcdef",
        ])
    };

    ask("stranger").refused("device_unknown");
    assert!(!hided::peer_host::endpoint_wanted(&core.state).unwrap());

    let taken = ask("moved-off");
    assert!(taken.ok, "{}", taken.text());
    let value = taken.json()["value"].clone();
    assert_eq!(value["adopted"], true, "{value}");
    // The answer as the machine that asked reads it (`moved_core::split`):
    // the core itself, by the key its peer records keep.
    let accepted: hide_peer::pairing::Accepted =
        serde_json::from_value(value["accepted"].clone()).expect("the core's answer reads");
    let own = hide_peer::MachineKey::load_or_create(
        &hide_kit::layout::machine_key(&core.state),
        &hide_platform::host::machine_id().unwrap(),
    )
    .unwrap()
    .0
    .public();
    assert_eq!(accepted.key, own, "{value}");
    let grant = value["accepted"]["grant"].clone();
    assert_eq!(grant, accepted.grant.as_str(), "{value}");
    let peer = core
        .peer_becomes("the node's machine is paired both ways", |peer| {
            peer["opened_by"] == "active" && !peer["opens"].is_null()
        })
        .await;
    assert_eq!(peer["id"], "moved-off", "{peer}");
    assert!(hided::peer_host::pairing_confirmed(&core.state).unwrap());

    let again = ask("moved-off");
    assert!(again.ok, "{}", again.text());
    assert_eq!(again.json()["value"]["accepted"]["grant"], grant);
    assert_eq!(core.peers().len(), 1);
}

/// A stand-in `wsl.exe` for one distribution, Ubuntu: it runs the command it
/// is given as the account the `account` file beside it names, with none of
/// this process's own `HIDE_` variables, as a distribution has none of its
/// PC's. Its text is fixed (`stand_ins`), so it is started once before a
/// test's deadline runs it.
#[cfg(unix)]
const STAND_IN_WSL: &str = r#"#!/bin/sh
[ "$1" = "--help" ] && exit 0
here=$(dirname "$0")
if [ "$1" = "--list" ]; then
    echo Ubuntu
    exit 0
fi
while [ $# -gt 0 ]; do
    case $1 in
        -d|-u|--cd) shift 2 ;;
        -e) shift; break ;;
        *) exit 64 ;;
    esac
done
for name in $(env | sed -n 's/^\(HIDE_[A-Za-z0-9_]*\)=.*/\1/p'); do
    unset "$name"
done
. "$here/account"
XDG_CONFIG_HOME=$HOME/xdg-config
HIDE_TAILSCALE_BIN=$HOME/no-tailscale
HIDE_RELAY_URL=ws://127.0.0.1:9/ws
HIDE_PEER_DIRECT=127.0.0.1:0
export HOME XDG_CONFIG_HOME HIDE_STATE_DIR HIDE_MACHINE_ID HIDE_TAILSCALE_BIN HIDE_RELAY_URL HIDE_PEER_DIRECT
unset XDG_STATE_HOME
cd "$HOME" || exit 1
exec "$@"
"#;

/// [`STAND_IN_WSL`] in `folder`, for the account `terminal` is.
#[cfg(unix)]
fn stand_in_wsl(folder: &Path, terminal: &Terminal) -> PathBuf {
    let quoted = |path: &Path| format!("'{}'", path.display());
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("account"),
        format!(
            "HOME={}\nHIDE_STATE_DIR={}\nHIDE_MACHINE_ID={}\n",
            quoted(&terminal.home),
            quoted(&terminal.state),
            terminal.machine.expect("a machine id of its own"),
        ),
    )
    .unwrap();
    let program = folder.join("wsl.exe");
    crate::stand_ins::program(&program, STAND_IN_WSL);
    program
}

/// A PC reaches the Hide its WSL distribution runs through `wsl.exe` alone
/// (PRD device-connect-iroh D-27, D-62, D-63, B43): the distribution's own
/// `hide device accept-ssh --wsl` saves the grant, `hide device carry`
/// hands its input and output to that account's core, and the core proves
/// its machine key over them before anything else is read. A core that
/// stopped with its distribution is started by the same command.
///
/// The distribution is a second daemon of this machine and `wsl.exe` a
/// stand-in that runs its commands in that account; the PC's side is the
/// daemon's own code (`pairing_wsl`, `Carriers`, the pipe transport), since
/// only a Windows daemon finds a `wsl.exe` of its own.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pc_reaches_its_distributions_hide_through_hide_device_carry() {
    use hide_node::ssh::host::wsl::WslAccess;
    use hide_peer::pipe::PipeTransport;
    use hide_peer::protocol::{Candidate, CloseCode, PeerAddr};
    use hide_peer::{MachineKey, PeerConn as _, PeerTransport as _};
    use hided::pairing_ssh::{Looked, Reach};
    use hided::pairing_wsl::{self, Carriers};

    let root = crate::fixtures::fixture_root("hw");
    let distribution = Terminal::new(
        root.path().join("wsl-home"),
        root.path().join("wsl-state"),
        Some("pairing-distribution"),
    );
    let _stop = StopOnDrop(&distribution);
    let commands = distribution.home.join(".local/bin");
    std::fs::create_dir_all(&commands).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_hide"), commands.join("hide")).unwrap();
    let found = hide_platform::wsl::Distribution {
        name: "Ubuntu".to_owned(),
        id: "25622596-db24-4e3a-9a3e-6b3f0d2e1c11".to_owned(),
        wsl_version: 2,
        installed: true,
        first_run_pending: false,
        default: true,
    };
    let access = WslAccess::with(
        stand_in_wsl(&root.path().join("pc"), &distribution),
        std::sync::Arc::new(move || Ok(vec![found.clone()])),
    );
    let reach = Reach {
        home: root.path().join("pc-home"),
        command_folder: None,
        hide: None,
    };

    // An account that never ran a Hide has none to pair with, and a look
    // starts none there.
    let (looked, asked) = (access.clone(), reach.clone());
    let absent = tokio::task::spawn_blocking(move || pairing_wsl::look(&looked, &asked, "Ubuntu"))
        .await
        .unwrap();
    assert_eq!(absent, Looked::Absent);
    let connected = distribution.run(&["connect"]);
    assert!(
        connected.ok,
        "the distribution's daemon: {}",
        connected.text()
    );

    // The PC asks the distribution's Hide for a grant, through wsl.exe.
    let mine = MachineKey::generate().unwrap();
    let (key, me) = (
        mine.public(),
        hide_peer::pairing::Me {
            name: "Windows PC".to_owned(),
            device: "windows-pc".to_owned(),
        },
    );
    let (asking, asked) = (access.clone(), reach.clone());
    let paired = tokio::task::spawn_blocking(move || {
        assert_eq!(pairing_wsl::look(&asking, &asked, "Ubuntu"), Looked::Ready);
        pairing_wsl::accept(&asking, &asked, "Ubuntu", "request-1", &me, &key)
    })
    .await
    .unwrap()
    .unwrap_or_else(|code| panic!("the distribution's Hide refused: {code}"));
    assert_eq!(paired.first, vec![Candidate::Wsl("Ubuntu".to_owned())]);
    let theirs = paired.accepted.key;
    assert_ne!(theirs, key);

    // The connection: `hide device carry` in the distribution is the
    // carrier, and its core proves the key it answered the pairing with.
    let carriers = Carriers::new(access, reach);
    let made = std::sync::Arc::clone(&carriers);
    let transport = PipeTransport::dialing(mine, move |to| {
        let distro = hide_peer::pipe::distribution(to).map(str::to_owned);
        let carriers = std::sync::Arc::clone(&made);
        async move {
            let distro = distro.ok_or_else(|| std::io::Error::other("no distribution"))?;
            tokio::task::spawn_blocking(move || carriers.carrier(&distro))
                .await
                .map_err(std::io::Error::other)?
        }
    });
    let to = PeerAddr::new(theirs, paired.first.clone());
    let reached = tokio::time::timeout(WITHIN, transport.dial(&to))
        .await
        .expect("the distribution's core answered")
        .unwrap_or_else(|error| panic!("the carrier did not reach the core: {error:?}"));
    assert_eq!(reached.peer(), theirs);
    assert!(reached.dialed());
    reached.close(CloseCode::Shutdown);

    // A program in a pane of that Hide is no operator: it gets no carrier,
    // and nothing is written where a connection's bytes would go.
    let mut from_a_pane = distribution.command(&["device", "carry"]);
    let refused = from_a_pane.env("HIDE_PANE_ID", "w1:p1").output().unwrap();
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty(), "{:?}", refused.stdout);
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("device_from_pane"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    // A key the core does not hold proves nothing: the dial ends.
    let other = MachineKey::generate().unwrap().public();
    let wrong = tokio::time::timeout(
        WITHIN,
        transport.dial(&PeerAddr::new(other, paired.first.clone())),
    )
    .await
    .expect("a dial to another key ended");
    assert!(
        wrong.is_err(),
        "another machine's key was taken for this core's"
    );

    // The core stopped, as it does with a distribution that has no
    // systemd: the next carrier starts it, since this account ran one.
    let stopped = distribution.run(&["stop"]);
    assert!(stopped.ok, "{}", stopped.text());
    let again = tokio::time::timeout(WITHIN, transport.dial(&to))
        .await
        .expect("the started core answered")
        .unwrap_or_else(|error| panic!("the carrier did not start the core: {error:?}"));
    assert_eq!(again.peer(), theirs);
    again.close(CloseCode::Shutdown);
    transport.close().await;
}

/// B7 for a machine that already runs its own Hide, reached over SSH: the
/// guest's `hide device add <alias>` looks through a real `sshd`, pairs by
/// the target's `hide device accept-ssh` with no words to compare, and
/// confirms over the connection. Nothing is installed on the target, and
/// adding the same machine again says it is already connected (B51).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
async fn a_machine_reached_over_ssh_pairs_with_the_hide_it_already_runs() {
    use crate::support::sshd::Sshd;

    let root = crate::fixtures::fixture_root("hp");
    let _running = hided::start_daemon(host_env(root.path())).await.unwrap();
    let target = Terminal::new(
        root.path().join("host-home"),
        root.path().join("host-state"),
        None,
    );
    let guest = Terminal::new(
        root.path().join("guest-home"),
        root.path().join("guest-state"),
        Some(GUEST),
    );
    // What an SSH login to the target's account finds: its home, its state
    // folder, and the `hide` command its own Hide linked.
    let ssh = Sshd::start(
        &root.path().join("sshd"),
        &[
            (hided::env::HOME.into(), target.home.clone().into()),
            ("HIDE_STATE_DIR".into(), target.state.clone().into()),
        ],
    )
    .unwrap();
    std::fs::create_dir_all(guest.home.join(".ssh")).unwrap();
    std::fs::write(guest.home.join(".ssh/config"), ssh.client_config("studio")).unwrap();
    let _stop = StopOnDrop(&guest);
    let connected = guest.run(&["connect"]);
    assert!(connected.ok, "the guest daemon: {}", connected.text());

    // The target's own command, as the guest's daemon runs it over SSH: a
    // look changes nothing, and the target stays paired with nothing.
    let commands = target.home.join(".local/bin");
    std::fs::create_dir_all(&commands).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_hide"), commands.join("hide")).unwrap();
    let looked = target.run(&["device", "look-ssh"]);
    assert!(looked.ok, "{}", looked.text());
    assert_eq!(
        looked.json()["value"]["look"]["core"],
        true,
        "{}",
        looked.text()
    );
    assert!(!hided::peer_host::endpoint_wanted(&target.state).unwrap());

    // One command pairs: no link, no words.
    let added = guest.run(&["device", "add", "studio", "--json"]);
    assert!(added.ok, "{}", added.text());
    let value = added.json()["value"].clone();
    assert_eq!(value["paired"], "ssh", "{value}");
    let opened = guest
        .peer_becomes("the guest opens the target", |peer| {
            peer["opens"] == "active" && peer["connected"] == true
        })
        .await;
    assert_eq!(opened["id"], value["device_id"], "{opened}");
    assert!(opened["opened_by"].is_null(), "{opened}");
    let opener = target
        .peer_becomes("the target is opened by the guest", |peer| {
            peer["opened_by"] == "active"
        })
        .await;
    assert_eq!(opener["id"], GUEST, "{opener}");
    assert!(opener["opens"].is_null(), "{opener}");

    // Nothing of the guest's was installed in the target's account: no
    // device helper, and the command folder holds only the target's own.
    assert!(!target.home.join(".hide").join("host-helper").exists());
    let linked: Vec<_> = std::fs::read_dir(&commands)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(linked, ["hide"]);

    // The same machine added again is the one it already is, and its key,
    // proved over the alias, keeps the alias as its SSH route.
    let again = guest.run(&["device", "add", "studio", "--json"]);
    assert!(!again.ok, "{}", again.text());
    assert_eq!(again.json()["error"]["code"], "device_already_connected");
    assert!(
        again.json()["error"]["message"]
            .as_str()
            .is_some_and(|said| said.contains("connect over SSH through studio")),
        "{}",
        again.text()
    );
    assert_eq!(guest.peers().len(), 1);

    let removed = guest.run(&["device", "remove", "studio", "--json"]);
    assert!(
        !removed.ok,
        "an alias names no paired machine: {}",
        removed.text()
    );
    let removed = guest.run(&[
        "device",
        "remove",
        value["device_id"].as_str().unwrap(),
        "--json",
    ]);
    assert!(removed.ok, "{}", removed.text());
    ssh.stop().unwrap();
}

/// Two Hides with no direct path and no relay between them connect over the
/// SSH alias the guest paired through, and both say the path is SSH:
/// the guest's first dial runs the ladder, the far account's
/// `hide device carry --ssh` hands its stdio to the target's daemon, and the
/// pending grant is confirmed over that connection.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
async fn two_hides_with_no_other_way_connect_over_ssh() {
    let pair = SshOnly::connected().await;
    let listed = pair.guest.run(&["device", "list"]);
    assert!(
        listed.text().contains("connected · SSH"),
        "{}",
        listed.text()
    );
    // The alias is the guest's own: the target's records never name it.
    let records =
        std::fs::read_to_string(hide_kit::layout::peer_grants(&pair.target.state)).unwrap();
    assert!(!records.contains("studio"), "{records}");
}

/// A cut SSH carrier is dialed again: ending only the carrier's `ssh` on
/// the guest ends the connection, and with no press the pair is back on
/// SSH on both machines, after the route's wait, since the connection that
/// fell was short.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
async fn a_cut_ssh_carrier_is_dialed_again() {
    let pair = SshOnly::connected().await;
    let carriers = pair.carrier_ssh();
    assert_eq!(carriers.len(), 1, "one carrier ssh: {carriers:?}");
    let killed = Command::new("kill")
        .args(["-TERM", &carriers[0].to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    pair.guest
        .peer_becomes("the guest sees the cut", |peer| peer["connected"] == false)
        .await;
    // A connection that ended within `STAYED_AFTER` puts the route into
    // its first wait (`CLIMB_FIRST`, 30 s); the keeper's redials, doubling
    // from 1 s, reach the due route at about 31 s, and a due route gives the
    // other transport `DIRECT_WITHIN` (4 s) before it signs in: 31-35 s,
    // and 12 s more for the sign-in and both lists reading it on a loaded
    // runner.
    let deadline = Instant::now()
        + hide_peer::limits::CLIMB_FIRST
        + hide_peer::limits::DIRECT_WITHIN
        + Duration::from_secs(12);
    for (side, terminal) in [("guest", &pair.guest), ("target", &pair.target)] {
        loop {
            let peers = terminal.peers();
            if let [peer] = peers.as_slice()
                && peer["connected"] == true
                && peer["path"] == "ssh"
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the {side} is not back on SSH: {peers:?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await; // wait-ok: the other machine's answer crosses the connection; the list is the daemon's own account of it
        }
    }
    assert_eq!(pair.carrier_ssh().len(), 1, "one carrier again");
}

/// Two Hides with no IP path and an unreachable relay, connected over SSH:
/// the target's daemon behind a real `sshd`, the guest's started by its
/// `hide connect`, paired by `hide device add studio`. Its drop stops the
/// guest's daemon and the `sshd`.
#[cfg(unix)]
struct SshOnly {
    target: Terminal,
    guest: Terminal,
    ssh: crate::support::sshd::Sshd,
    _running: hided::RunningDaemon,
    _root: tempfile::TempDir,
}

#[cfg(unix)]
impl SshOnly {
    async fn connected() -> Self {
        use crate::support::sshd::Sshd;

        let root = crate::fixtures::fixture_root("hq");
        let mut env = host_env(root.path());
        env.peer_direct = None;
        env.peer_relay = Some(NO_RELAY.to_owned());
        env.peer_relay_only = true;
        let running = hided::start_daemon(env).await.unwrap();
        let target = Terminal::new(
            root.path().join("host-home"),
            root.path().join("host-state"),
            None,
        )
        .with_no_direct_path();
        let guest = Terminal::new(
            root.path().join("guest-home"),
            root.path().join("guest-state"),
            Some(GUEST),
        )
        .with_no_direct_path();
        let ssh = Sshd::start(
            &root.path().join("sshd"),
            &[
                (hided::env::HOME.into(), target.home.clone().into()),
                ("HIDE_STATE_DIR".into(), target.state.clone().into()),
            ],
        )
        .unwrap();
        std::fs::create_dir_all(guest.home.join(".ssh")).unwrap();
        std::fs::write(guest.home.join(".ssh/config"), ssh.client_config("studio")).unwrap();
        let pair = Self {
            target,
            guest,
            ssh,
            _running: running,
            _root: root,
        };
        let connected = pair.guest.run(&["connect"]);
        assert!(connected.ok, "the guest daemon: {}", connected.text());
        let commands = pair.target.home.join(".local/bin");
        std::fs::create_dir_all(&commands).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_hide"), commands.join("hide")).unwrap();

        let added = pair.guest.run(&["device", "add", "studio", "--json"]);
        assert!(added.ok, "{}", added.text());
        assert_eq!(added.json()["value"]["paired"], "ssh", "{}", added.text());
        let opened = pair
            .guest
            .peer_becomes("the guest opens the target over SSH", |peer| {
                peer["opens"] == "active" && peer["connected"] == true
            })
            .await;
        assert_eq!(opened["path"], "ssh", "{opened}");
        let opener = pair
            .target
            .peer_becomes("the target is opened by the guest over SSH", |peer| {
                peer["opened_by"] == "active" && peer["connected"] == true
            })
            .await;
        assert_eq!(opener["path"], "ssh", "{opener}");
        pair
    }

    /// The `ssh` processes the guest's daemon runs to the target: the ones
    /// that read the guest's own SSH configuration.
    fn carrier_ssh(&self) -> Vec<u32> {
        let config = self.guest.home.join(".ssh/config");
        let config = config.to_string_lossy();
        let listed = Command::new("ps")
            .args(["-A", "-o", "pid=,command="])
            .output()
            .unwrap();
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, command) = line.trim().split_once(' ')?;
                let program = command.split_whitespace().next()?;
                (program.ends_with("/ssh") || program == "ssh")
                    .then_some(())
                    .filter(|()| command.contains(config.as_ref()))?;
                pid.parse().ok()
            })
            .collect()
    }
}

#[cfg(unix)]
impl Drop for SshOnly {
    fn drop(&mut self) {
        let _ = self.guest.command(&["stop"]).status();
        let _ = self.ssh.stop();
    }
}

/// A node is split from its core only once the two have met on the
/// connection two Hides use, and until the split commits its core goes on
/// taking it as the node it was (PRD device-connect-iroh B45). The core's
/// Hide answers both asks, as over an SSH login that works. While the new
/// connection cannot be made, and again when the two met and this machine
/// then could not set its record aside, the record is where it was, the
/// core has confirmed nothing with the machine and still lists it as one
/// that dials in, so its link over SSH is taken as before; the same split
/// asked again then commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_split_that_stops_before_its_commit_leaves_the_node_its_core_takes() {
    use hide_peer::pairing::{Accepted, Me};
    use hide_peer::protocol::{Candidate, PeerAddr, PeerKey};
    use hided::moved_core::{Splitting, split};

    let root = crate::fixtures::fixture_root("hs");
    let env = host_env(root.path());
    moved_off_dials_in(&env.state_dir, "moved-off");
    let running = hided::start_daemon(env).await.unwrap();
    let core = Terminal::new(
        root.path().join("host-home"),
        root.path().join("host-state"),
        None,
    );
    let core_node = hide_core::node::NodeId::of_this_machine()
        .unwrap()
        .to_string();

    // The machine the core was moved off: its state folder with the record
    // that names its core, and no daemon, since the split is what is asked.
    let state = root.path().join("node-state");
    hide_platform::fs::private::create_dir_all(&state).unwrap();
    let home = root.path().join("node-home");
    crate::fixtures::fixture_home(&home);
    hided::placement::write(
        &state,
        &hided::placement::Placement {
            alias: "studio".to_owned(),
            node: core_node.clone(),
            program: "/opt/hide/hided".to_owned(),
            state_dir: None,
            disconnected: false,
        },
    )
    .unwrap();
    let machine = hide_platform::host::machine_id().unwrap();
    let me = Me {
        device: "moved-off".to_owned(),
        name: "MacBook".to_owned(),
    };
    let splitting = || Splitting {
        home: &home,
        state_dir: &state,
        machine: &machine,
        me: &me,
        core: &core_node,
        panes: hide_kit::layout::PanesInstall {
            id: None,
            cli: None,
        },
    };
    // The core's own `hide`, as the node runs it over the SSH login.
    let ask = |request: &str, key: &PeerKey, grant: Option<&str>| -> Result<Value, &'static str> {
        let key = key.to_hex();
        let mut words = vec![
            "device",
            "accept-ssh",
            "--request",
            request,
            "--key",
            &key,
            "--device",
            "moved-off",
            "--name",
            "MacBook",
        ];
        words.extend(grant.into_iter().flat_map(|grant| ["--grant", grant]));
        let ran = core.run(&words);
        match ran.ok {
            true => Ok(ran.json()["value"].clone()),
            false => panic!("the core refused: {}", ran.text()),
        }
    };
    let endpoint = hided::peer_host::Settings {
        state_dir: state.clone(),
        build: "1.3.0-test".to_owned(),
        device: "moved-off".to_owned(),
        name: None,
        release: false,
        direct: Some("127.0.0.1:0".parse().unwrap()),
        relay: None,
        relay_only: false,
        wsl: None,
        ssh: None,
    };
    // The split runs where the node role runs it: on a thread no runtime
    // drives.
    let splits = |meet: &(dyn Fn(&Accepted) -> Result<(), String> + Sync)| {
        tokio::task::block_in_place(|| {
            std::thread::scope(|scope| {
                scope
                    .spawn(|| split(&splitting(), &ask, meet))
                    .join()
                    .unwrap()
            })
        })
    };
    // What the core's own screen and terminal say of the machine: nothing
    // confirmed with it, and still a node that dials in.
    async fn still_its_node(
        running: &hided::RunningDaemon,
        core: &Terminal,
        state: &Path,
        when: &str,
    ) {
        assert!(
            hided::placement::read(state, "moved-off")
                .unwrap()
                .is_some(),
            "{when}: the record was set aside"
        );
        let peers = core.peers();
        let [peer] = peers.as_slice() else {
            panic!("{when}: {peers:?}");
        };
        assert_eq!(peer["opens"], "confirming", "{when}: {peer}");
        assert_eq!(peer["opened_by"], "active", "{when}: {peer}");
        let mut window = Window::open(running, "web").await;
        let shown = window
            .frame("the core's snapshot", |frame| frame["type"] == "snapshot")
            .await;
        let devices = &shown["payload"]["rest"]["navigator"]["devices"];
        let node = devices
            .as_array()
            .into_iter()
            .flatten()
            .find(|device| device["id"] == "moved-off")
            .unwrap_or_else(|| panic!("{when}: {devices}"));
        assert_eq!(node["dials_in"], true, "{when}: {devices}");
    }

    // The SSH login works and the new connection cannot be made: a port
    // that was free a moment ago, with nothing bound to it.
    let free = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let nowhere = free.local_addr().unwrap();
    drop(free);
    let apart = |host: &Accepted| {
        hided::peer_host::meets(
            &endpoint,
            &PeerAddr::new(host.key, vec![Candidate::Ip(nowhere)]),
            Duration::from_millis(500),
        )
    };
    let failure = splits(&apart).unwrap_err();
    assert_eq!(failure.code, "split_not_connected", "{failure:?}");
    still_its_node(&running, &core, &state, "the connection could not be made").await;

    // The two meet, and this machine then cannot set its record aside.
    let meets = |host: &Accepted| {
        hided::peer_host::meets(
            &endpoint,
            &PeerAddr::new(host.key, host.candidates.clone()),
            WITHIN,
        )
    };
    let kept = hide_kit::layout::moved_out(&state);
    std::fs::write(&kept, b"").unwrap();
    let failure = splits(&meets).unwrap_err();
    assert_eq!(failure.code, "split_not_saved", "{failure:?}");
    still_its_node(&running, &core, &state, "the record could not be set aside").await;

    // Asked again, the same split commits; the core confirms with the
    // machine only once that machine runs a core, which none does here.
    std::fs::remove_file(&kept).unwrap();
    splits(&meets).unwrap();
    assert!(
        hided::placement::read(&state, "moved-off")
            .unwrap()
            .is_none()
    );
    assert!(kept.join("core-placement.json").is_file());
    assert_eq!(core.peers().len(), 1);
}
