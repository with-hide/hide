//! A bot's journey (PRD hide-external-control c1, c2; mission-87): a fake
//! tool, the built `hide` run with a home of its own and no pane identity,
//! asks to connect, the operator's window approves it in a mode with the
//! event Settings sends, and the same request run again hands the tool its
//! key. It then runs what its mode allows and is refused the rest with a
//! code and what to do; a refusal, a disconnect, its own leave, a restart,
//! a damaged key and a connection an earlier build approved View only each
//! answer as the tool can act on.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::SinkExt;
use hided::env::Env;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn daemon_env(root: &Path) -> Env {
    let state = root.join("state");
    hide_platform::fs::private::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(root.join("operator")).unwrap();
    Env {
        home: root.join("operator"),
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
        peer_direct: None,
        device_connect: false,
        search_path: None,
    }
}

/// The operator's window: a web client, the one kind of connection that may
/// approve, refuse, change the mode of and disconnect a bot.
pub(crate) struct Window(Socket);

impl Window {
    pub(crate) async fn open(running: &hided::RunningDaemon) -> Self {
        Self::open_as(running, Some("web")).await
    }

    /// A screen connection of `kind`; only a `web` or `desktop` one is a
    /// window, and one that names no kind draws nothing.
    async fn open_as(running: &hided::RunningDaemon, kind: Option<&str>) -> Self {
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
        socket
            .send(Message::Text(
                {
                    let mut hello = json!({"token": running.token, "schema_version": 2});
                    if let Some(kind) = kind {
                        hello["client_kind"] = json!(kind);
                    }
                    hello
                }
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        Self(socket)
    }

    pub(crate) async fn send(&mut self, kind: &str, payload: Value) {
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

    /// The message of the next error the daemon answers this connection.
    async fn error(&mut self) -> String {
        use futures_util::StreamExt;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let frame = tokio::time::timeout(left, self.0.next())
                .await
                .expect("no error came")
                .expect("the connection closed")
                .unwrap();
            let Message::Text(text) = frame else { continue };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "error" {
                return value["message"].as_str().unwrap().to_owned();
            }
        }
    }
}

/// A shell recorded as a pane's program, waiting to start `hide`.
#[cfg(unix)]
struct InAPane {
    child: std::process::Child,
    record: PathBuf,
}

#[cfg(unix)]
impl InAPane {
    /// Lets the shell start `hide`, and what it answered.
    fn finish(mut self) -> Ran {
        use std::io::Write;
        self.child.stdin.take().unwrap().write_all(b"go\n").unwrap();
        let output = self.child.wait_with_output().unwrap();
        std::fs::remove_file(&self.record).unwrap();
        Ran::of(output)
    }
}

/// A fake tool: the built `hide`, run with its own home, in that home, with
/// every `HIDE_*` of this process removed but the state folder
/// of the daemon it asks.
pub(crate) struct Tool {
    home: PathBuf,
    state: PathBuf,
}

/// What one `hide` run answered.
pub(crate) struct Ran {
    pub(crate) ok: bool,
    pub(crate) out: String,
}

impl Ran {
    fn of(output: std::process::Output) -> Self {
        Self {
            ok: output.status.success(),
            out: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    }

    /// The JSON line the run printed last.
    pub(crate) fn json(&self) -> Value {
        let line = self.out.lines().rev().find(|line| line.starts_with('{'));
        serde_json::from_str(line.unwrap_or_else(|| panic!("no JSON line: {}", self.out))).unwrap()
    }

    /// Asserts a refusal with `code` and the next action that code says.
    fn refused(&self, code: &str) {
        assert!(!self.ok, "{code} expected, the command ran: {}", self.out);
        assert!(self.out.contains(code), "{code} expected: {}", self.out);
        if let Some(next) = hided::access::next_action(code) {
            assert!(
                self.out.contains(next),
                "{code}'s next action: {}",
                self.out
            );
        }
    }
}

impl Tool {
    pub(crate) fn new(root: &Path, name: &str) -> Self {
        let home = root.join(name);
        std::fs::create_dir_all(&home).unwrap();
        Self {
            home,
            state: root.join("state"),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_of(Path::new(env!("CARGO_BIN_EXE_hide")), args)
    }

    fn command_of(&self, program: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(program);
        for (key, _) in std::env::vars_os() {
            let upper = key.to_string_lossy().to_uppercase();
            if upper.starts_with("HIDE_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .current_dir(&self.home)
            // The home the platform names: `USERPROFILE` on Windows.
            .env(hided::env::HOME, &self.home)
            .env_remove("XDG_STATE_HOME")
            .env("HIDE_STATE_DIR", &self.state)
            .stdin(Stdio::null());
        command
    }

    pub(crate) fn run(&self, args: &[&str]) -> Ran {
        self.run_with(args, &[])
    }

    /// `hide args` run in `folder`, as a tool runs it from a project.
    fn run_in(&self, folder: &Path, args: &[&str]) -> Ran {
        Ran::of(self.command(args).current_dir(folder).output().unwrap())
    }

    fn run_with(&self, args: &[&str], env: &[(&str, &str)]) -> Ran {
        let mut command = self.command(args);
        for (key, value) in env {
            command.env(key, value);
        }
        Ran::of(command.output().unwrap())
    }

    /// `hide args` started by a shell the core's engine records as a pane's
    /// program, as an agent in a pane that cleared its environment runs it:
    /// the shell waits until [`InAPane::finish`], so its record can be read
    /// meanwhile.
    #[cfg(unix)]
    fn in_a_pane(&self, args: &[&str]) -> InAPane {
        // `exit` after `hide` keeps the shell from becoming it, so `hide`
        // is the shell's child.
        let mut shell = vec![
            "-c",
            "read _ && \"$0\" \"$@\"; exit $?",
            env!("CARGO_BIN_EXE_hide"),
        ];
        shell.extend_from_slice(args);
        let child = self
            .command_of(Path::new("/bin/sh"), &shell)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let record =
            hide_kit::layout::pane_record(&self.state, &format!("in-a-pane-{}", child.id()));
        hide_platform::fs::private::create_dir_all(record.parent().unwrap()).unwrap();
        let program = hide_holder_protocol::record::Identity::of(child.id()).unwrap();
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
                program: Some(program),
                end: None,
            },
        )
        .unwrap();
        InAPane { child, record }
    }

    fn key_file(&self, name: &str, extension: &str) -> PathBuf {
        hide_kit::layout::access_keys(&self.home).join(format!("{name}.{extension}"))
    }

    /// `hide access request --name <name> --no-wait --json` until the
    /// answer is not pending, as a script polls after the operator was asked.
    pub(crate) async fn request_until_answered(&self, name: &str) -> Ran {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let ran = self.run(&["access", "request", "--name", name, "--no-wait", "--json"]);
            if !(ran.ok && ran.json()["value"]["state"] == "pending") {
                return ran;
            }
            assert!(Instant::now() < deadline, "still pending: {}", ran.out);
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's approval reached the core; the tool's next request reads it
        }
    }

    /// Requests without waiting and returns the pending request's id (B1).
    pub(crate) fn put_up(&self, name: &str) -> String {
        let ran = self.run(&["access", "request", "--name", name, "--no-wait", "--json"]);
        assert!(ran.ok, "{}", ran.out);
        let value = &ran.json()["value"];
        assert_eq!(value["state"], "pending", "{}", ran.out);
        assert!(
            value["next_action"]
                .as_str()
                .unwrap()
                .contains(&format!("hide access request --name {name}")),
            "{}",
            ran.out
        );
        value["id"].as_str().unwrap().to_owned()
    }
}

/// c1: request, the window's approval, the key on the request run again;
/// a bot of the whole Hide runs the reads and the commands of B21, letters
/// and inbox among them; devices are the operator's to add or remove; and
/// B22, B23 are refused with their codes.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bot_asks_is_approved_for_the_whole_hide_and_runs_what_the_operator_does() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let mut window = Window::open(&running).await;
    let tool = Tool::new(root.path(), "tool-home");

    let id = tool.put_up("nightly");
    // Asked again, the same request, not a second one (B5, B9).
    assert_eq!(tool.put_up("nightly"), id);
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    let connected = tool.request_until_answered("nightly").await;
    assert!(connected.ok, "{}", connected.out);
    assert_eq!(connected.json()["value"]["state"], "connected");
    assert_eq!(connected.json()["value"]["mode"], json!({"kind": "all"}));
    let key = tool.key_file("nightly", "key");
    assert!(
        hide_platform::fs::private::is_private(&key).unwrap(),
        "the key file is open to others"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the key file is {mode:o}");
    }
    assert!(!tool.key_file("nightly", "request").exists());
    let human = tool.run(&["access", "request", "--name", "nightly"]);
    assert!(
        human.out.contains("Connected as nightly · whole Hide"),
        "{}",
        human.out
    );

    for args in [
        &["access", "status", "--json"][..],
        &["access", "list", "--json"],
        &["ps", "--json"],
        &["agent", "list"],
        &["mission", "list"],
        &["device", "list", "--json"],
        &["inbox"],
    ] {
        let ran = tool.run(args);
        assert!(ran.ok, "{args:?}: {}", ran.out);
    }
    let list = tool.run(&["access", "list", "--json"]).json();
    assert_eq!(list["value"]["connections"][0]["name"], "nightly");
    assert_eq!(
        list["value"]["connections"][0]["mode"],
        json!({"kind": "all"})
    );
    // A letter goes as far as the recipient: there is none, and the core
    // says so, not the mode.
    let send = tool.run(&[
        "request", "send", "someone", "--intent", "look", "--body", "please",
    ]);
    assert!(
        !send.ok && send.out.contains("target_unavailable"),
        "{}",
        send.out
    );
    assert!(!send.out.contains("access_"), "{}", send.out);
    // Adding or removing a device stays the operator's (operator decision
    // 2026-11-21).
    tool.run(&["device", "add", "mini"])
        .refused("access_operator_only");
    tool.run(&["device", "remove", "mini"])
        .refused("access_operator_only");

    // B22, B23.
    tool.run(&[
        "agent", "spawn", "--parent", "here", "--name", "helper", "--intent", "help", "--kind",
        "codex",
    ])
    .refused("access_spawn_parent");
    for args in [
        &["mission", "create", "--title", "tool's"][..],
        &["agent", "show", "here"],
        &["agent", "ask-human", "auth", "github"],
        &["task", "node", "one", "--status", "done"],
    ] {
        tool.run(args).refused("access_needs_agent");
    }
    running.stop();
}

/// c2: a refusal (B7), a disconnect (B13), the tool's own leave (B14), a
/// request over the pending cap (B9), a restart (B9, B38), a damaged key and
/// a stopped Hide (B32), and a pane's own caller (B8, B19).
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refusals_disconnects_leave_restart_and_a_damaged_key_each_say_what_to_do() {
    let root = tempfile::tempdir().unwrap();
    let env = daemon_env(root.path());
    let running = hided::start_daemon(env.clone()).await.unwrap();
    let mut window = Window::open(&running).await;

    // B7: refused, the next request says so and drops the key; the one after
    // asks anew.
    let refused = Tool::new(root.path(), "refused-home");
    let first = refused.put_up("refused");
    window.send("access_refuse", json!({"id": first})).await;
    let answer = refused.request_until_answered("refused").await;
    answer.refused("access_refused");
    assert!(!refused.key_file("refused", "request").exists());
    assert_ne!(refused.put_up("refused"), first, "a new request");

    // B13: disconnected, its next command is refused as such.
    let revoked = Tool::new(root.path(), "revoked-home");
    let id = revoked.put_up("revoked");
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert!(revoked.request_until_answered("revoked").await.ok);
    window.send("access_revoke", json!({"id": id})).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ran = revoked.run(&["inbox"]);
        if !ran.ok {
            ran.refused("access_revoked");
            break;
        }
        assert!(Instant::now() < deadline, "still connected: {}", ran.out);
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's disconnect reached the core; the tool's next command reads it
    }

    // B14: the tool leaves on its own; its key goes and so does its row.
    let leaving = Tool::new(root.path(), "leaving-home");
    let id = leaving.put_up("leaving");
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert!(leaving.request_until_answered("leaving").await.ok);
    let left = leaving.run(&["access", "leave", "--json"]);
    assert!(left.ok, "{}", left.out);
    assert_eq!(left.json()["value"]["state"], "left");
    assert!(!leaving.key_file("leaving", "key").exists());
    let status = leaving.run(&["access", "status", "--json"]).json();
    assert_eq!(status["value"]["state"], "not_connected");

    // B9: a fifth waiting name drops the oldest, which hears it expired.
    let waiting: Vec<Tool> = (0..5)
        .map(|n| Tool::new(root.path(), &format!("waiting-{n}")))
        .collect();
    for (n, tool) in waiting.iter().enumerate() {
        tool.put_up(&format!("waiting-{n}"));
    }
    waiting[0]
        .run(&["access", "status", "--json"])
        .refused("access_request_expired");

    // A connected tool for the restart.
    let kept = Tool::new(root.path(), "kept-home");
    let id = kept.put_up("kept");
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert!(kept.request_until_answered("kept").await.ok);
    drop(window);
    // Dropped, not only stopped: the drop saves the core's state and lets go
    // of the state folder's lock, as a daemon's exit does.
    drop(running);

    // B32: no Hide runs.
    let stopped = kept.run(&["access", "status", "--json"]);
    assert!(
        !stopped.ok && stopped.out.contains("hide_unavailable"),
        "{}",
        stopped.out
    );

    // B9, B38: after a restart the approved tool is still connected, and a
    // request that was waiting hears it expired.
    let running = hided::start_daemon(env).await.unwrap();
    let status = kept.run(&["access", "status", "--json"]);
    assert!(status.ok, "{}", status.out);
    assert_eq!(status.json()["value"]["state"], "connected");
    waiting[4]
        .run(&["access", "status", "--json"])
        .refused("access_request_expired");

    // B32: a damaged key file, and the way back its refusal names: the
    // request replaces the file, the core says the name is still
    // connected, the operator disconnects it, and the request asks anew.
    std::fs::write(kept.key_file("kept", "key"), "{not a key").unwrap();
    kept.run(&["ps"]).refused("access_key_unreadable");
    kept.run(&["access", "request", "--name", "kept", "--json"])
        .refused("access_name_taken");
    assert!(!kept.key_file("kept", "key").exists());
    let mut window = Window::open(&running).await;
    window.send("access_revoke", json!({"id": id})).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let again = loop {
        let ran = kept.run(&["access", "request", "--name", "kept", "--no-wait", "--json"]);
        if ran.ok {
            break ran;
        }
        ran.refused("access_name_taken");
        assert!(Instant::now() < deadline, "still taken: {}", ran.out);
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's disconnect reached the core; the tool's next request reads it
    };
    let renewed = again.json()["value"]["id"].as_str().unwrap().to_owned();
    assert_ne!(renewed, id, "a new key is a new connection");
    window
        .send(
            "access_approve",
            json!({"id": renewed, "mode": {"kind": "all"}}),
        )
        .await;
    assert!(kept.request_until_answered("kept").await.ok);
    assert!(kept.run(&["ps", "--json"]).ok);

    // B8, B19: a caller that names a pane runs as a pane, never as a tool.
    kept.run_with(
        &["access", "request", "--name", "kept", "--json"],
        &[("HIDE_PANE_ID", "w1:p1")],
    )
    .refused("access_from_pane");
    running.stop();
}

/// D-56: a request stays until the operator answers it, approved or
/// refused, and a request whose wait ended becomes the key on the tool's
/// next command, with no second `hide access request`.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_waits_for_its_answer_and_a_later_command_picks_up_the_key() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let mut window = Window::open(&running).await;

