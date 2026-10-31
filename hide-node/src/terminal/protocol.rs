//! The pinned Herdr's `terminal session` NDJSON stream: frames and the close
//! it reads, and the input, resize, scroll and release lines it takes. The
//! CLI is the official client; this module only frames its lines.

pub use hide_node_link::terminal::{decode_base64, encode_base64};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Control,
    Observe,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Observe => "observe",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum SessionEvent {
    Frame {
        width: u16,
        height: u16,
        full: bool,
        bytes: Vec<u8>,
    },
    /// Herdr's `terminal.closed`, or its stream's end; its reason is read by
    /// [`closed_category`].
    Closed { reason: Option<String> },
    /// A session that closed with a reason its transport names itself (a
    /// pane's holder), never read from words.
    Ended {
        category: &'static str,
        message: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Envelope {
    #[serde(rename = "terminal.frame")]
    Frame {
        encoding: String,
        width: u16,
        height: u16,
        full: bool,
        bytes: String,
    },
    #[serde(rename = "terminal.closed")]
    Closed { reason: Option<String> },
}

pub fn parse_line(line: &str) -> Result<SessionEvent, String> {
    if line.trim().is_empty() {
        return Err("terminal session emitted an empty NDJSON line".to_owned());
    }
    let envelope: Envelope = serde_json::from_str(line)
        .map_err(|error| format!("terminal session emitted invalid NDJSON: {error}"))?;
    match envelope {
        Envelope::Frame {
            encoding,
            width,
            height,
            full,
            bytes,
        } => {
            if encoding != "ansi" {
                return Err(format!(
                    "terminal session negotiated unsupported encoding {encoding:?}"
                ));
            }
            Ok(SessionEvent::Frame {
                width,
                height,
                full,
                bytes: decode_base64(&bytes)?,
            })
        }
        Envelope::Closed { reason } => Ok(SessionEvent::Closed { reason }),
    }
}

pub fn input_line(bytes: &[u8]) -> String {
    let mut line = json!({"type": "terminal.input", "bytes": encode_base64(bytes)}).to_string();
    line.push('\n');
    line
}

/// A wheel carries the pointer's cell and modifiers because Herdr uses them
/// when the application tracks the mouse. Coordinates are zero-based.
pub fn scroll_line(
    lines: i32,
    column: Option<u16>,
    row: Option<u16>,
    modifiers: u8,
) -> Option<String> {
    if lines == 0 {
        return None;
    }
    let mut line = json!({
        "type": "terminal.scroll",
        "direction": if lines > 0 { "up" } else { "down" },
        "lines": lines.unsigned_abs().min(u16::MAX.into()) as u16,
        "source": "wheel",
        "column": column,
        "row": row,
        "modifiers": modifiers,
    })
    .to_string();
    line.push('\n');
    Some(line)
}

pub fn resize_line(rows: u16, cols: u16) -> Result<String, String> {
    if rows == 0 || cols == 0 {
        return Err("terminal dimensions must be positive".to_owned());
    }
    let mut line = json!({
        "type": "terminal.resize",
        "cols": cols,
        "rows": rows,
        "cell_width_px": 0,
        "cell_height_px": 0,
    })
    .to_string();
    line.push('\n');
    Ok(line)
}

pub fn release_line() -> String {
    "{\"type\":\"terminal.release\"}\n".to_owned()
}

/// Why a session ended: another client holds the pane, the stream ended
/// without a reason, or Herdr closed it.
pub fn closed_category(reason: Option<&str>) -> &'static str {
    let Some(reason) = reason else {
        return "transport_eof";
    };
    let normalized = reason.to_ascii_lowercase();
    if normalized == "terminal attach taken over"
        || (normalized.contains("already has an attached client")
            && normalized.contains("retry with --takeover"))
    {
        "owner_conflict"
    } else {
        "terminal_closed"
    }
}

pub fn session_arguments(mode: Mode, pane_id: &str, rows: u16, cols: u16) -> Vec<String> {
    vec![
        "terminal".to_owned(),
        "session".to_owned(),
        mode.as_str().to_owned(),
        pane_id.to_owned(),
        "--cols".to_owned(),
        cols.to_string(),
        "--rows".to_owned(),
        rows.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn official_terminal_ndjson_boundary_decodes_frames_and_closed_reasons() {
        let frame = parse_line(
            r#"{"type":"terminal.frame","seq":7,"encoding":"ansi","width":100,"height":30,"full":true,"bytes":"G1szMW0="}"#,
        )
        .expect("frame parses");
        assert_eq!(
            frame,
            SessionEvent::Frame {
                width: 100,
                height: 30,
                full: true,
                bytes: b"\x1b[31m".to_vec(),
            }
        );

        let reason = "terminal attach failed: terminal 42 already has an attached client; retry with --takeover";
        let closed = parse_line(&format!(
            r#"{{"type":"terminal.closed","reason":{}}}"#,
            serde_json::to_string(reason).expect("reason JSON")
        ))
        .expect("closed parses");
        assert_eq!(
            closed,
            SessionEvent::Closed {
                reason: Some(reason.to_owned())
            }
        );
        assert_eq!(closed_category(Some(reason)), "owner_conflict");
        assert_eq!(
            closed_category(Some("terminal attach taken over")),
            "owner_conflict"
        );
        assert_eq!(closed_category(None), "transport_eof");
    }

    #[test]
    fn official_terminal_control_boundary_encodes_input_resize_scroll_and_release() {
        let input: Value = serde_json::from_str(input_line(b"hello\n").trim_end()).unwrap();
        assert_eq!(input["type"], "terminal.input");
        assert_eq!(input["bytes"], "aGVsbG8K");

        let resize: Value =
            serde_json::from_str(resize_line(30, 100).expect("resize").trim_end()).unwrap();
        assert_eq!(resize["type"], "terminal.resize");
        assert_eq!(resize["cols"], 100);
        assert_eq!(resize["rows"], 30);
        assert_eq!(resize["cell_width_px"], 0);
        assert_eq!(resize["cell_height_px"], 0);

        let scroll: Value =
            serde_json::from_str(scroll_line(3, Some(24), Some(12), 2).unwrap().trim_end())
                .unwrap();
        assert_eq!(
            scroll,
            json!({
                "type": "terminal.scroll", "direction": "up", "lines": 3,
                "source": "wheel", "column": 24, "row": 12, "modifiers": 2,
            })
        );
        assert!(scroll_line(0, None, None, 0).is_none());

        let release: Value = serde_json::from_str(release_line().trim_end()).unwrap();
        assert_eq!(release, json!({"type": "terminal.release"}));
    }

    #[test]
    fn official_terminal_cli_arguments_never_request_takeover() {
        assert_eq!(
            session_arguments(Mode::Control, "w1:p2", 30, 100),
            [
                "terminal", "session", "control", "w1:p2", "--cols", "100", "--rows", "30"
            ]
        );
        assert_eq!(
            session_arguments(Mode::Observe, "w1:p2", 30, 100),
            [
                "terminal", "session", "observe", "w1:p2", "--cols", "100", "--rows", "30"
            ]
        );
        assert!(
            session_arguments(Mode::Control, "w1:p2", 30, 100)
                .iter()
                .all(|argument| argument != "--takeover")
        );
    }
}
