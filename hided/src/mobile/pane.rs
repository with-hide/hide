//! A phone reads one agent pane through Herdr's socket API (PRD D-13).
//! A reply or key first waits for the pane node's existing creation holder,
//! then uses `pane.send_input`; success means Herdr answered the write.
//! Neither changes the core's attach set, focus, terminal size or visible tab. Parameters and the answer come from
//! `herdr_core::wire`, which holds the generated types of the pinned Herdr
//! schema.

use std::sync::Arc;
use std::time::Duration;

use herdr_core::wire;
use hide_herdr_client::ApiConnector;
use hide_herdr_client::ApiError;
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

fn classify(error: ApiError) -> PaneError {
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

pub fn read(
    connector: &Arc<dyn ApiConnector>,
    pane_id: &str,
    lines: u32,
) -> Result<Rows, PaneError> {
    let params =
        wire::pane_rows_params(pane_id, lines.min(MAX_LINES)).map_err(PaneError::Unavailable)?;
    let value =
        hide_herdr_client::request_with_connector(connector.as_ref(), "pane.read", params, TIMEOUT)
            .map_err(classify)?;
    let (text, truncated) = wire::pane_rows(value).map_err(PaneError::Unavailable)?;
    Ok(Rows { text, truncated })
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

pub fn send(
    terminals: &dyn TerminalNode,
    core_pane_id: &str,
    connector: &Arc<dyn ApiConnector>,
    pane_id: &str,
    input: Input<'_>,
) -> Result<(), PaneError> {
    let bytes = match &input {
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
    let params = match input {
        Input::Reply(text) => wire::pane_reply_params(pane_id, text),
        Input::Key(key) => wire::pane_key_params(pane_id, key),
    }
    .map_err(PaneError::Unavailable)?;
    hide_herdr_client::request_with_connector(
        connector.as_ref(),
        "pane.send_input",
        params,
        TIMEOUT,
    )
    .map(|_| ())
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
        use hide_herdr_client::{ApiStream, ConnectionShutdown};
        use hide_node_link::terminal::{GridSize, InputPermitReply, KeyTarget, TerminalControl};
        use std::io::{Read, Write};
        use std::sync::{Mutex, mpsc};

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
        struct Stream {
            written: mpsc::Sender<serde_json::Value>,
            request: Vec<u8>,
            ack: Option<mpsc::Receiver<()>>,
            answer: std::io::Cursor<Vec<u8>>,
        }
        impl Read for Stream {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                if let Some(ack) = self.ack.take() {
                    ack.recv_timeout(Duration::from_secs(5))
                        .map_err(std::io::Error::other)?;
                }
                self.answer.read(bytes)
            }
        }
        impl Write for Stream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.request.extend_from_slice(bytes);
                if bytes.contains(&b'\n') {
                    self.written
                        .send(serde_json::from_slice(&self.request).unwrap())
                        .unwrap();
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl ApiStream for Stream {
            fn set_read_timeout(&self, _: Option<Duration>) -> Result<(), ApiError> {
                Ok(())
            }
            fn set_write_timeout(&self, _: Option<Duration>) -> Result<(), ApiError> {
                Ok(())
            }
            fn read_line_with_timeout(&mut self, _: Duration) -> Result<String, ApiError> {
                unreachable!()
            }
            fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
                unreachable!()
            }
        }
        struct Connector(Mutex<Option<Stream>>);
        impl ApiConnector for Connector {
            fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
                Ok(Box::new(self.0.lock().unwrap().take().expect("one write")))
            }
        }
        for allowed in [true, false] {
            let (asked, permission) = mpsc::channel();
            let gate = Arc::new(Gate(Mutex::new(asked)));
            let (written, request) = mpsc::channel();
            let (release, ack) = mpsc::channel();
            let connector: Arc<dyn ApiConnector> = Arc::new(Connector(Mutex::new(Some(Stream {
                written,
                request: Vec::new(),
                ack: Some(ack),
                answer: std::io::Cursor::new(
                    b"{\"id\":\"herdr-core:pane.send_input\",\"ok\":true,\"result\":{}}\n".to_vec(),
                ),
            }))));
            let (done, result) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                done.send(send(
                    gate.as_ref(),
                    "w1:p1",
                    &connector,
                    "w1:p1",
                    Input::Reply("yes"),
                ))
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
        }
    }
}
