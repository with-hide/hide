//! A phone reads one agent pane through the engine that runs it (PRD D-13).
//! A reply or key first waits for the pane node's existing creation holder,
//! then goes to the engine as input; success means the engine answered the
//! write. Neither changes the core's attach set, focus, terminal size or
//! visible tab.

use std::time::Duration;

use herdr_core::engine::{EngineError, EngineHandle};
use hide_node_link::terminal::{InputPermitError, TerminalNode};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The rows a detail asks for first, how many a pull adds, and the most it may hold.
pub const FIRST_LINES: u32 = 200;
pub const MORE_LINES: u32 = 200;
pub const MAX_LINES: u32 = 1000;

/// The longest reply one send may carry, in characters.
pub const MAX_REPLY_CHARS: usize = 2000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PaneError {
    /// The pane is closed or no longer exists.
    Gone,
    /// The SSH device that owns it is not connected.
    DeviceUnreachable,
    /// This Mac's Herdr did not answer.
    Unavailable(String),
    /// The agent is ending, asleep or waking, and what is under it is its shell.
    Asleep,
    /// The node refused or discarded the input before any bytes were written.
    InputRefused,
}

impl PaneError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::DeviceUnreachable => "device_unreachable",
            Self::Unavailable(_) => "unavailable",
            Self::Asleep => "asleep",
            Self::InputRefused => "input_refused",
        }
    }
}

