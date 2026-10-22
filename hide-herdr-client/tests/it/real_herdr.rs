//! The client against the pinned Herdr itself, on the operating system the
//! test runs on: a request, a snapshot and a live event subscription over the
//! platform's local stream (a Unix socket, or a named pipe on Windows).
//!
//! It needs the pinned binary, so it is ignored by default; the `os contract`
//! lane runs it with `--ignored` on every system and names the binary in
//! `HIDE_E2E_HERDR_BIN`. The server it starts is private: its own socket,
//! session, config and home, none of the operator's.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use hide_herdr_client::{request_with_timeout, subscribe};
use serde_json::{Value, json};

fn pinned_version() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/herdr-bundle.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest).unwrap()).unwrap();
    manifest["version"].as_str().unwrap().to_owned()
}

fn herdr_command(bin: &Path, home: &Path, socket: &Path, config: &Path) -> Command {
    let mut command = Command::new(bin);
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().to_uppercase();
        if key.starts_with("HERDR_") || key.starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .env("HERDR_SESSION", "hide-contract")
        .env("HERDR_SOCKET_PATH", socket)
        .env("HERDR_CONFIG_PATH", config)
        .env("HERDR_DISABLE_SOUND", "1");
    command
}

struct PrivateHerdr {
    bin: PathBuf,
    home: PathBuf,
    socket: PathBuf,
    config: PathBuf,
    server: Child,
    // Dropped last: it holds every path above.
    _root: tempfile::TempDir,
}

impl PrivateHerdr {
    fn start(bin: PathBuf) -> Self {
        let version = String::from_utf8(
            Command::new(&bin)
                .arg("--version")
                .output()
                .expect("herdr --version")
                .stdout,
        )
        .unwrap();
        assert_eq!(
            version.split_whitespace().last().unwrap(),
            pinned_version(),
            "{} is not the pinned Herdr",
            bin.display()
        );
        // A Unix socket path is limited to about a hundred bytes.
        let root = if cfg!(unix) {
            tempfile::Builder::new().prefix("hr").tempdir_in("/tmp")
        } else {
            tempfile::Builder::new().prefix("hr").tempdir()
        }
        .unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let config = root.path().join("herdr-config.toml");
        std::fs::write(
            &config,
            "[update]\nversion_check = false\nmanifest_check = false\n",
        )
        .unwrap();
        let socket = root.path().join("herdr.sock");
        let log = std::fs::File::create(root.path().join("server.log")).unwrap();
        let server = herdr_command(&bin, &home, &socket, &config)
            .arg("server")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("herdr server starts");
        let herdr = Self {
            bin,
            home,
            socket,
            config,
            server,
            _root: root,
        };
        wait_for_ping(&herdr.socket);
        herdr
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        herdr_command(&self.bin, &self.home, &self.socket, &self.config)
            .args(args)
            .output()
            .expect("herdr cli runs")
    }

    /// The CLI's JSON answer to `args`, which must succeed.
    fn cli_json(&self, args: &[&str]) -> Value {
        let output = self.cli(args);
        assert!(
            output.status.success(),
            "`herdr {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("the CLI answers JSON")
    }
}

/// Asks a starting server `ping` until it answers; the minute only ends a
/// server that never comes up.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for_ping(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while request_with_timeout(socket, "ping", json!({}), Duration::from_secs(2)).is_err() {
        assert!(Instant::now() < deadline, "the server never answered ping");
        thread::sleep(Duration::from_millis(250));
    }
}

