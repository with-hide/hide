//! A guest's node at its launch contract (PRD device-connect-iroh D-77):
//! `hided node serve --guest <dir>` as a host of another build starts it,
//! over the folder, the two endpoints, the lines on its standard streams
//! and its exit codes, which are all the host build knows of it
//! (`hide_peer::protocol::launch`).
//!
//! Every node here is started as the host starts one
//! (`hided::guests::node::start`). What a pane does under a guest's node
//! needs the built pane holder: the two-core journey's on Unix, and on
//! Windows the one ignored test below, which the `windows check` lane runs
//! with the holder built.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hide_node_link::protocol::{Call, Hello};
use hide_peer::protocol::launch::{self, FromNode, Ready};
use hide_peer::protocol::tap::{self, ToHost, ToNode};
use hide_platform::ipc::LocalStream;
use hide_platform::process::OwnedChild;

/// A hang guard, never a pause.
const WITHIN: Duration = Duration::from_secs(30);

/// A folder short enough that the node's endpoints fit a Unix socket
/// address below it (docs/TESTING.md, Writing a Rust test, rule 7).
fn short_root() -> tempfile::TempDir {
    #[cfg(unix)]
    return tempfile::Builder::new()
        .prefix("hg")
        .tempdir_in("/tmp")
        .unwrap();
    #[cfg(not(unix))]
    tempfile::tempdir().unwrap()
}

fn hided(home: &Path) -> Command {
    hided_at(Path::new(env!("CARGO_BIN_EXE_hided")), home)
}

fn hided_at(program: &Path, home: &Path) -> Command {
    // A new build's first start is slow on macOS and Windows; it starts
    // once before a test waits on it (rule 8).
    crate::stand_ins::ready_with_args(program, &["node", "--version"]);
    let mut command = Command::new(program);
    for (key, _) in std::env::vars_os() {
        let upper = key.to_string_lossy().to_uppercase();
        if hide_platform::process::is_foreign_pane_identity(&upper) || upper.starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData").join("Roaming"))
        .env("LOCALAPPDATA", home.join("AppData").join("Local"))
        .env_remove("XDG_STATE_HOME")
        .stderr(Stdio::inherit());
    command
}

/// One guest's node, ended with the test.
struct Node {
    child: OwnedChild,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    dir: PathBuf,
}

impl Node {
    /// Starts a node over `<root>/n`; its ready line is not read yet.
    fn start(root: &Path) -> Self {
        Self::start_from(root, Path::new(env!("CARGO_BIN_EXE_hided")))
    }

    /// [`Node::start`] of the `hided` at `program`, as the host starts a
    /// guest's node.
    fn start_from(root: &Path, program: &Path) -> Self {
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let dir = root.join(launch::NODE_SIDE);
        let mut child = hided::guests::node::start(
            hided_at(program, &home)
                .args(launch::ARGUMENTS)
                .arg(&dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped()),
        )
        .expect("the node starts");
        let input = child.take_stdin();
        let output = child.take_stdout().unwrap();
        let (told, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if told.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            lines,
            dir,
        }
    }

    fn line(&self, what: &str) -> String {
        self.lines
            .recv_timeout(WITHIN)
            .unwrap_or_else(|_| panic!("the node did not write {what}"))
    }

    fn ready(&self) -> Ready {
        let line = self.line("its ready line");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("{line}: {error}"))
    }

    fn idle(&self, what: &str) -> bool {
        let line = self.line(what);
        match FromNode::parse(&line) {
            Some(FromNode::Idle(idle)) => idle,
            other => panic!("{what}: {line} read as {other:?}"),
        }
    }

    fn tell(&mut self, line: &str) {
        let input = self.input.as_mut().expect("the node's input is open");
        writeln!(input, "{line}").unwrap();
        input.flush().unwrap();
    }

    /// The code the node exits with, waited for on a thread of its own so
    /// a node that never ends fails the test instead of holding it.
    fn exit(mut self) -> Option<i32> {
        let (ended, code) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = ended.send(self.child.wait().ok().and_then(|status| status.code()));
        });
        code.recv_timeout(WITHIN).expect("the node did not exit")
    }

    fn tap(&self) -> LocalStream {
        let mut stream = LocalStream::connect(&self.dir.join(launch::TAP_ENDPOINT)).unwrap();
        stream.set_read_timeout(Some(WITHIN)).unwrap();
        stream
            .write_all(
                &ToNode::Hello {
                    oldest: tap::OLDEST_TAP,
                    newest: tap::NEWEST_TAP,
                }
                .encode()
                .unwrap(),
            )
            .unwrap();
        stream
    }
}

