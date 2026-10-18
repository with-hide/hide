//! A phone reads and writes one agent pane straight through Herdr's socket
//! API (PRD D-13): `pane.read` for the recent rows, `pane.send_input` for a
//! reply or one key. Neither goes through the core's attach set or
//! `Event::Key`, so the desktop's visible tab, focus, terminal size and
//! attachments stay where they are. Parameters and the answer come from
//! `herdr_core::wire`, which holds the generated types of the pinned Herdr
//! schema.

use std::sync::Arc;
use std::time::Duration;

use herdr_core::wire;
use hide_herdr_client::ApiConnector;
use hide_herdr_client::ApiError;

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
}

impl PaneError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Gone => "gone",
            Self::DeviceUnreachable => "device_unreachable",
            Self::Unavailable(_) => "unavailable",
            Self::Asleep => "asleep",
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
    connector: &Arc<dyn ApiConnector>,
    pane_id: &str,
    input: Input<'_>,
) -> Result<(), PaneError> {
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
}