impl Drop for PrivateHerdr {
    fn drop(&mut self) {
        let _ = self.cli(&["server", "stop"]);
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn the_pinned_herdr_answers_a_request_and_streams_events_over_the_local_stream() {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    let herdr = PrivateHerdr::start(bin);

    let pong = request_with_timeout(&herdr.socket, "ping", json!({}), Duration::from_secs(5))
        .expect("ping");
    assert_eq!(pong["type"], "pong");

    let snapshot = request_with_timeout(
        &herdr.socket,
        "session.snapshot",
        json!({}),
        Duration::from_secs(10),
    )
    .expect("session.snapshot");
    assert_eq!(snapshot["type"], "session_snapshot");

    let subscription = subscribe(
        &herdr.socket,
        json!({"subscriptions": [{"type": "workspace.created"}]}),
        Duration::from_secs(5),
    )
    .expect("subscribe");
    assert_eq!(subscription.ack.kind, "subscription_started");
    let shutdown_after = {
        let (mut reader, shutdown) = subscription.into_parts();
        let (lines, heard) = mpsc::channel();
        thread::spawn(move || {
            use std::io::BufRead;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => {
                        let _ = lines.send(None);
                        return;
                    }
                    Ok(_) => {
                        let _ = lines.send(Some(line));
                    }
                }
            }
        });

        let created = herdr.cli(&[
            "workspace",
            "create",
            "--cwd",
            herdr.home.to_str().unwrap(),
            "--label",
            "contract",
        ]);
        assert!(
            created.status.success(),
            "workspace create failed: {}",
            String::from_utf8_lossy(&created.stderr)
        );
        let line = heard
            .recv_timeout(Duration::from_secs(30))
            .expect("an event arrived over the subscription")
            .expect("the subscription ended before an event");
        let event: Value = serde_json::from_str(&line).expect("event JSON");
        assert!(
            event["event"]
                .as_str()
                .is_some_and(|name| name.contains("workspace")),
            "unexpected event: {line}"
        );

        // The reader is blocked on the stream now; the handle must free it.
        let started = Instant::now();
        shutdown.shutdown();
        loop {
            match heard.recv_timeout(Duration::from_secs(5)) {
                Ok(None) => break,
                Ok(Some(_)) => continue,
                Err(_) => panic!("shutdown did not end the blocked read"),
            }
        }
        started.elapsed()
    };
    assert!(
        shutdown_after < Duration::from_secs(5),
        "{shutdown_after:?}"
    );
}

/// PRD herdr-takeover B23, D-25: where the pinned Herdr cannot hand its
/// panes to another Herdr, Hide moves the operator's older server by stopping
/// and restarting it after they confirm what ends. Herdr answers this on Unix
/// only today; the day a bump makes Windows answer `true`, this fails so the
/// restart path and its cell in docs/INSTALL.md, What each system supports,
/// change with it.
#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn the_pinned_herdr_hands_a_server_over_live_on_unix_only() {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    let herdr = PrivateHerdr::start(bin);
    let pong = request_with_timeout(&herdr.socket, "ping", json!({}), Duration::from_secs(5))
        .expect("ping");
    assert_eq!(pong["type"], "pong");
    assert_eq!(
        pong["capabilities"]["live_handoff"]
            .as_bool()
            .unwrap_or(false),
        cfg!(unix),
        "the pinned Herdr changed whether it hands a server over live here: update the \
         restart path (desktop/src/main/host.ts, restartHerdr) and the Windows cell of \
         docs/INSTALL.md, What each system supports"
    );
}

/// The socket `hide_platform::host` names as Herdr's default is the one the
/// pinned Herdr itself uses when nothing overrides it. The child inherits
/// this process's home and config variables and none of Herdr's own, and is
/// only asked for its status, which starts and creates nothing. Run by hand,
/// point HOME (USERPROFILE on Windows) at a short empty folder so the answer
/// never names the operator's socket.
#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn the_default_socket_is_the_one_the_pinned_herdr_uses() {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    let mut command = Command::new(&bin);
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().to_uppercase();
        if key.starts_with("HERDR_") || key.starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(["status", "server", "--json"])
        .stdin(Stdio::null())
        .output()
        .expect("herdr status server");
    let status: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "herdr status server --json is not JSON ({error}): {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let socket = status["socket"]
        .as_str()
        .expect("the status names a socket");
    assert_eq!(
        PathBuf::from(socket),
        hide_platform::host::herdr_socket_default().unwrap()
    );
}

