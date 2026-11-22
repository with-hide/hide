//! Two Hides pair (PRD device-connect-iroh c1, c2): a host whose window
//! shares a link and answers, and a guest that is a second daemon with a
//! machine id of its own, driven by the built `hide` at a terminal. They
//! pair by the link and the same four words, the host asks to be let in the
//! other way, each direction is stopped and removed apart, and what is the
//! operator's alone is refused to a pane and to a screen that is not this
//! machine's own window (B52).
//!
//! The two endpoints bind loopback with no relay (`HIDE_PEER_DIRECT`), so
//! nothing leaves the machine. Both builds offer connections between two
//! Hides (`HIDE_DEVICE_CONNECT=1`); one that does not refuses all of it.

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

#[cfg(unix)]
mod core_letters;
#[cfg(unix)]
mod guest_work;

/// The machine id the guest daemon runs under, so one host is two machines.
const GUEST: &str = "pairing-guest";

const WITHIN: Duration = Duration::from_secs(40);

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
        device_connect: true,
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
        }
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
            .env("HIDE_PEER_DIRECT", "127.0.0.1:0")
            .env(hided::env::HIDE_DEVICE_CONNECT, "1")
            .env("LANG", "en_US.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(machine) = self.machine {
            command.env(hided::env::HIDE_MACHINE_ID, machine);
        }
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
    assert!(!hided::peer_host::endpoint_wanted(&host.state, true).unwrap());

    // What is the operator's alone is refused where an agent runs: by this
    // command's own environment, and by what the kernel says of the caller.
    let mut from_pane = host.command(&["device", "share", "--json"]);
    from_pane.env("HIDE_PANE_ID", "w1:p1");
    Ran::of(from_pane.output().unwrap()).refused("device_from_pane");
    #[cfg(unix)]
    in_a_pane(&host, &["device", "share", "--json"]).refused("device_from_pane");

    // A screen that is not this machine's own window is refused, hears so
    // alone, and is never sent the pairings under way.
    let mut phone = Window::open(&running, "mobile").await;
    phone
        .send("pairing_share", json!({"request_id": "p1"}))
        .await;
    let refused = phone.result("share").await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["reason"], "device_from_other_screen", "{refused}");

    // B1: the host's window shares a link that names the words' language.
    let mut window = Window::open(&running, "web").await;
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

    // The link worked once (B4), and the same machine is not added twice
    // (B51).
    guest
        .run(&["device", "add", &link, "--json"])
        .refused("device_already_connected");

    // A client never names the paired machines.
    window.send("peer_machines", json!({"machines": []})).await;
    let error = window
        .frame("an error", |frame| frame["type"] == "error")
        .await;
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("sent by the daemon only"),
        "{error}"
    );
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
        if frame["type"] == "pairing_result" && frame["payload"]["request_id"] == "p2" {
            break;
        }
    }
    assert_eq!(pairing_frames, 0);
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
HIDE_DEVICE_CONNECT=1
export HOME XDG_CONFIG_HOME HIDE_STATE_DIR HIDE_MACHINE_ID HIDE_TAILSCALE_BIN HIDE_RELAY_URL HIDE_PEER_DIRECT HIDE_DEVICE_CONNECT
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
            (hided::env::HIDE_DEVICE_CONNECT.into(), "1".into()),
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
    assert!(!hided::peer_host::endpoint_wanted(&target.state, true).unwrap());

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

    // The same machine added again is the one it already is.
    guest
        .run(&["device", "add", "studio", "--json"])
        .refused("device_already_connected");
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

/// A build that does not offer connections between two Hides (no
/// `HIDE_DEVICE_CONNECT`, every build until that feature ships) says so in
/// its hello, so its window shows no pairing, and refuses every pairing ask
/// from a window and from a terminal, opening nothing, and its help leaves
/// the pairing verbs out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_build_without_connections_between_hides_refuses_pairing_and_says_so_to_its_window() {
    let root = crate::fixtures::fixture_root("hp");
    let running = hided::start_daemon(Env {
        device_connect: false,
        ..host_env(root.path())
    })
    .await
    .unwrap();
    let host = Terminal {
        home: root.path().join("host-home"),
        state: root.path().join("host-state"),
        machine: None,
    };

    let mut window = Window::open(&running, "web").await;
    let hello = window
        .frame("the daemon frame", |frame| frame["type"] == "daemon")
        .await;
    assert_eq!(hello["payload"]["device_connect"], false, "{hello}");
    window
        .send(
            "pairing_share",
            json!({"request_id": "s1", "language": "en"}),
        )
        .await;
    let refused = window.result("share").await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["reason"], "device_connect_unavailable", "{refused}");

    // The terminal's own build refuses before it asks; one that offers
    // pairing hears the daemon's refusal, which is the same.
    let mut off = host.command(&["device", "share", "--json"]);
    off.env_remove(hided::env::HIDE_DEVICE_CONNECT);
    Ran::of(off.output().unwrap()).refused("device_connect_unavailable");
    host.run(&["device", "pause", "studio", "--json"])
        .refused("device_connect_unavailable");
    // A carrier is refused too, by the terminal's own build and by the
    // daemon one that offers them asks; nothing is written where a
    // connection's bytes would go.
    let mut off = host.command(&["device", "carry"]);
    off.env_remove(hided::env::HIDE_DEVICE_CONNECT);
    for refused in [
        off.output().unwrap(),
        host.command(&["device", "carry"]).output().unwrap(),
    ] {
        assert!(!refused.status.success());
        assert!(refused.stdout.is_empty(), "{:?}", refused.stdout);
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("device_connect_unavailable"),
            "{}",
            String::from_utf8_lossy(&refused.stderr)
        );
    }

    // Its help leaves the pairing verbs out; a build that offers them
    // lists them.
    let help = |command: &mut std::process::Command| {
        let out = command.output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let mut off = host.command(&["--help"]);
    off.env_remove(hided::env::HIDE_DEVICE_CONNECT);
    assert!(!help(&mut off).contains("hide device share"));
    assert!(help(&mut host.command(&["--help"])).contains("hide device share"));

    // Nothing was opened for it.
    assert!(!hided::peer_host::endpoint_wanted(&host.state, false).unwrap());
    assert!(!hide_kit::layout::machine_key(&host.state).exists());
}