/// The next frame a node wrote on its tap; `None` once the stream ended.
fn tap_frame(stream: &mut LocalStream) -> Option<ToHost> {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).ok()?;
    let mut body = vec![0u8; tap::frame_length(prefix).unwrap()];
    stream.read_exact(&mut body).ok()?;
    ToHost::decode(body[0], &body[1..]).unwrap()
}

#[test]
fn a_word_after_serve_the_node_does_not_know_exits_64_before_it_touches_a_file() {
    let root = short_root();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = root.path().join("n");
    let status = OwnedChild::spawn(
        hided(&home)
            .args(["node", "serve", "--guest-of-a-later-host"])
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null()),
    )
    .unwrap()
    .wait()
    .unwrap();
    assert_eq!(status.code(), Some(launch::EXIT_UNKNOWN_WORD));
    assert!(!dir.exists());
}

#[test]
fn a_node_binds_its_endpoints_says_ready_then_idle_and_ends_with_its_input() {
    let root = short_root();
    let mut node = Node::start(root.path());
    let ready = node.ready();
    assert_eq!(ready.guest_node, launch::GUEST_NODE);
    assert_eq!(ready.pid, node.child.id());
    assert_eq!(
        (ready.link.as_str(), ready.tap.as_str()),
        (launch::LINK_ENDPOINT, launch::TAP_ENDPOINT)
    );
    assert_eq!(ready.tap_versions, [tap::OLDEST_TAP, tap::NEWEST_TAP]);
    assert!(!ready.build.is_empty() && !ready.build_id.is_empty());
    for endpoint in [&ready.link, &ready.tap] {
        assert!(
            hide_platform::ipc::is_endpoint(&node.dir.join(endpoint)).unwrap(),
            "{endpoint} is bound in the node's folder"
        );
    }
    assert!(node.idle("that it is idle"), "no pane and no guest: idle");

    // A second node of the folder is refused while the first holds it, and
    // the first is left as it was.
    let second = Node::start(root.path());
    assert_eq!(second.exit(), Some(launch::EXIT_FOLDER_HELD));
    let mut host = node.tap();
    assert_eq!(
        tap_frame(&mut host),
        Some(ToHost::Welcome {
            version: tap::NEWEST_TAP,
            panes: Vec::new()
        })
    );

    // The host is gone when the node's input ends: the node exits.
    drop(node.input.take());
    assert_eq!(node.exit(), Some(launch::EXIT_ENDED));
}

