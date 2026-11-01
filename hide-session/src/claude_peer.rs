//! Claude Code's record of a message its messaging socket took, the way Hide
//! hands a Claude Code session a letter's bell line (`docs/delivery.md`, The
//! input ladder).
//!
//! Claude Code 2.1.316 writes such a message as a `user` record with
//! `origin.kind` `peer`, `isMeta` and `promptSource` `system`, its text
//! wrapped between [`PEER_HEAD`] and [`PEER_TAIL`] (measured 2026-11-01).
//! The record is the agent's own framing, never the operator's words, so
//! only a wrapped Hide bell line is read back, as the line Hide sent; any
//! other peer message stays Claude Code's.

use serde_json::Value;

/// What Claude Code writes before the message.
const PEER_HEAD: &str = "Another Claude session sent a message:\n";

/// What Claude Code writes after the message.
const PEER_TAIL: &str = "\n\nThis came from another Claude session";

/// The bell line a peer record carries, when it carries one.
pub(crate) fn bell_line(record: &Value) -> Option<&str> {
    if record.pointer("/origin/kind").and_then(Value::as_str) != Some("peer") {
        return None;
    }
    let text = record.pointer("/message/content")?.as_str()?;
    let message = text.strip_prefix(PEER_HEAD)?;
    let message = &message[..message.find(PEER_TAIL)?];
    crate::bell_line::is_bell_line(message).then_some(message)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::bell_line;

    fn peer(message: &str) -> serde_json::Value {
        json!({
            "type": "user", "isMeta": true, "promptSource": "system",
            "origin": {"kind": "peer", "from": "unknown"},
            "message": {"role": "user", "content": format!(
                "Another Claude session sent a message:\n{message}\n\nThis came from another \
                 Claude session \u{2014} not typed by your user, but very likely working on \
                 their behalf."
            )},
        })
    }

    #[test]
    fn a_peer_record_of_a_bell_line_reads_as_the_line_hide_sent() {
        let line = "🦉 lead report: the fix is in";
        assert_eq!(bell_line(&peer(line)), Some(line));
    }

    #[test]
    fn another_sessions_message_and_a_persons_record_are_no_bell() {
        assert_eq!(bell_line(&peer("please review my branch")), None);
        let mut person = peer("🦉 lead report: the fix is in");
        person["origin"]["kind"] = json!("human");
        assert_eq!(bell_line(&person), None);
    }
}