    for (name, answer, payload_mode, state) in [
        (
            "approved",
            "access_approve",
            Some(json!({"kind": "all"})),
            Some("connected"),
        ),
        ("refused", "access_refuse", None, None),
    ] {
        let tool = Tool::new(root.path(), &format!("{name}-home"));
        let waiting = tool
            .command(&["access", "request", "--name", name, "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let id = loop {
            let status = tool.run(&["access", "status", "--json"]);
            if status.ok && status.json()["value"]["state"] == "pending" {
                break status.json()["value"]["id"].as_str().unwrap().to_owned();
            }
            assert!(Instant::now() < deadline, "never put up: {}", status.out);
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the waiting request's own process puts it up
        };
        let payload = match payload_mode {
            Some(mode) => json!({"id": id, "mode": mode}),
            None => json!({"id": id}),
        };
        window.send(answer, payload).await;
        let ran = Ran::of(
            tokio::task::spawn_blocking(move || waiting.wait_with_output())
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(
            ran.out
                .contains("Waiting up to 10 minutes for the operator to approve"),
            "{}",
            ran.out
        );
        match state {
            Some(state) => {
                assert!(ran.ok, "{}", ran.out);
                assert_eq!(ran.json()["value"]["state"], state, "{}", ran.out);
                assert!(tool.key_file(name, "key").exists());
            }
            None => ran.refused("access_refused"),
        }
        assert!(!tool.key_file(name, "request").exists());
    }

    let later = Tool::new(root.path(), "later-home");
    let id = later.put_up("later");
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    // `access status` reads where the request stands and keeps no key.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = later.run(&["access", "status", "--json"]);
        if status.ok && status.json()["value"]["state"] == "connected" {
            break;
        }
        assert!(Instant::now() < deadline, "never approved: {}", status.out);
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's approval reached the core
    }
    assert!(later.key_file("later", "request").exists());
    // The first command after the approval already runs as the tool.
    let ps = later.run(&["ps", "--json"]);
    assert!(ps.ok, "{}", ps.out);
    assert!(later.key_file("later", "key").exists());
    assert!(!later.key_file("later", "request").exists());
    running.stop();
}

/// B19: a home with two keys runs as neither until `HIDE_ACCESS` picks one.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_home_with_two_keys_needs_hide_access_to_choose() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let mut window = Window::open(&running).await;
    let tool = Tool::new(root.path(), "shared-home");
    for name in ["first", "second"] {
        let id = tool.put_up(name);
        window
            .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
            .await;
        assert!(tool.request_until_answered(name).await.ok);
    }
    tool.run(&["ps"]).refused("access_ambiguous");
    let chosen = tool.run_with(
        &["access", "status", "--json"],
        &[("HIDE_ACCESS", "second")],
    );
    assert!(chosen.ok, "{}", chosen.out);
    assert_eq!(chosen.json()["value"]["name"], "second");
    running.stop();
}