/// One exchange with the node's away endpoint, as a pane's `hide` has it.
fn away(node: &Node, ask: serde_json::Value) -> serde_json::Value {
    let mut stream = LocalStream::connect(&node.dir.join("away.sock")).unwrap();
    stream.set_read_timeout(Some(WITHIN)).unwrap();
    writeln!(stream, "{ask}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|error| panic!("{line}: {error}"))
}

#[test]
fn a_node_sends_the_box_an_earlier_node_left_and_answers_a_pane_while_its_guest_is_away() {
    let root = short_root();
    // An earlier node of this guest, of another build, kept two events;
    // the guest's core recorded the first.
    let recorded = include_str!("../../../contracts/fixtures/peer-link/event-box.jsonl");
    let dir = root.path().join(launch::NODE_SIDE);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("box.jsonl"), recorded).unwrap();
    std::fs::write(dir.join("acked"), r#"{"v":1,"outbox":"recorded","seq":1}"#).unwrap();

    let mut node = Node::start(root.path());
    assert_eq!(node.ready().capabilities, launch::NODE_CAPABILITIES);
    assert!(node.idle("that it is idle"));

    // Nothing is sent until the guest's core names its place; then the
    // event it has not recorded comes as it was written, with the fields
    // this build does not know.
    node.tell(r#"{"resume":{"outbox":"recorded","seq":1}}"#);
    let kept: serde_json::Value = serde_json::from_str(recorded.lines().nth(1).unwrap()).unwrap();
    let sent: serde_json::Value = serde_json::from_str(&node.line("its kept event")).unwrap();
    assert_eq!(sent, serde_json::json!({ "event": kept }));
    node.tell(r#"{"ack":{"outbox":"recorded","seq":2}}"#);
    node.tell(r#""away""#);

    // With no link, a pane's `hide` asks the node itself: a command that
    // needs the guest's core is told the core is away, and a letter is
    // kept only for a caller this machine's kernel finds in the pane it
    // names, which this test is not.
    assert_eq!(
        away(&node, serde_json::json!({"pane_id": "w1:p1"})),
        serde_json::json!({"ok": false, "reason": "owner_offline"})
    );
    let letter = away(
        &node,
        serde_json::json!({"pane_id": "w1:p1", "command": {
            "op": "send", "target": "parent", "intent": "a", "body": "b", "kind": "report"
        }}),
    );
    assert_eq!(letter["ok"], false, "{letter}");
    assert_ne!(letter["reason"], "owner_offline", "{letter}");

    drop(node.input.take());
    assert_eq!(node.exit(), Some(launch::EXIT_ENDED));
    // What the guest recorded is gone from the box, and its place is kept.
    assert_eq!(std::fs::read(dir.join("box.jsonl")).unwrap(), b"");
    let place: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("acked")).unwrap()).unwrap();
    assert_eq!(
        place,
        serde_json::json!({"v": 1, "outbox": "recorded", "seq": 2})
    );
}

#[test]
fn a_new_host_on_the_tap_replaces_the_one_before_it() {
    let root = short_root();
    let node = Node::start(root.path());
    node.ready();
    let mut first = node.tap();
    assert!(matches!(
        tap_frame(&mut first),
        Some(ToHost::Welcome { .. })
    ));
    let mut second = node.tap();
    assert!(matches!(
        tap_frame(&mut second),
        Some(ToHost::Welcome { .. })
    ));
    assert_eq!(
        tap_frame(&mut first),
        None,
        "the earlier host's stream ends"
    );
}

#[test]
fn a_guests_link_is_served_again_after_it_ends_and_a_new_one_ends_the_one_before() {
    let root = short_root();
    let node = Node::start(root.path());
    node.ready();
    assert!(node.idle("that it is idle"));
    let endpoint = node.dir.join(launch::LINK_ENDPOINT);
    let greet = |name: &str| {
        let stream = LocalStream::connect(&endpoint).unwrap();
        let link =
            hide_node::ssh::host::over_local_stream(name, stream, None, Box::new(|_| {})).unwrap();
        let hello: Hello = hide_node_link::call_as(&link, Call::Hello, WITHIN).unwrap();
        (link, hello)
    };

    let (first, hello) = greet("guest:first");
    assert_eq!(hello.protocol, hide_node_link::protocol::PROTOCOL_VERSION);
    assert!(!node.idle("that a guest is linked"));
    drop(first);
    assert!(node.idle("that the guest left"));

    // A connection that never ends by itself, as one the guest's machine
    // dropped without a word: the next one is served all the same.
    let mut stuck = LocalStream::connect(&endpoint).unwrap();
    stuck.set_read_timeout(Some(WITHIN)).unwrap();
    assert!(!node.idle("that the stuck connection is linked"));
    let (_second, hello) = greet("guest:second");
    assert_eq!(hello.protocol, hide_node_link::protocol::PROTOCOL_VERSION);
    let mut byte = [0u8; 1];
    assert!(
        matches!(stuck.read(&mut byte), Ok(0) | Err(_)),
        "the earlier connection is ended"
    );
}

#[test]
fn a_node_told_to_end_exits_as_ended() {
    for end in [r#"{"end":"now"}"#, r#"{"end":"after_last_pane"}"#] {
        let root = short_root();
        let mut node = Node::start(root.path());
        node.ready();
        // A line of a later host is skipped, not an end.
        node.tell(r#"{"later":true}"#);
        node.tell(end);
        assert_eq!(node.exit(), Some(launch::EXIT_ENDED), "{end}");
    }
}

#[cfg(unix)]
#[test]
fn a_folder_whose_endpoints_would_not_fit_a_socket_address_is_refused() {
    let root = short_root();
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = root.path().join("x".repeat(80));
    let status = OwnedChild::spawn(
        hided(&home)
            .args(launch::ARGUMENTS)
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null()),
    )
    .unwrap()
    .wait()
    .unwrap();
    assert_eq!(status.code(), Some(launch::EXIT_FOLDER_REFUSED));
    assert!(!dir.join(launch::LINK_ENDPOINT).exists());
}

/// The node's folder of programs as a guest's build lays it out on a Windows
/// host: this build's `hided.exe` and `hide.exe`, the built pane holder and
/// its pinned ConPTY runtime in `holder-conpty`, from which the node takes
/// its holders' version folder.
#[cfg(windows)]
fn windows_build(folder: &Path) -> PathBuf {
    let variable =
        |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    std::fs::create_dir_all(folder.join(hide_kit::holders::CONPTY_SOURCE)).unwrap();
    let built = Path::new(env!("CARGO_BIN_EXE_hided")).parent().unwrap();
    for program in ["hided.exe", "hide.exe"] {
        std::fs::copy(built.join(program), folder.join(program)).unwrap();
    }
    std::fs::copy(
        variable("HIDE_TEST_HOLDER_BIN"),
        folder.join(hide_kit::holders::program_name()),
    )
    .unwrap();
    let conpty = variable("HIDE_HOLDER_CONPTY");
    let mut folders = vec![PathBuf::new()];
    while let Some(relative) = folders.pop() {
        for entry in std::fs::read_dir(conpty.join(&relative)).unwrap() {
            let entry = entry.unwrap();
            let relative = relative.join(entry.file_name());
            let target = folder
                .join(hide_kit::holders::CONPTY_SOURCE)
                .join(&relative);
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                folders.push(relative);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    folder.join("hided.exe")
}

#[cfg(windows)]
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + WITHIN;
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(250)); // wait-ok: a pane's screen and a process's end, read by polling; the deadline bounds it
    }
}

/// D-77 (6) on a Windows host: the node its host started starts a guest's
/// pane, takes its input, and the pane outlives that node, which ends with
/// its owner, as a node a host lost or started again; a later node finds
/// the same pane, and closing it ends its program. Before the host let a
/// resident leave its node's job, every pane a paired computer opened on a
/// Windows host was refused (`this process's job does not let a process
/// leave it`), on the operator's laptop on 2026-11-24.
#[cfg(windows)]
#[test]
#[ignore = "the built pane holder and its ConPTY runtime; set HIDE_TEST_HOLDER_BIN and HIDE_HOLDER_CONPTY"]
fn a_windows_hosts_node_starts_a_guests_pane_that_outlives_the_node_and_ends_when_closed() {
    use std::sync::Arc;

    use hide_node_link::NodeLink;
    use hide_node_link::holders::{HolderStart, PaneHolders, PaneInput};
    use hide_node_link::link_panes::LinkPanes;

    const PANE: &str = "guest-pane";
    let root = short_root();
    let program = windows_build(&root.path().join("p"));
    let linked = |node: &Node| {
        let stream = LocalStream::connect(&node.dir.join(launch::LINK_ENDPOINT)).unwrap();
        let link: Arc<dyn NodeLink> = Arc::new(
            hide_node::ssh::host::over_local_stream(
                "guest:windows",
                stream,
                None,
                Box::new(|_| {}),
            )
            .unwrap(),
        );
        let _: Hello = hide_node_link::call_as(&*link, Call::Hello, WITHIN).unwrap();
        let panes = LinkPanes::new(
            Arc::clone(&link),
            Arc::new(|record| eprintln!("guest pane record: {record}")),
        );
        (link, panes)
    };
    let screen = |panes: &LinkPanes| panes.read(PANE, 40).map(|(text, _)| text).ok();

    let first = Node::start_from(root.path(), &program);
    first.ready();
    let node = first.child.id();
    let (link, panes) = linked(&first);
    let start = HolderStart {
        pane: PANE.to_owned(),
        program: Some(vec![
            "cmd.exe".to_owned(),
            "/d".to_owned(),
            "/k".to_owned(),
            "echo guest_pane_up".to_owned(),
        ]),
        cwd: hide_platform::path::to_wire(root.path()).unwrap(),
        env: Vec::new(),
        rows: 24,
        cols: 100,
        notice: None,
    };
    // As the host's core starts a pane: a number, its reservation, its
    // start, then the number retired.
    let request = panes.request().unwrap();
    panes
        .reserve(&start, None, request)
        .and_then(|reserved| panes.start_reserved(&start, reserved, request))
        .unwrap_or_else(|error| panic!("the guest's pane did not start: {error:?}"));
    panes.retire(request).unwrap();
    let proof = panes.proof(PANE).unwrap();
    wait_for("the pane's first output", || {
        screen(&panes)?.contains("guest_pane_up").then_some(())
    });
    panes
        .input(PANE, PaneInput::Bytes(b"echo typed_%OS%\r".to_vec()))
        .unwrap();
    wait_for("the pane's answer to what was typed", || {
        screen(&panes)?.contains("typed_Windows_NT").then_some(())
    });

    // The node ends with its owner, as when the host ends or starts it
    // again; the pane's holder left the node's job and keeps its program.
    drop(panes);
    drop(link);
    drop(first);
    wait_for("the node's end", || {
        (!hide_platform::process::is_alive(node)).then_some(())
    });
    assert_eq!(
        hide_platform::process::start_time(proof.program).ok(),
        Some(proof.started),
        "the pane's program ended with its node"
    );

    let second = Node::start_from(root.path(), &program);
    second.ready();
    let (link, panes) = linked(&second);
    let recovery = panes.recovered(WITHIN).unwrap();
    assert_eq!(recovery.present, [PANE.to_owned()], "{recovery:?}");
    assert_eq!(panes.proof(PANE).unwrap(), proof);
    panes.terminate(PANE).unwrap();
    wait_for("the closed pane's program to end", || {
        (hide_platform::process::start_time(proof.program).ok() != Some(proof.started))
            .then_some(())
    });
    drop(panes);
    drop(link);
}
