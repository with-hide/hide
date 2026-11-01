//! What a node writes to a Claude Code session's messaging socket when the
//! input ladder hands it a letter (`hide_host::input`, `Call::Deliver`): the
//! session's protocol, one JSON object a line, `auth` with the token its
//! SessionStart hook received and then the `user` message (Claude Code
//! 2.1.316's own help text; measured 2026-11-01).

use std::io::Read;
use std::sync::atomic::AtomicBool;

use hide_node_link::input::{Delivery, InputChannel};
use hide_platform::ipc::LocalListener;

#[cfg(unix)]
#[test]
fn a_claude_session_socket_gets_the_token_then_the_message_whole() {
    let folder = tempfile::tempdir().unwrap();
    let socket = folder.path().join("session.sock");
    let listener = LocalListener::bind(&socket).unwrap();
    let reader = std::thread::spawn(move || {
        let mut stream = listener.accept().unwrap();
        let mut bytes = String::new();
        stream.read_to_string(&mut bytes).unwrap();
        bytes
    });
    let text = "🦉 A letter from parent\nsecond line";
    let delivery = hide_host::input::deliver(
        folder.path(),
        &InputChannel::ClaudeMessaging {
            socket: socket.to_string_lossy().into_owned(),
            token: "token-1".into(),
        },
        text,
        &AtomicBool::new(false),
    );
    assert_eq!(delivery, Delivery::Sent, "the socket answers nothing");
    let written = reader.join().unwrap();
    let lines: Vec<serde_json::Value> = written
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            serde_json::json!({"type": "auth", "token": "token-1"}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}),
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_socket_that_is_gone_sends_nothing_and_the_next_rung_takes_the_input() {
    let folder = tempfile::tempdir().unwrap();
    let delivery = hide_host::input::deliver(
        folder.path(),
        &InputChannel::ClaudeMessaging {
            socket: folder
                .path()
                .join("gone.sock")
                .to_string_lossy()
                .into_owned(),
            token: "token-1".into(),
        },
        "letter",
        &AtomicBool::new(false),
    );
    assert!(
        matches!(delivery, Delivery::Unavailable { .. }),
        "{delivery:?}"
    );
}

/// Windows names the session's socket as a pipe no run has measured, so the
/// rung is unavailable there and typing takes the input (`docs/INSTALL.md`,
/// What each system supports).
#[cfg(windows)]
#[test]
fn a_claude_session_socket_is_unavailable_on_windows() {
    let folder = tempfile::tempdir().unwrap();
    let delivery = hide_host::input::deliver(
        folder.path(),
        &InputChannel::ClaudeMessaging {
            socket: folder
                .path()
                .join("session.sock")
                .to_string_lossy()
                .into_owned(),
            token: "token-1".into(),
        },
        "letter",
        &AtomicBool::new(false),
    );
    assert_eq!(
        delivery,
        Delivery::Unavailable {
            reason: "unsupported_system".into()
        }
    );
}