/// B8 (D-31): an approval from a connection that is not an operator window
/// is refused, and the request still waits; a window's approval connects it.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_an_operator_window_approves_a_tool() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let tool = Tool::new(root.path(), "tool-home");

    let id = tool.put_up("nightly");
    let mut screen = Window::open_as(&running, None).await;
    screen
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert_eq!(
        screen.error().await,
        "External tools are approved in an operator window"
    );
    let status = tool.run(&["access", "status", "--json"]);
    assert_eq!(status.json()["value"]["state"], "pending", "{}", status.out);

    let mut window = Window::open(&running).await;
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert!(tool.request_until_answered("nightly").await.ok);
    running.stop();
}

/// B8 (D-08): a process that descends from a pane's program is never a
/// tool, with no pane in its environment and a valid key in its home: it
/// runs as its folder, as before, and cannot ask as a tool; any other
/// process with the key runs as the tool.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pane_programs_child_is_never_a_tool() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let tool = Tool::new(root.path(), "tool-home");
    let id = tool.put_up("paned");
    let mut window = Window::open(&running).await;
    window
        .send("access_approve", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    assert!(tool.request_until_answered("paned").await.ok);

    let pane = tool.in_a_pane(&["ps", "--json"]);
    let outside = tool.run(&["ps", "--json"]);
    assert!(
        outside.ok,
        "a process outside every pane is the tool: {}",
        outside.out
    );
    let inside = pane.finish();
    inside.refused("checkout_not_registered");
    assert!(!inside.out.contains("access_"), "{}", inside.out);
    tool.in_a_pane(&["access", "request", "--name", "paned", "--json"])
        .finish()
        .refused("access_from_pane");
    running.stop();
}