/// The session the pinned Herdr names for a pane's agent. Hide's subagent
/// counts take an event of any other session for a nested one and ignore it
/// (`docs/agent-hooks.md`, Which session is the pane's), and what decides
/// which session the pane has is Herdr's, in `src/terminal/state.rs` of the
/// pinned tag (`session_report_allows_session_replacement` and
/// `conflicting_same_owner_session_ref`): for `herdr:claude`, a report of
/// another session replaces the pane's only when its `session_start_source` is
/// `clear`, `resume` or `compact`, and a `startup` of another session, which
/// is all a nested `claude -p` reports, is dropped. Herdr does not document
/// this, so a bump that changes it has to fail here and not in the counts.
#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn the_pinned_herdr_keeps_a_panes_session_against_a_nested_startup_and_follows_clear_resume_and_compact()
 {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    let herdr = PrivateHerdr::start(bin);

    // Herdr takes a session report only for an agent it detects in the pane,
    // by the process it started: a program named `claude`.
    let agent_dir = herdr.home.join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let source = agent_dir.join("claude.c");
    std::fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { sleep(600); return 0; }\n",
    )
    .unwrap();
    let agent = agent_dir.join("claude");
    let built = Command::new("cc")
        .arg(&source)
        .arg("-o")
        .arg(&agent)
        .status()
        .expect("cc builds the stand-in agent");
    assert!(built.success());

    let created = herdr.cli(&[
        "workspace",
        "create",
        "--cwd",
        herdr.home.to_str().unwrap(),
        "--label",
        "session",
    ]);
    assert!(created.status.success());
    let created: Value = serde_json::from_slice(&created.stdout).unwrap();
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("the workspace's pane")
        .to_owned();
    let ran = herdr.cli(&["pane", "run", &pane, agent.to_str().unwrap()]);
    assert!(ran.status.success());

    let ask = |method: &str, params: Value| {
        request_with_timeout(&herdr.socket, method, params, Duration::from_secs(5)).unwrap()
    };
    let session = || {
        ask("pane.get", json!({"pane_id": pane}))["pane"]["agent_session"]["value"]
            .as_str()
            .map(str::to_owned)
    };
    let mut seq = 0_u64;
    let mut report = |session: &str, start: &str| {
        seq += 1;
        ask(
            "pane.report_agent_session",
            json!({"pane_id": pane, "source": "herdr:claude", "agent": "claude",
                   "seq": seq, "agent_session_id": session, "session_start_source": start}),
        );
    };

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        report("pane-session", "startup");
        if session().as_deref() == Some("pane-session") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Herdr never took the pane's session"
        );
        wait_a_moment();
    }

    // A nested `claude -p` reports a startup of its own session: dropped.
    report("nested-session", "startup");
    assert_eq!(session().as_deref(), Some("pane-session"));

    // A real session change replaces it, and a nested startup after that is
    // dropped again.
    for (start, next) in [
        ("clear", "after-clear"),
        ("resume", "after-resume"),
        ("compact", "after-compact"),
    ] {
        report(next, start);
        assert_eq!(session().as_deref(), Some(next), "{start}");
        report("nested-session", "startup");
        assert_eq!(
            session().as_deref(),
            Some(next),
            "{start}, then a nested startup"
        );
    }
}

#[cfg(unix)]
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state
fn wait_a_moment() {
    thread::sleep(Duration::from_millis(250));
}

/// A `terminal session control` stream the test drives: its stdin takes the
/// control lines, and a thread hands every `terminal.frame` it writes over a
/// channel as the frame's size and decoded bytes.
struct Controller {
    child: Child,
    stdin: std::process::ChildStdin,
    frames: mpsc::Receiver<(u16, u16, Vec<u8>)>,
}

