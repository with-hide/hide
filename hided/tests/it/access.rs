//! An external tool's journey (PRD hide-external-control c1, c2): a fake
//! tool, the built `hide` run with a home of its own and no pane identity,
//! asks to connect, the operator's window approves it with the event
//! Settings sends, and the same request run again hands the tool its key.
//! It then runs what its level allows and is refused the rest with a code
//! and what to do; a refusal, a disconnect, its own leave, a restart and a
//! damaged key each answer as the tool can act on.

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
        search_path: None,
    }
}

/// The operator's window: a web client, the one kind of connection that may
/// approve, refuse, change the level of and disconnect a tool.
struct Window(Socket);

impl Window {
    async fn open(running: &hided::RunningDaemon) -> Self {
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
struct Tool {
    home: PathBuf,
    state: PathBuf,
}

/// What one `hide` run answered.
struct Ran {
    ok: bool,
    out: String,
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
    fn json(&self) -> Value {
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
    fn new(root: &Path, name: &str) -> Self {
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

    fn run(&self, args: &[&str]) -> Ran {
        self.run_with(args, &[])
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

    /// `hide access request --name <name> --json` until the answer is not
    /// pending, as a tool polls after the operator was asked.
    async fn request_until_answered(&self, name: &str) -> Ran {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let ran = self.run(&["access", "request", "--name", name, "--json"]);
            if !(ran.ok && ran.json()["value"]["state"] == "pending") {
                return ran;
            }
            assert!(Instant::now() < deadline, "still pending: {}", ran.out);
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's approval reached the core; the tool's next request reads it
        }
    }

    /// Requests and returns the pending request's id (B1).
    fn put_up(&self, name: &str) -> String {
        let ran = self.run(&["access", "request", "--name", name, "--json"]);
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

/// c1: request, the window's approval, the key on the request run again; at
/// View only the reads of B20 and nothing else; at Act the commands of B21,
/// letters and inbox among them; and B22, B23 refused with their codes.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_asks_is_approved_and_runs_what_its_level_allows() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let mut window = Window::open(&running).await;
    let tool = Tool::new(root.path(), "tool-home");

    let id = tool.put_up("nightly");
    // Asked again, the same request, not a second one (B5, B9).
    assert_eq!(tool.put_up("nightly"), id);
    window
        .send("access_approve", json!({"id": id, "level": "view"}))
        .await;
    let connected = tool.request_until_answered("nightly").await;
    assert!(connected.ok, "{}", connected.out);
    assert_eq!(connected.json()["value"]["state"], "connected");
    assert_eq!(connected.json()["value"]["level"], "view");
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
        human.out.contains("Connected as nightly · View only"),
        "{}",
        human.out
    );

    // View only (B20).
    for args in [
        &["access", "status", "--json"][..],
        &["access", "list", "--json"],
        &["ps", "--json"],
        &["agent", "list"],
        &["mission", "list"],
        &["device", "list", "--json"],
    ] {
        let ran = tool.run(args);
        assert!(ran.ok, "{args:?}: {}", ran.out);
    }
    let list = tool.run(&["access", "list", "--json"]).json();
    assert_eq!(list["value"]["connections"][0]["name"], "nightly");
    assert_eq!(list["value"]["connections"][0]["level"], "view");
    for args in [
        &["inbox"][..],
        &[
            "request", "send", "someone", "--intent", "look", "--body", "please",
        ],
        &["agent", "sleep", "agent-1"],
        &["pane", "send", "w1:p1", "hi"],
    ] {
        tool.run(args).refused("access_view_only");
    }

    // Act (B21): the operator's switch reaches the tool's next command.
    window
        .send("access_level", json!({"id": id, "level": "act"}))
        .await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while tool.run(&["access", "status", "--json"]).json()["value"]["level"] != "act" {
        assert!(Instant::now() < deadline, "the level never moved");
        tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the window's level change reached the core; the tool's next status reads it
    }
    let inbox = tool.run(&["inbox"]);
    assert!(inbox.ok, "{}", inbox.out);
    // A letter goes as far as the recipient: there is none, and the core
    // says so, not the level.
    let send = tool.run(&[
        "request", "send", "someone", "--intent", "look", "--body", "please",
    ]);
    assert!(
        !send.ok && send.out.contains("target_unavailable"),
        "{}",
        send.out
    );
    assert!(!send.out.contains("access_"), "{}", send.out);

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
        .send("access_approve", json!({"id": id, "level": "act"}))
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
        .send("access_approve", json!({"id": id, "level": "view"}))
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
        .send("access_approve", json!({"id": id, "level": "view"}))
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
        let ran = kept.run(&["access", "request", "--name", "kept", "--json"]);
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
        .send("access_approve", json!({"id": renewed, "level": "view"}))
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
            .send("access_approve", json!({"id": id, "level": "view"}))
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

/// B8 (D-08): who is never a tool, and who answers one. A process that
/// descends from a pane's program is never a tool, with no pane in its
/// environment and a valid key in its home: it runs as its folder, as
/// before; any other process with the key runs as the tool. An approval
/// from a connection that is not an operator window is refused, and the
/// request still waits.
// The daemon runs on this test's runtime while a fake tool's `hide` blocks
// a thread of it, so it needs threads of its own.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pane_program_is_never_a_tool_and_only_a_window_approves_one() {
    let root = tempfile::tempdir().unwrap();
    let running = hided::start_daemon(daemon_env(root.path())).await.unwrap();
    let tool = Tool::new(root.path(), "tool-home");

    let id = tool.put_up("paned");
    let mut screen = Window::open_as(&running, None).await;
    screen
        .send("access_approve", json!({"id": id, "level": "act"}))
        .await;
    assert_eq!(
        screen.error().await,
        "External tools are approved in an operator window"
    );
    let status = tool.run(&["access", "status", "--json"]);
    assert_eq!(status.json()["value"]["state"], "pending", "{}", status.out);

    let mut window = Window::open(&running).await;
    window
        .send("access_approve", json!({"id": id, "level": "act"}))
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
