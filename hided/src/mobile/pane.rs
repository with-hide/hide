//! A phone reads one agent pane through the engine that runs it (PRD D-13).
//! A reply or key goes through the core's one operator write
//! (`hide_core::operator_input`), the desktop conversation's too: the pane
//! node's input permit, the input gate, then the engine; success means the
//! engine answered the write. Neither changes the core's attach set, focus,
//! terminal size or visible tab.

use std::time::Duration;

use hide_core::engine::{EngineError, EngineHandle};
use hide_core::operator_input::Unwritten;

const TIMEOUT: Duration = Duration::from_secs(5);

/// The rows a detail asks for first, how many a pull adds, and the most it may hold.
pub const FIRST_LINES: u32 = 200;
pub const MORE_LINES: u32 = 200;
pub const MAX_LINES: u32 = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PaneError {
    /// The pane is closed or no longer exists.
    Gone,
    /// The SSH device that owns it is not connected.
    DeviceUnreachable,
    /// This Mac's engine did not answer.
    Unavailable(String),
}

impl PaneError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::DeviceUnreachable => "device_unreachable",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

/// The reason a phone is told for a write that did not land, in the phone's
/// words (`gone` where the desktop says `pane_gone`).
pub fn unwritten_reason(unwritten: Unwritten) -> &'static str {
    match unwritten {
        Unwritten::Gone => PaneError::Gone.reason(),
        other => other.reason(),
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
    pub alternate_screen: bool,
}

pub fn read(engine: &EngineHandle, pane_id: &str, lines: u32) -> Result<Rows, PaneError> {
    let read = engine
        .read_rows(pane_id, lines.min(MAX_LINES), TIMEOUT)
        .map_err(classify)?;
    Ok(Rows {
        text: read.text,
        truncated: read.truncated,
        alternate_screen: read.alternate_screen,
    })
}

/// The keys a phone may send, as key-combo strings.
pub fn engine_key(key: &str) -> Option<&'static str> {
    match key {
        "enter" => Some("enter"),
        "escape" => Some("esc"),
        "up" => Some("up"),
        "down" => Some("down"),
        "ctrl_c" => Some("ctrl+c"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_quick_keys_map_to_engine_keys() {
        assert_eq!(engine_key("escape"), Some("esc"));
        assert_eq!(engine_key("ctrl_c"), Some("ctrl+c"));
        assert_eq!(engine_key("ctrl_d"), None);
        assert_eq!(engine_key("prefix+c"), None);
    }
}