/// The approved bot `name` in `mode`, its key in its own home.
async fn approved(root: &Path, window: &mut Window, name: &str, mode: Value) -> (Tool, String) {
    let tool = Tool::new(root, &format!("{name}-home"));
    let id = tool.put_up(name);
    window
        .send("access_approve", json!({"id": id, "mode": mode}))
        .await;
    let connected = tool.request_until_answered(name).await;
    assert!(connected.ok, "{}", connected.out);
    assert_eq!(connected.json()["value"]["mode"], mode, "{}", connected.out);
    (tool, id)
}

/// A plain-folder project Hide has open, `project` (`w-project`), with a
/// `notes.md`, under the operator's home by its real path.
fn a_project(env: &Env) -> PathBuf {
    let project = hide_platform::fs::identity::canonical(&env.home)
        .unwrap()
        .join("projects")
        .join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("notes.md"), "# notes\n").unwrap();
    std::fs::write(
        env.state_dir.join("core-state.json"),
        json!({
            "schema_version": 1,
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "workspace_registrations": [{
                "id": "w-project",
                "label": "project",
                "path": hide_platform::path::to_wire(&project).unwrap(),
            }],
        })
        .to_string(),
    )
    .unwrap();
    project
}

/// The refusal an own-mode bot gets for what is not its own work.
fn not_own(ran: Ran, what: &str) {
    assert!(!ran.ok, "{what} ran: {}", ran.out);
    assert!(
        ran.out.contains("access_not_own_work"),
        "{what}: {}",
        ran.out
    );
    assert!(
        ran.out
            .contains(hide_core::access::NOT_OWN_WORK_NEXT_ACTION),
        "{what}'s next action: {}",
        ran.out
    );
}

