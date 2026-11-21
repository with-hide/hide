//! What a pane's session says to the terminal service, whatever transport
//! carries it: the frames it draws and the reason it ended.

pub use hide_node_link::terminal::{decode_base64, encode_base64};

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
    /// The text the pane's program wrote to the clipboard (OSC 52), once:
    /// no part of the screen, so nothing that draws the screen again
    /// repeats it.
    Clipboard { text: Vec<u8> },
    /// A session that closed with the reason its transport names, never
    /// read from words.
    Ended {
        category: &'static str,
        message: String,
    },
}
