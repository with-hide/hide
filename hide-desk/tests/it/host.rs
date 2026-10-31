//! A Desk process end to end against a stand-in ACP agent: this test binary
//! run again in the agent's role. It proves what a caller of the host relies
//! on: the session opens in its ask mode, a turn's reply and its permission
//! request arrive, a refused ask mode keeps the session unused, a command
//! the agent runs finds Desk's own `hide` first, and ending the host leaves
//! nothing it started running.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_desk::AdapterId;
use hide_desk::adapter::{self, Launch, Paths};
use hide_desk::host::Host;
use hide_desk::rpc::{Incoming, Sink};
use hide_desk::session::{self, OpenError};
use serde_json::{Value, json};

/// The variable that puts this binary in the stand-in agent's role.
const ROLE: &str = "HIDE_DESK_TEST_ROLE";
const WAIT: Duration = Duration::from_secs(30);

/// The stand-in agent: answers ACP on its standard streams. Run as a test
/// without [`ROLE`], it returns at once.
#[test]
#[allow(clippy::disallowed_methods)] // the sleeper stays alive until the host ends it
fn stand_in_agent() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    if role == "hide" {
        let at = std::env::current_exe().unwrap();
        println!("HIDE-AT {}", at.display());
        return;
    }
    if role == "sleeper" {
        println!("SLEEPING");
        std::thread::sleep(Duration::from_secs(120));
        return;
    }
    let refuse_mode = role == "refuse-mode";
    let stdout = std::io::stdout();
    let send = |value: Value| {
        let mut out = stdout.lock();
        writeln!(out, "{value}").unwrap();
        out.flush().unwrap();
    };
    let mut lines = BufReader::new(std::io::stdin()).lines();
    let mut sleeper = None;
    while let Some(Ok(line)) = lines.next() {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message["id"].clone();
        match message["method"].as_str() {
            Some("initialize") => send(json!({"jsonrpc": "2.0", "id": id, "result": {
                "protocolVersion": 1, "agentCapabilities": {"loadSession": false}}})),
            Some("session/new") => send(json!({"jsonrpc": "2.0", "id": id, "result": {
                "sessionId": "s1",
                "configOptions": [{"id": "model", "category": "model", "currentValue": "m1",
                    "options": [{"value": "m1", "name": "Model one"}]}],
                "_meta": {"piAcp": {"startupInfo": "pi v1 started"}}}})),
            Some("session/set_mode") if refuse_mode => send(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32602, "message": "no such mode"}})),
            Some("session/set_mode") => send(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
            Some("session/prompt") => {
                let text = message["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or_default();
                if text == "which hide" {
                    // As the agent's shell runs `hide`: found on the PATH
                    // the Desk process was started with.
                    let output = Command::new("hide")
                        .args(STAND_IN)
                        .arg("--nocapture")
                        .env(ROLE, "hide")
                        .output()
                        .unwrap();
                    let found = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .find_map(|line| line.strip_prefix("HIDE-AT ").map(str::to_owned))
                        .unwrap_or_default();
                    send(
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": found}}}}),
                    );
                    send(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}));
                    continue;
                }
                if text == "start a sleeper" {
                    let child = Command::new(std::env::current_exe().unwrap())
                        .args(STAND_IN)
                        .env(ROLE, "sleeper")
                        .stdout(Stdio::null())
                        .spawn()
                        .unwrap();
                    send(
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": child.id().to_string()}}}}),
                    );
                    sleeper = Some(child);
                    send(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}));
                    continue;
                }
                send(
                    json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "pong"}}}}),
                );
                send(
                    json!({"jsonrpc": "2.0", "id": "ask-1", "method": "session/request_permission",
                    "params": {"sessionId": "s1",
                        "toolCall": {"toolCallId": "t1", "kind": "execute", "title": "touch x",
                            "rawInput": {"command": "touch x"}},
                        "options": [
                            {"optionId": "yes", "kind": "allow_once", "name": "Allow"},
                            {"optionId": "always", "kind": "allow_always", "name": "Always"},
                            {"optionId": "no", "kind": "reject_once", "name": "Reject"}]}}),
                );
                // The answer comes before anything else is read.
                let answer: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                let chosen = answer["result"]["outcome"]["optionId"]
                    .as_str()
                    .unwrap_or("none");
                send(
                    json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": format!("chose {chosen}")}}}}),
                );
                send(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}));
            }
            _ => send(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": "unknown method"}})),
        }
    }
    drop(sleeper);
}

/// The arguments that run this binary as the stand-in agent alone.
const STAND_IN: [&str; 4] = ["--exact", "host::stand_in_agent", "-q", "--test-threads=1"];

fn launch(role: &str, cwd: &Path) -> Launch {
    Launch {
        program: std::env::current_exe().unwrap(),
        args: STAND_IN.into_iter().map(OsString::from).collect(),
        env: vec![(ROLE.into(), role.into())],
        cwd: cwd.to_owned(),
        files: Vec::new(),
        pi_marker: None,
    }
}

fn start(role: &str, cwd: &Path) -> (Host, Receiver<Incoming>) {
    start_launch(&launch(role, cwd))
}

fn start_launch(launch: &Launch) -> (Host, Receiver<Incoming>) {
    let (sender, received) = mpsc::channel();
    let sender = Mutex::new(sender);
    let sink: Sink = Arc::new(move |incoming| {
        let _ = sender.lock().unwrap().send(incoming);
    });
    (Host::start(launch, sink).unwrap(), received)
}