/// mission-87 c2: a bot of its own work is refused, family by family, what
/// names the operator's work (an agent, a letter's recipient, a pane, a
/// mission) or no one bot's (the Views and files of a project, its records,
/// devices, the bots, the automations), with one code and what to do; it
/// lists its own work only, so with none started its lists are empty; it
/// starts agents in a project Hide has open, past the hold; and the whole
/// Hide, set from the window, runs those again with no new approval. Its
/// own agents and a connected device's pane are the core's tests
/// (`runtime::tests::access_letters`, `missions`, `remote_spawn`), which
/// hold the started records this daemon would need an agent CLI for.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_own_mode_bot_is_refused_the_operators_work_and_lists_only_its_own() {
    let root = tempfile::tempdir().unwrap();
    let env = daemon_env(root.path());
    let project = a_project(&env);
    let running = hided::start_daemon(env).await.unwrap();
    let mut window = Window::open(&running).await;
    let (bot, id) = approved(root.path(), &mut window, "keeper", json!({"kind": "own"})).await;
    let human = bot.run(&["access", "status"]);
    assert!(
        human.out.contains("Connected as keeper · own work"),
        "{}",
        human.out
    );
    let project_wire = hide_platform::path::to_wire(&project).unwrap();

    // Agents and letters: what it names is not its work.
    let agents = bot.run(&["agent", "list", "--json"]);
    assert!(agents.ok, "{}", agents.out);
    assert_eq!(agents.json()["value"]["items"], json!([]), "{}", agents.out);
    for args in [
        &["agent", "show", "agent-1"][..],
        &["agent", "end", "agent-1"],
        &["agent", "sleep", "agent-1"],
        &[
            "request", "send", "someone", "--intent", "look", "--body", "please",
        ],
        &["watch", "start", "agent-1"],
    ] {
        not_own(bot.run(args), &format!("{args:?}"));
    }
    // It starts agents in a project Hide has open: past the hold to the
    // spawn's own answer, since no such agent is installed.
    let spawn = bot.run(&[
        "agent",
        "spawn",
        "--repo",
        &project_wire,
        "--name",
        "helper",
        "--intent",
        "help",
        "--kind",
        "no-such-agent",
    ]);
    assert!(!spawn.out.contains("access_"), "{}", spawn.out);
    // Its own letters and watches are its own.
    for args in [&["inbox"][..], &["watch", "list"]] {
        let ran = bot.run(args);
        assert!(ran.ok, "{args:?}: {}", ran.out);
    }

    // Panes.
    not_own(bot.run(&["pane", "read", "w1:p1"]), "pane read");
    not_own(bot.run(&["pane", "send", "w1:p1", "hi"]), "pane send");
    not_own(bot.run(&["agent", "read", "agent-1"]), "agent read");

    // Missions and `hide ps` list its own work only.
    let missions = bot.run(&["mission", "list"]);
    assert!(missions.ok, "{}", missions.out);
    not_own(
        bot.run(&["mission", "show", "--mission", "mission-1"]),
        "mission show",
    );
    let ps = bot.run(&["ps", "--json"]);
    assert!(ps.ok, "{}", ps.out);
    assert_eq!(ps.json()["value"]["panes"], json!([]), "{}", ps.out);

    // A project's Views, files and records, devices, the bots and the
    // automations are no one bot's work; its own standing still reads.
    not_own(bot.run_in(&project, &["view", "list"]), "view list");
    not_own(
        bot.run(&[
            "file",
            "open",
            &format!("{project_wire}/notes.md"),
            "--repo",
            &project_wire,
        ]),
        "file open",
    );
    not_own(bot.run_in(&project, &["links", "branch", "main"]), "links");
    for args in [
        &["device", "list", "--json"][..],
        &["access", "list", "--json"],
        &["automation", "list"],
    ] {
        not_own(bot.run(args), &format!("{args:?}"));
    }
    assert!(bot.run(&["access", "status", "--json"]).ok);
    bot.run(&["device", "add", "mini"])
        .refused("access_operator_only");

    // The whole Hide, set from the window, reaches the next command.
    window
        .send("access_mode", json!({"id": id, "mode": {"kind": "all"}}))
        .await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while bot.run(&["access", "status", "--json"]).json()["value"]["mode"] != json!({"kind": "all"})
    {
        assert!(Instant::now() < deadline, "the mode never moved");
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's mode change reached the core; the bot's next status reads it
    }
    assert!(bot.run(&["device", "list", "--json"]).ok);
    assert!(bot.run_in(&project, &["view", "list"]).ok);
    running.stop();
}