fn classify(error: EngineError) -> PaneError {
    match error.code() {
        Some(code) if code.contains("not_found") => PaneError::Gone,
        _ => PaneError::Unavailable(error.to_string()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rows {
    pub text: String,
    pub truncated: bool,
}

pub fn read(engine: &EngineHandle, pane_id: &str, lines: u32) -> Result<Rows, PaneError> {
    let read = engine
        .read_rows(pane_id, lines.min(MAX_LINES), TIMEOUT)
        .map_err(classify)?;
    Ok(Rows {
        text: read.text,
        truncated: read.truncated,
    })
}

/// The keys a phone may send, as Herdr key-combo strings.
pub fn herdr_key(key: &str) -> Option<&'static str> {
    match key {
        "enter" => Some("enter"),
        "escape" => Some("esc"),
        "up" => Some("up"),
        "down" => Some("down"),
        "ctrl_c" => Some("ctrl+c"),
        _ => None,
    }
}

/// Why a reply was not sent; checked before anything reaches Herdr.
pub fn reply_problem(text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return Some("empty");
    }
    if text.chars().count() > MAX_REPLY_CHARS {
        return Some("too_long");
    }
    if text.chars().any(char::is_control) {
        return Some("control_characters");
    }
    None
}

/// What one input carries: a reply that ends with Enter, or one key.
pub enum Input<'a> {
    Reply(&'a str),
    Key(&'static str),
}

pub fn wait_for_input(
    terminals: &dyn TerminalNode,
    core_pane_id: &str,
    input: &Input<'_>,
) -> Result<(), PaneError> {
    let bytes = match &input {
        // Reserve payload plus Enter; Herdr's bracketed-paste wrapper is
        // protocol framing and is not part of this held payload budget.
        Input::Reply(text) => text.len() + 1,
        // Herdr's supported quick keys encode as one byte or a three-byte
        // cursor sequence (CSI/SS3), not the length of their logical name.
        Input::Key("up" | "down") => 3,
        Input::Key("enter" | "esc" | "ctrl+c") => 1,
        Input::Key(_) => return Err(PaneError::InputRefused),
    };
    let (reply, result) = std::sync::mpsc::channel();
    terminals.input_permit(
        core_pane_id,
        bytes,
        Box::new(move |outcome| {
            let _ = reply.send(outcome);
        }),
    );
    match result.recv_timeout(Duration::from_secs(35)) {
        Ok(Ok(())) => {}
        Ok(Err(InputPermitError::Asleep)) => return Err(PaneError::Asleep),
        Ok(Err(_)) | Err(_) => return Err(PaneError::InputRefused),
    }
    Ok(())
}

pub fn send(engine: &EngineHandle, pane_id: &str, input: Input<'_>) -> Result<(), PaneError> {
    match input {
        Input::Reply(text) => engine.submit_text(pane_id, text, TIMEOUT),
        Input::Key(key) => engine.press_key(pane_id, key, TIMEOUT),
    }
    .map_err(classify)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_quick_keys_map_to_herdr_keys() {
        assert_eq!(herdr_key("escape"), Some("esc"));
        assert_eq!(herdr_key("ctrl_c"), Some("ctrl+c"));
        assert_eq!(herdr_key("ctrl_d"), None);
        assert_eq!(herdr_key("prefix+c"), None);
    }

    #[test]
    fn a_reply_must_be_one_short_line() {
        assert_eq!(reply_problem("   "), Some("empty"));
        assert_eq!(reply_problem("yes\n"), Some("control_characters"));
        assert_eq!(
            reply_problem(&"가".repeat(MAX_REPLY_CHARS + 1)),
            Some("too_long")
        );
        assert_eq!(reply_problem("PR 병합 전에 한 번 더 확인해 주세요"), None);
    }

    /// A phone must receive neither an early success nor a false success
    /// when the holder refuses it; Herdr's acknowledgment remains decisive.
    #[test]
    fn a_phone_reply_waits_for_both_the_node_permission_and_herdr_acknowledgment() {
        use hide_node_link::terminal::{GridSize, InputPermitReply, KeyTarget, TerminalControl};
        use std::io::{BufRead, BufReader, Write};
        use std::sync::{Arc, Mutex, mpsc};

        struct Gate(Mutex<mpsc::Sender<InputPermitReply>>);
        impl TerminalNode for Gate {
            fn input_permit(&self, _: &str, bytes: usize, reply: InputPermitReply) {
                assert_eq!(bytes, 4);
                self.0.lock().unwrap().send(reply).unwrap();
            }
            fn control(&self, _: TerminalControl) {
                panic!("phone must not attach or focus");
            }
            fn key(&self, _: KeyTarget, _: Vec<u8>, _: u64) {
                panic!("phone keeps Herdr logical input");
            }
            fn view(&self, _: &str, _: GridSize, _: bool) {
                panic!("phone must not resize");
            }
            fn redraw(&self, _: &str) {
                panic!("phone must not redraw");
            }
        }
        for allowed in [true, false] {
            let (asked, permission) = mpsc::channel();
            let gate = Arc::new(Gate(Mutex::new(asked)));
            // A Herdr that holds its answer until `release`, so an early
            // success would show before it.
            let root = tempfile::tempdir().unwrap();
            let socket = root.path().join("herdr.sock");
            let listener = hide_platform::ipc::LocalListener::bind(&socket).unwrap();
            let closer = listener.closer();
            let (written, request) = mpsc::channel();
            let (release, ack) = mpsc::channel::<()>();
            let server = std::thread::spawn(move || {
                let Ok(mut stream) = listener.accept() else {
                    return;
                };
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let sent: serde_json::Value = serde_json::from_str(&line).unwrap();
                written.send(sent.clone()).unwrap();
                ack.recv_timeout(Duration::from_secs(5)).unwrap();
                writeln!(
                    stream,
                    "{}",
                    serde_json::json!({"id": sent["id"], "ok": true, "result": {}})
                )
                .unwrap();
            });
            let engine = EngineHandle::herdr(hide_node::herdr(&socket));
            let (done, result) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                done.send(
                    wait_for_input(gate.as_ref(), "w1:p1", &Input::Reply("yes"))
                        .and_then(|()| send(&engine, "w1:p1", Input::Reply("yes"))),
                )
                .unwrap();
            });
            let grant = permission.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(request.try_recv(), Err(mpsc::TryRecvError::Empty)));
            assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            grant(if allowed {
                Ok(())
            } else {
                Err(InputPermitError::Discarded)
            });
            if allowed {
                let sent = request.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(sent["method"], "pane.send_input");
                assert_eq!(
                    sent["params"],
                    serde_json::json!({"pane_id":"w1:p1", "text":"yes", "keys":["enter"]})
                );
                assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
                release.send(()).unwrap();
                assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(()));
            } else {
                assert_eq!(
                    result.recv_timeout(Duration::from_secs(5)).unwrap(),
                    Err(PaneError::InputRefused)
                );
                assert!(request.try_recv().is_err());
            }
            worker.join().unwrap();
            closer.close();
            server.join().unwrap();
        }
    }
}