/// A copy of this binary named `hide` in `folder`, which answers where it is.
fn hide_in(folder: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let name = if cfg!(windows) { "hide.exe" } else { "hide" };
    let program = folder.join(name);
    std::fs::copy(std::env::current_exe().unwrap(), &program).unwrap();
    program
}

fn next(received: &Receiver<Incoming>) -> Incoming {
    received.recv_timeout(WAIT).expect("the agent sent nothing")
}

fn text(incoming: Incoming) -> String {
    match incoming {
        Incoming::Notification { params, .. } => match session::read_update(&params) {
            session::Update::Text(text) => text,
            other => panic!("not reply text: {other:?}"),
        },
        other => panic!("not a notification: {other:?}"),
    }
}

#[test]
fn a_turn_s_reply_and_its_permission_request_reach_the_owner() {
    let folder = tempfile::tempdir().unwrap();
    let (host, received) = start("agent", folder.path());
    let connection = host.connection().clone();
    let opened = session::open(&connection, AdapterId::Claude, folder.path(), None, &|| {
        Ok(())
    })
    .unwrap();
    assert_eq!(opened.session_id, "s1");
    assert!(!opened.resumed);
    assert_eq!(opened.choices.model.unwrap().current.as_deref(), Some("m1"));
    // The startup text the agent says it writes on its own is not a reply.
    assert_eq!(opened.prelude.as_deref(), Some("pi v1 started"));

    session::prompt(&connection, "s1", "ping", 7).unwrap();
    assert_eq!(text(next(&received)), "pong");
    let Incoming::Request { id, method, params } = next(&received) else {
        panic!("no permission request");
    };
    assert_eq!(method, "session/request_permission");
    let request = hide_desk::permission::read_request(AdapterId::Claude, &params, None);
    assert_eq!(request.allow_once(), Some("yes"));
    let reject = request.reject().unwrap().to_owned();
    connection
        .respond(
            &id,
            json!({"outcome": {"outcome": "selected", "optionId": reject}}),
        )
        .unwrap();
    assert_eq!(text(next(&received)), "chose no");
    let Incoming::Reply { tag, result } = next(&received) else {
        panic!("no prompt answer");
    };
    assert_eq!(tag, 7);
    assert_eq!(session::stop_reason(&result.unwrap()), Some("end_turn"));
    host.end().unwrap();
}

#[test]
fn a_session_whose_ask_mode_is_refused_is_not_used() {
    let folder = tempfile::tempdir().unwrap();
    let (host, _received) = start("refuse-mode", folder.path());
    let opened = session::open(
        host.connection(),
        AdapterId::Codex,
        folder.path(),
        None,
        &|| Ok(()),
    );
    assert!(
        matches!(opened, Err(OpenError::AskModeUnconfirmed(_))),
        "{opened:?}"
    );
    host.end().unwrap();
}

/// Desk's Codex runs its commands with the Desk process's `PATH` (its login
/// shell is off), so the `hide` beside hided must come before any `hide` on
/// the account's own `PATH`, such as an older installed one.
#[test]
fn a_command_desk_s_codex_runs_finds_desk_s_own_hide_before_the_account_s() {
    let root = tempfile::tempdir().unwrap();
    let desk = root.path().join("desk");
    let desk_hide = hide_in(&root.path().join("app"));
    let account = root.path().join("account-bin");
    hide_in(&account);
    let codex_home = desk.join("codex");
    let mut launch = adapter::launch(
        AdapterId::Codex,
        &Paths {
            package: &root.path().join("adapter"),
            node: &std::env::current_exe().unwrap(),
            hide_dir: desk_hide.parent().unwrap(),
            search: Some(&account.clone().into_os_string()),
            desk: &desk,
            capability: &root.path().join("desk.json"),
            codex_home: &codex_home,
            own_cli: None,
        },
        "n1",
    );
    launch.args = STAND_IN.into_iter().map(OsString::from).collect();
    launch.env.push((ROLE.into(), "agent".into()));
    let (host, received) = start_launch(&launch);
    // Desk's Codex configuration and rules are in its own home before it runs.
    assert!(codex_home.join("config.toml").is_file());
    assert!(codex_home.join("rules").join("hide-desk.rules").is_file());
    let connection = host.connection().clone();
    session::open(&connection, AdapterId::Codex, &desk, None, &|| Ok(())).unwrap();

    session::prompt(&connection, "s1", "which hide", 1).unwrap();
    let found = text(next(&received));
    assert_eq!(
        std::fs::canonicalize(&found).unwrap(),
        std::fs::canonicalize(&desk_hide).unwrap(),
        "{found}"
    );
    host.end().unwrap();
}

#[test]
fn ending_the_host_leaves_nothing_it_started_running() {
    let folder = tempfile::tempdir().unwrap();
    let (host, received) = start("agent", folder.path());
    let connection = host.connection().clone();
    session::open(&connection, AdapterId::Claude, folder.path(), None, &|| {
        Ok(())
    })
    .unwrap();
    session::prompt(&connection, "s1", "start a sleeper", 1).unwrap();
    let sleeper: u32 = text(next(&received)).parse().unwrap();
    assert!(hide_platform::process::descends_from(sleeper, host.pid()));
    let agent = host.pid();

    host.end().unwrap();
    assert!(
        gone_within(&[sleeper, agent], WAIT),
        "a Desk process outlived its host"
    );
    // Every message after the end is the stream's close.
    while let Ok(incoming) = received.try_recv() {
        assert!(
            matches!(incoming, Incoming::Closed { .. } | Incoming::Reply { .. }),
            "{incoming:?}"
        );
    }
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn gone_within(pids: &[u32], wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if !pids
            .iter()
            .any(|pid| hide_platform::process::is_alive(*pid))
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
