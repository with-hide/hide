//! What two cores say to each other on the `core` stream (PRD D-62, D-72).
//!
//! Each machine of a pair has a core of its own, and this is the one
//! stream the two cores talk on, apart from a guest's node and its
//! management: a side that holds an active grant on the other opens it and
//! both write. A letter one core's agent writes to an agent of the other,
//! the answer to it, and a read or a start one asks of the other go here.
//!
//! The list grows by kind, as the management messages do: a later message
//! names the capability it came with in `contracts/peer-link.json`, and a
//! reader skips a type it does not know. What a core does with a message
//! is its own to decide; the daemons carry it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::frame::{self, FrameError};

/// Whether an agent is named by its registration or by its pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    Agent,
    Pane,
}

/// Who asks a [`CoreMessage::Call`], as its own core names it: an agent by
/// its registration there, or one that has none by its pane, and the name it
/// goes by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    pub item: Item,
    pub id: String,
    pub name: String,
}

/// One agent of a core by its registration there, and the name it goes by:
/// a letter is between agents, never panes (PRD letters-missions-simplify
/// D-21).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
}

/// What a [`CoreMessage::Call`] asks for: an agent command (list, show, a
/// start with no parent) or a pane command (a read), as the asking core's
/// CLI contract spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ask {
    Delivery,
    Pane,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum CoreMessage {
    /// A letter an agent of the sending core wrote to an agent of the
    /// receiving one. `id` is the sending core's own id for it: with the
    /// machine it came from it names the letter, and the answer names it
    /// back. The same letter may come again until it is answered.
    Letter {
        id: String,
        from: Agent,
        /// The recipient's registration on the receiving core, by its id
        /// or the name it registered under; a pane is not a recipient.
        to: String,
        /// For a reply, the id the receiving core gave the letter it
        /// answers, which went the other way.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        kind: String,
        intent: String,
        body: String,
        #[serde(default)]
        now: bool,
    },
    /// The receiving core keeps letter `id` for `recipient`.
    LetterTaken { id: String, recipient: Agent },
    /// The receiving core keeps nothing of letter `id`, and why: the
    /// letter ends with no recipient for its sender.
    LetterRefused { id: String, reason: String },
    /// A read or a start `from` asks the receiving core for. `call` names
    /// the answer.
    Call {
        call: u64,
        from: Party,
        ask: Ask,
        command: Value,
    },
    /// The answer to `call`: what the command answered, or the code it was
    /// refused with.
    CallAnswer {
        call: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refused: Option<String>,
    },
}

impl CoreMessage {
    pub const TYPES: [&'static str; 5] = [
        "letter",
        "letter_taken",
        "letter_refused",
        "call",
        "call_answer",
    ];

    /// `None` for a type this build does not know.
    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_of_a_later_build_is_skipped_and_a_known_one_is_read() {
        let later = serde_json::to_vec(&serde_json::json!({"t": "in_use", "busy": true})).unwrap();
        assert_eq!(CoreMessage::parse(&later).unwrap(), None);
        let taken = CoreMessage::LetterTaken {
            id: "letter-4".into(),
            recipient: Agent {
                id: "agent-2".into(),
                name: "worker".into(),
            },
        };
        let body = serde_json::to_vec(&taken).unwrap();
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(taken));
    }
}