/// mission-87 c3 (D4): a store an earlier build wrote (schema 1) loads with
/// the connection at Act as a bot of the whole Hide, which runs on with no
/// new approval, and the one at View only ended: its next command and
/// request are refused with the reason that asks it to request again, the
/// request drops its key, and the next one puts it up for the operator.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_earlier_builds_act_tool_runs_as_the_whole_hide_and_its_view_only_tool_asks_again() {
    let root = tempfile::tempdir().unwrap();
    let env = daemon_env(root.path());
    let running = hided::start_daemon(env.clone()).await.unwrap();
    let mut window = Window::open(&running).await;
    let all = json!({"kind": "all"});
    let (acting, _) = approved(root.path(), &mut window, "acting", all.clone()).await;
    let (viewing, _) = approved(root.path(), &mut window, "viewing", all.clone()).await;
    drop(window);
    // Dropped, not only stopped: the drop saves the core's state and lets go
    // of the state folder's lock, as a daemon's exit does.
    drop(running);

    // The file as schema 1 kept the same two: a level where a mode is now.
    let store = hide_kit::layout::access_store(&env.state_dir);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert_eq!(saved["schema_version"], 2);
    saved["schema_version"] = json!(1);
    for connection in saved["connections"].as_array_mut().unwrap() {
        let object = connection.as_object_mut().unwrap();
        object.remove("mode");
        let level = if object["name"] == "acting" {
            "act"
        } else {
            "view"
        };
        object.insert("level".into(), json!(level));
    }
    std::fs::write(&store, serde_json::to_vec(&saved).unwrap()).unwrap();

    let running = hided::start_daemon(env).await.unwrap();
    let status = acting.run(&["access", "status", "--json"]);
    assert!(status.ok, "{}", status.out);
    assert_eq!(status.json()["value"]["mode"], all, "{}", status.out);
    assert!(acting.run(&["ps", "--json"]).ok);

    viewing.run(&["ps", "--json"]).refused("access_reapprove");
    // As for a refused tool (B7): the next request says so and drops the
    // key, and the one after asks the operator anew.
    viewing
        .run(&[
            "access",
            "request",
            "--name",
            "viewing",
            "--no-wait",
            "--json",
        ])
        .refused("access_reapprove");
    assert!(!viewing.key_file("viewing", "key").exists());
    viewing.put_up("viewing");
    running.stop();
}
