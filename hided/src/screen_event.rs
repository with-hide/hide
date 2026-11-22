//! A screen's text frame, read once for the routing a node's daemon or a
//! core's screen relay does before the core reads it: its `kind`, read
//! without building the rest of the frame, and the whole event only for a
//! kind taken there. Every other frame goes on as it was written.

use serde::Deserialize;
use serde_json::Value;

/// The kinds a node's daemon or a core's relay takes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Key,
    FileBytes,
    AttachmentStage,
    AttachmentCancel,
    AttachmentCommit,
    TerminalInput,
    TerminalAttachment,
    CoreLink,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Head {
    kind: Kind,
}

/// A frame of one of the kinds taken here, read whole.
pub struct Routed {
    pub kind: Kind,
    pub event: Value,
}

/// The frame as an event of one of `taken`, or `None` for any other frame,
/// which is not built.
pub fn read(text: &str, taken: &[Kind]) -> Option<Routed> {
    let kind = serde_json::from_str::<Head>(text).ok()?.kind;
    if !taken.contains(&kind) {
        return None;
    }
    let event = serde_json::from_str(text).ok()?;
    Some(Routed { kind, event })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame is taken by its `kind` wherever the key stands; any other
    /// kind, a kind not taken here, or a frame that is not an event, is not.
    #[test]
    fn a_frame_is_taken_by_its_kind_wherever_the_key_stands() {
        let taken = [Kind::Key, Kind::FileBytes];
        let routed = read(r#"{"payload":{"kind":"x"},"kind":"key"}"#, &taken).unwrap();
        assert_eq!(routed.kind, Kind::Key);
        assert_eq!(routed.event["payload"]["kind"], "x");
        assert!(read(r#"{"kind":"terminal_input","payload":{}}"#, &taken).is_none());
        assert!(read(r#"{"kind":"select_pane","payload":{}}"#, &taken).is_none());
        assert!(read(r#"{"type":"ping"}"#, &taken).is_none());
        assert!(read("not json", &taken).is_none());
    }
}