impl Controller {
    fn attach(herdr: &PrivateHerdr, pane: &str, cols: u16, rows: u16) -> Self {
        let mut child = herdr_command(&herdr.bin, &herdr.home, &herdr.socket, &herdr.config)
            .args([
                "terminal",
                "session",
                "control",
                pane,
                "--cols",
                &cols.to_string(),
                "--rows",
                &rows.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the control session starts");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, frames) = mpsc::channel();
        thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(record) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if record["type"] != "terminal.frame" {
                    continue;
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(record["bytes"].as_str().unwrap_or_default())
                    .unwrap_or_default();
                let width = record["width"].as_u64().unwrap_or_default() as u16;
                let height = record["height"].as_u64().unwrap_or_default() as u16;
                if sender.send((width, height, bytes)).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            frames,
        }
    }

    fn send(&mut self, line: Value) {
        use std::io::Write;
        writeln!(self.stdin, "{line}").expect("the control stream takes a line");
        self.stdin.flush().unwrap();
    }

    fn type_line(&mut self, text: &str) {
        let bytes = base64::engine::general_purpose::STANDARD.encode(format!("{text}\r"));
        self.send(json!({"type": "terminal.input", "bytes": bytes}));
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.send(json!({
            "type": "terminal.resize", "cols": cols, "rows": rows,
            "cell_width_px": 0, "cell_height_px": 0,
        }));
    }

    /// Reads frames until one's text shows `marker` (or, with none, until the
    /// first frame) and says that frame's size. Frames before it may still
    /// carry the size before a resize, so only the marker's frame is judged.
    /// The deadline only ends a stream that never answers.
    fn frames_until(&self, marker: Option<&str>, deadline: Instant) -> (u16, u16) {
        let mut screen = String::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let (width, height, bytes) = self.frames.recv_timeout(left).unwrap_or_else(|_| {
                panic!("no frame showed {marker:?}; the screen read: {screen:?}")
            });
            screen.push_str(&String::from_utf8_lossy(&bytes));
            match marker {
                Some(marker) if !plain(&screen).contains(marker) => continue,
                _ => return (width, height),
            }
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The screen's text without its escape sequences (CSI, OSC and two-byte
/// ESC sequences), so a marker the shell printed can be found in it.
fn plain(screen: &str) -> String {
    let mut text = String::new();
    let mut chars = screen.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '@' || c == '~' {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    text
}

/// The PTY size is the controller's (PRD grid-ahead-of-herdr D-02, B15): a
/// pane runs at the grid its controller attached with and last sent as
/// `terminal.resize`, and neither a zoom nor its sibling's close moves it,
/// so Hide may size a pane's grid from the rectangle it draws before Herdr
/// confirms the layout. The frames the controller receives carry the size,
/// and on Unix the shell's own `stty size` is read as well. A pinned Herdr
/// that sizes the PTY from its layout fails here, and the bump that brings
/// it decides what the drawn-ahead grid does on it (`docs/ARCHITECTURE.md`,
/// Drawn ahead of Herdr).
#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn the_pinned_herdr_keeps_the_controllers_pty_size_through_a_zoom_and_a_siblings_close() {
    let bin = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    let herdr = PrivateHerdr::start(bin);
    let created = herdr.cli_json(&[
        "workspace",
        "create",
        "--cwd",
        herdr.home.to_str().unwrap(),
        "--label",
        "grid",
    ]);
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("the workspace's pane")
        .to_owned();
    let split = herdr.cli_json(&["pane", "split", &pane, "--direction", "right"]);
    let sibling = split["result"]["pane"]["pane_id"]
        .as_str()
        .expect("the split's pane")
        .to_owned();

    let mut controller = Controller::attach(&herdr, &pane, 45, 20);
    let deadline = || Instant::now() + Duration::from_secs(60);
    // The attach draws the pane at the controller's size, and the shell's
    // prompt in it means the shell reads keys.
    assert_eq!(
        controller.frames_until(None, deadline()),
        (45, 20),
        "the attach frame is the controller's size"
    );
    // After each step the shell prints a marker, on Unix with the PTY's own
    // size as `rows x cols`, and the frame that shows it is judged.
    let mut step = 0;
    let mut pty_size = |controller: &mut Controller, cols: u16, rows: u16, after: &str| {
        step += 1;
        let marker = if cfg!(unix) {
            controller.type_line(&format!("echo SZ{step}=$(stty size | tr ' ' x)"));
            format!("SZ{step}={rows}x{cols}")
        } else {
            controller.type_line(&format!("echo SZ{step}"));
            format!("SZ{step}")
        };
        assert_eq!(
            controller.frames_until(Some(&marker), deadline()),
            (cols, rows),
            "the frame after {after}"
        );
    };
    pty_size(&mut controller, 45, 20, "the attach");

    controller.resize(90, 20);
    pty_size(
        &mut controller,
        90,
        20,
        "the controller's resize to 90 columns",
    );

    let zoomed = herdr.cli_json(&["pane", "zoom", &pane, "--on"]);
    assert_eq!(zoomed["result"]["zoom"]["changed"], true, "{zoomed}");
    pty_size(&mut controller, 90, 20, "the zoom");
    let unzoomed = herdr.cli_json(&["pane", "zoom", &pane, "--off"]);
    assert_eq!(unzoomed["result"]["zoom"]["changed"], true, "{unzoomed}");
    pty_size(&mut controller, 90, 20, "the unzoom");

    herdr.cli_json(&["pane", "close", &sibling]);
    pty_size(&mut controller, 90, 20, "the sibling's close");

    // And it still follows the controller afterwards.
    controller.resize(60, 24);
    pty_size(
        &mut controller,
        60,
        24,
        "the controller's resize to 60 by 24",
    );
    controller.send(json!({"type": "terminal.release"}));
}
