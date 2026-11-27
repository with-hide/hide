//! A Desk process end to end against a stand-in ACP agent: this test binary
//! run again in the agent's role. It proves what a caller of the host relies
//! on: the session opens, a turn's reply and its permission request
//! arrive, a command the agent runs finds Desk's own `hide` first, the
//! agent has the account's environment without a pane's identity or Hide's
//! own variables, and ending the host leaves nothing it started running.

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
use hide_desk::permission::allow_once;
use hide_desk::rpc::{Incoming, Sink};
use hide_desk::session::{self, OpenError};
use serde_json::{Value, json};

/// The variable that puts this binary in the stand-in agent's role.
pub(crate) const ROLE: &str = "HIDE_DESK_TEST_ROLE";
/// A file the stand-in writes its pid to as it starts, when set.
pub(crate) const PID_FILE: &str = "HIDE_DESK_TEST_PID";
const WAIT: Duration = Duration::from_secs(30);

/// The stand-in agent: answers ACP on its standard streams. Run as a test
/// without [`ROLE`], it returns at once.
#[test]
#[allow(clippy::disallowed_methods)] // the sleeper stays alive until the host ends it
fn stand_in_agent() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    if let Ok(file) = std::env::var(PID_FILE) {
        std::fs::write(file, std::process::id().to_string()).unwrap();
    }
    if role == "stuck" {
        // Answers nothing, until its input closes.
        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        return;
    }
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
    // Loads a session as ACP says: the conversation so far replayed as
    // updates before the load answers.
    let replays = role == "replay";
    // codex-acp 2.5.0 with a login it can no longer refresh: it logs itself
    // out, says so, and refuses the open with an internal error.
    let stale_login = role == "stale-login";
    // The agent's commands as its session opens (`available_commands_update`):
    // after the session's answer, before it, or twice, the later list shorter.
    let list = |names: &[&str]| {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1",
            "update": {"sessionUpdate": "available_commands_update", "availableCommands":
                names.iter().map(|name| json!({"name": name, "description": format!("{name} it")})).collect::<Vec<_>>()}}})
    };
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
                "protocolVersion": 1, "agentCapabilities": {"loadSession": replays,
                    "sessionCapabilities": if role.starts_with("lists") { json!({"delete": {}}) } else { json!({}) }}}})),
            // Records the session it was asked to delete beside its pid file.
            Some("session/delete") => {
                if let Ok(file) = std::env::var(PID_FILE) {
                    let session = message["params"]["sessionId"].as_str().unwrap_or_default();
                    std::fs::write(format!("{file}.deleted"), session).unwrap();
                }
                send(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
            }
            Some("session/new") if role == "sign-in" => send(json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": session::AUTH_REQUIRED, "message": "Authentication required"}})),
            Some("session/new") if role.starts_with("lists") => {
                let answer = json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": "s1"}});
                match role.as_str() {
                    "lists-early" => {
                        send(list(&["compact", "logout"]));
                        send(answer);
                    }
                    "lists-twice" => {
                        send(answer);
                        send(list(&["compact", "review"]));
                        send(list(&["compact"]));
                    }
                    "lists-none" => send(answer),
                    "lists-empty" => {
                        send(answer);
                        send(list(&["logout"]));
                    }
                    _ => {
                        send(answer);
                        send(list(&["compact", "logout"]));
                    }
                }
            }
            Some("session/new") if stale_login => {
                send(json!({"jsonrpc": "2.0", "method": "_auth/status_update",
                    "params": {"authStatus": {"kind": "none", "label": "Not logged in"}}}));
                send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603,
                    "message": "Internal error", "data": "Please log out and sign in again.\n\nYou have been logged out. Please try again."}}));
            }
            Some("session/load") if replays => {
                for said in ["응답: 알파첫째", "응답: 브라보둘째"] {
                    send(
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "s1", "update": {"sessionUpdate": "user_message_chunk",
                        "content": {"type": "text", "text": said}}}}),
                    );
                    send(
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": said}}}}),
                    );
                }
                send(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
            }
            Some("session/prompt") if replays => {
                send(
                    json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                    "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "응답: 델타넷째"}}}}),
                );
                send(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}));
            }
            Some("session/new") => send(json!({"jsonrpc": "2.0", "id": id, "result": {
                "sessionId": "s1",
                "configOptions": [{"id": "model", "category": "model", "currentValue": "m1",
                    "options": [{"value": "m1", "name": "Model one"}]}],
                "_meta": {"piAcp": {"startupInfo": "pi v1 started"}}}})),
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
                if let Some(names) = text.strip_prefix("env ") {
                    // As the agent's own configuration reads a variable.
                    let seen: Vec<String> = names
                        .split(',')
                        .map(|name| match std::env::var(name) {
                            Ok(value) => format!("{name}={value}"),
                            Err(_) => format!("{name} unset"),
                        })
                        .collect();
                    send(
                        json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": "s1", "update": {"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": seen.join("\n")}}}}),
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

pub(crate) fn launch(role: &str, cwd: &Path) -> Launch {
    Launch {
        program: std::env::current_exe().unwrap(),
        args: STAND_IN.into_iter().map(OsString::from).collect(),
        env: vec![(ROLE.into(), role.into())],
        cwd: cwd.to_owned(),
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
            session::Update::Text { text, .. } => text,
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
    let opened = session::open(&connection, folder.path(), None).unwrap();
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
    let request = hide_desk::permission::read_request(&params, None);
    assert_eq!(allow_once(&request.choices), Some("yes"));
    let reject = request
        .choices
        .iter()
        .find(|choice| choice.kind.rejects())
        .unwrap()
        .id
        .clone();
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

/// letter-47179: a session loaded after `다시 시작` replays the
/// conversation, and the owner reads those updates only after it sent the
/// next prompt. Numbered as they arrived, every replayed one comes before
/// the prompt's mark, so the turn owns its own answer alone.
#[test]
fn a_loaded_session_s_replay_arrives_before_the_next_prompt_s_mark() {
    let folder = tempfile::tempdir().unwrap();
    let (sender, received) = mpsc::channel();
    let sender = Mutex::new(sender);
    let arrivals = session::Arrivals::default();
    let sink = arrivals.sink(move |number, incoming| {
        let _ = sender.lock().unwrap().send((number, incoming));
    });
    let host = Host::start(&launch("replay", folder.path()), sink).unwrap();
    let connection = host.connection().clone();
    let opened = session::open(&connection, folder.path(), Some("s1")).unwrap();
    assert!(opened.resumed);

    // As Desk's worker does: the mark read just before the prompt, and
    // every message read only after it, the replay still waiting.
    let mut turn = session::TurnReader::new(opened.prelude.clone(), None, None);
    let from = arrivals.next();
    session::prompt(&connection, "s1", "델타넷째", 9).unwrap();
    turn.start(9, from);
    let (mut replayed, mut reply) = (0, Vec::new());
    loop {
        let (number, incoming) = received.recv_timeout(WAIT).expect("the agent sent nothing");
        match incoming {
            Incoming::Notification { params, .. } => {
                let update = session::read_update(&params);
                if matches!(update, session::Update::Text { .. }) && number < from {
                    replayed += 1;
                }
                reply.extend(turn.read(number, &update).map(|(_, text)| text));
            }
            Incoming::Reply { tag, .. } => {
                assert_eq!(turn.end(tag), Some(session::Answered::default()));
                break;
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
    assert_eq!(replayed, 2, "the load replayed both earlier answers");
    assert_eq!(reply, ["응답: 델타넷째"]);
    host.end().unwrap();
}

/// letter-47232: an agent that refuses the open for a login it could no
/// longer refresh answers an internal error, but says it is signed out
/// first, so its owner knows the refusal was for the sign-in by the time
/// the refusal arrives.
#[test]
fn an_agent_says_it_is_signed_out_before_it_refuses_the_open() {
    let folder = tempfile::tempdir().unwrap();
    let signed_out = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&signed_out);
    let sink: Sink = Arc::new(move |incoming| {
        if let Incoming::Notification { method, params } = &incoming
            && let Some(signed_in) = session::sign_in_status(method, params)
        {
            *seen.lock().unwrap() = Some(!signed_in);
        }
    });
    let host = Host::start(&launch("stale-login", folder.path()), sink).unwrap();
    let opened = session::open(host.connection(), folder.path(), None);
    assert!(matches!(opened, Err(OpenError::Failed(_))), "{opened:?}");
    assert_eq!(*signed_out.lock().unwrap(), Some(true));
    host.end().unwrap();
}

/// The `hide` beside hided comes first on the Desk process's `PATH`, before
/// any `hide` on the account's own `PATH`, such as an older installed one,
/// so a command the agent runs with the process's `PATH` reaches Desk's.
#[test]
fn a_command_desk_s_agent_runs_finds_desk_s_own_hide_before_the_account_s() {
    let root = tempfile::tempdir().unwrap();
    let desk = root.path().join("desk");
    let desk_hide = hide_in(&root.path().join("app"));
    let account = root.path().join("account-bin");
    hide_in(&account);
    let mut launch = adapter::launch(
        AdapterId::CODEX,
        &Paths {
            package: Some((
                &root.path().join("adapter"),
                &std::env::current_exe().unwrap(),
            )),
            own_cli: None,
            hide_dir: desk_hide.parent().unwrap(),
            search: Some(&account.clone().into_os_string()),
            account: None,
            desk: &desk,
            capability: Some(&root.path().join("desk.json")),
        },
    )
    .unwrap();
    launch.args = STAND_IN.into_iter().map(OsString::from).collect();
    launch.env.push((ROLE.into(), "agent".into()));
    let (host, received) = start_launch(&launch);
    let connection = host.connection().clone();
    session::open(&connection, &desk, None).unwrap();

    session::prompt(&connection, "s1", "which hide", 1).unwrap();
    let found = text(next(&received));
    assert_eq!(
        std::fs::canonicalize(&found).unwrap(),
        std::fs::canonicalize(&desk_hide).unwrap(),
        "{found}"
    );
    host.end().unwrap();
}

/// A provider key the account's startup files export reaches the agent, as
/// it reaches one the operator starts in a terminal (OpenCode's
/// `{env:BAI_API_KEY}`) and so does its own configuration, as in its
/// terminal (D-92), while a pane's identity and Hide's own variables do not,
/// wherever they come from.
#[test]
fn the_agent_has_the_account_s_environment_but_no_pane_identity_or_hide_s_own() {
    let root = tempfile::tempdir().unwrap();
    let desk = root.path().join("desk");
    let account: Vec<(OsString, OsString)> = [
        ("DESK_PROVIDER_KEY", "from-the-profile"),
        ("HERDR_PANE_ID", "w1:p1"),
        ("HIDE_STATE_DIR", "/elsewhere"),
        ("hide_cap_ref", "/a-pane-s.json"),
        ("OPENCODE_CONFIG_CONTENT", "{}"),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect();
    let mut launch = adapter::launch(
        AdapterId::OPENCODE,
        &Paths {
            package: None,
            own_cli: Some(&std::env::current_exe().unwrap()),
            hide_dir: &root.path().join("app"),
            search: None,
            account: Some(&account),
            desk: &desk,
            capability: Some(&root.path().join("desk.json")),
        },
    )
    .unwrap();
    launch.args = STAND_IN.into_iter().map(OsString::from).collect();
    launch.env.push((ROLE.into(), "agent".into()));
    let (host, received) = start_launch(&launch);
    let connection = host.connection().clone();
    session::open(&connection, &desk, None).unwrap();

    session::prompt(
        &connection,
        "s1",
        "env DESK_PROVIDER_KEY,HERDR_PANE_ID,HIDE_STATE_DIR,HIDE_CAP_REF,OPENCODE_CONFIG_CONTENT",
        1,
    )
    .unwrap();
    let seen = text(next(&received));
    let capability = root.path().join("desk.json");
    let expected = [
        "DESK_PROVIDER_KEY=from-the-profile".to_owned(),
        "HERDR_PANE_ID unset".to_owned(),
        "HIDE_STATE_DIR unset".to_owned(),
        format!("HIDE_CAP_REF={}", capability.display()),
    ];
    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(lines[..4], expected, "{seen}");
    assert_eq!(lines[4], "OPENCODE_CONFIG_CONTENT={}", "{seen}");
    host.end().unwrap();
}

#[test]
fn ending_the_host_leaves_nothing_it_started_running() {
    let folder = tempfile::tempdir().unwrap();
    let (host, received) = start("agent", folder.path());
    let connection = host.connection().clone();
    session::open(&connection, folder.path(), None).unwrap();
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
