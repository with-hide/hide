//! The files the operator's View feedback carries (PRD view-opener-feedback
//! D-16, D-23): screenshots the screen's node stages when a comment is
//! left, and the letter attachments a Send places on the recipient's
//! machine. Only names and paths cross the core; the bytes go from the node
//! that holds them to the node that keeps them, read and written off the
//! core's lock by a worker, as terminal attachments are.

use base64::Engine;
use serde::{Deserialize, Serialize};

/// Screenshots a machine keeps staged at once, and their bytes.
pub const MAX_STAGED_SHOTS: usize = 2_048;
pub const MAX_STAGED_SHOT_BYTES: u64 = 256 * 1024 * 1024;
/// The largest screenshot or feedback file, in bytes.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// What one call carries at most, so a request line stays far below the
/// node's 40 MiB line cap once base64 inflates it.
pub const MAX_CALL_BYTES: u64 = 24 * 1024 * 1024;
/// Letter attachments one machine keeps at once (D-16: the terminal
/// attachments' cap, `crate::attachments::MAX_STAGED_BYTES`).
pub const MAX_PLACED_BYTES: u64 = crate::attachments::MAX_STAGED_BYTES;
/// Files one letter carries at most: a screenshot per comment of the
/// fullest View, and the feedback file.
pub const MAX_LETTER_FILES: usize = 64;
/// Letters whose attachments one sweep keeps.
pub const MAX_KEPT_LETTERS: usize = 8_192;

/// Whether `name` is a staged screenshot's name: `shot-<uuid>.png`.
pub fn valid_shot_name(name: &str) -> bool {
    name.strip_prefix("shot-")
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(crate::attachments::valid_request_id)
}

/// Whether `name` may be a file in a letter's folder: a screenshot's name,
/// or the one feedback file.
pub fn valid_letter_file_name(name: &str) -> bool {
    valid_shot_name(name) || name == FEEDBACK_FILE
}

/// The file holding feedback past a letter's body cap (D-15, B26).
pub const FEEDBACK_FILE: &str = "feedback.md";

/// Whether `intent` names a letter's folder: the intent its Send gave it,
/// `view-feedback-<uuid>`.
pub fn valid_letter_folder(intent: &str) -> bool {
    intent
        .strip_prefix(LETTER_PREFIX)
        .is_some_and(crate::attachments::valid_request_id)
}

pub const LETTER_PREFIX: &str = "view-feedback-";

/// One file and its bytes, base64 on the wire.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct File {
    pub name: String,
    data: String,
}

impl File {
    pub fn new(name: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|_| format!("{} is not valid base64", self.name))
    }

    /// The bytes this file holds, without decoding it: the padding that
    /// ends the encoding carries none.
    pub fn len(&self) -> u64 {
        let padding = self
            .data
            .bytes()
            .rev()
            .take(2)
            .take_while(|&byte| byte == b'=')
            .count() as u64;
        (self.data.len() as u64 / 4 * 3).saturating_sub(padding)
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// What a node does for View feedback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackCall {
    /// Reads staged screenshots, answered as `Vec<File>` in order; a name
    /// that is not staged is an error.
    ReadShots { names: Vec<String> },
    /// Removes staged screenshots; one already gone is not an error.
    RemoveShots { names: Vec<String> },
    /// Removes every staged screenshot `keep` does not name.
    SweepShots { keep: Vec<String> },
    /// Places a letter's files in its own folder, answered as
    /// [`Placed`]. A file already there with the same bytes is kept, so a
    /// retried Send places nothing twice; past [`MAX_PLACED_BYTES`] the
    /// whole call is refused and nothing is written.
    Place { letter: String, files: Vec<File> },
    /// Removes a letter's folder; one already gone is not an error.
    Remove { letter: String },
    /// Removes every letter folder `keep` does not name.
    Sweep { keep: Vec<String> },
}

/// Where a letter's files were placed, in the wire spelling of the
/// recipient's machine (`hide_platform::path`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub paths: Vec<String>,
}

/// Why a letter's files could not be placed, which the screen says.
pub const PLACE_FULL: &str = "attachments_full";
/// A screenshot its machine no longer holds: the comment goes without it.
pub const SHOT_GONE: &str = "shot_gone";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_generated_names_reach_a_folder() {
        let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert!(valid_shot_name(&format!("shot-{id}.png")));
        assert!(!valid_shot_name(&format!("shot-{id}.png/../x")));
        assert!(!valid_shot_name("shot-x.png"));
        assert!(valid_letter_file_name(FEEDBACK_FILE));
        assert!(valid_letter_folder(&format!("{LETTER_PREFIX}{id}")));
        assert!(!valid_letter_folder("view-feedback-../../etc"));
    }

    #[test]
    fn a_file_carries_its_bytes_unchanged() {
        let file = File::new("feedback.md", b"\x00\x01hello");
        assert_eq!(file.bytes().unwrap(), b"\x00\x01hello");
    }

    #[test]
    fn a_file_s_length_is_its_bytes_whatever_its_padding() {
        for size in 0..=7 {
            assert_eq!(File::new("a", &vec![7; size]).len(), size as u64);
        }
    }
}
