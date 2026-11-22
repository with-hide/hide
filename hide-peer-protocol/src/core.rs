//! What two cores say to each other on the `core` stream (PRD D-62, D-72).
//!
//! Each machine of a pair has a core of its own, and this is the one
//! stream the two cores talk on, apart from a guest's node and its
//! management: a side that holds an active grant on the other opens it and
//! both write. A letter one core's agent writes to an agent of the other,
//! the answer to it, and a read or a start one asks of the other go here,
//! and so does what the two machines show of each other's work (D-50,
//! D-60): a machine's own work that waits on a person, for the machines
//! that may open it, each computer's seat, and a notification handed to
//! the computer someone is at.
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
    /// The sending machine's own work that waits on a person or is done,
    /// whole each time, for a machine that may open it (D-50): two counts
    /// and its top rows as its core made them. The receiving core checks
    /// every bound, keeps it only for a machine it opens, and forgets it
    /// with the connection.
    Work { work: Value },
    /// The sending computer's seat, for a machine it may open (D-60,
    /// D-78), sent when it changes and never per input. Every value is a
    /// length of time, never a clock reading: `input_ms` is how long ago
    /// its last input was while someone uses it and absent while no one
    /// does, `shows` whether a window there shows notifications, and
    /// `phone_ms` how long ago a phone paired with it last acted.
    Seat {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_ms: Option<u64>,
        shows: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        phone_ms: Option<u64>,
    },
    /// One notification of the sending core's work, for the receiving
    /// computer to show because someone is at it (D-60). `id` names the
    /// answer.
    Notice { id: u64, notice: Notice },
    /// Whether the receiving computer showed notice `id`. One that no one
    /// uses any more did not, and the sender goes on to its next place, as
    /// it does when no answer comes (B71).
    NoticeAnswer { id: u64, taken: bool },
    /// The notifications under `tags` are over: the receiving computer
    /// closes the ones it showed.
    NoticeClear { tags: Vec<String> },
}

/// One notification as the core that judged it worded it: data only, which
/// the computer that shows it words in its own language.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// The work's machine and pane as its core names them. Together they
    /// are the notification's tag ([`Notice::tag`]): a later one under
    /// the same tag replaces it, and a clear names it.
    pub device_id: String,
    pub pane_id: String,
    pub title: String,
    /// `needs_you` or `done`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// What the core asks on the row, as its snapshot spells it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Value>,
    pub place: String,
}

/// The most tags one [`CoreMessage::NoticeClear`] names.
pub const MAX_CLEAR_TAGS: usize = 64;
const MAX_TAG_BYTES: usize = 600;
const MAX_TITLE_CHARS: usize = 200;
const MAX_DETAIL_CHARS: usize = 400;
const MAX_SUBJECT_BYTES: usize = 2048;

impl Notice {
    pub fn tag(&self) -> String {
        format!("{}|{}", self.device_id, self.pane_id)
    }

    /// The same notice with its texts cut to what [`Self::valid`] takes.
    pub fn clipped(mut self) -> Self {
        let cut = |text: &mut String, chars: usize| {
            if let Some((at, _)) = text.char_indices().nth(chars) {
                text.truncate(at);
            }
        };
        cut(&mut self.title, MAX_TITLE_CHARS);
        cut(&mut self.place, MAX_TITLE_CHARS);
        if let Some(detail) = &mut self.detail {
            cut(detail, MAX_DETAIL_CHARS);
        }
        self
    }

    /// Whether every part is within what a computer shows: a notice from
    /// another machine is drawn in this one's notification center, so its
    /// sizes are checked before anything of it is.
    pub fn valid(&self) -> bool {
        let short = |text: &str, chars: usize| text.chars().count() <= chars;
        valid_tag(&self.device_id)
            && valid_tag(&self.pane_id)
            && short(&self.title, MAX_TITLE_CHARS)
            && matches!(self.state.as_str(), "needs_you" | "done")
            && self
                .status_code
                .as_deref()
                .is_none_or(|code| short(code, 64))
            && self
                .detail
                .as_deref()
                .is_none_or(|detail| short(detail, MAX_DETAIL_CHARS))
            && self.subject.as_ref().is_none_or(|subject| {
                serde_json::to_vec(subject).is_ok_and(|bytes| bytes.len() <= MAX_SUBJECT_BYTES)
            })
            && short(&self.place, MAX_TITLE_CHARS)
    }
}

/// Whether `tag` can name a notification or a part of one: not empty,
/// bounded, and no control character.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= MAX_TAG_BYTES && !tag.chars().any(char::is_control)
}

impl CoreMessage {
    pub const TYPES: [&'static str; 10] = [
        "letter",
        "letter_taken",
        "letter_refused",
        "call",
        "call_answer",
        "work",
        "seat",
        "notice",
        "notice_answer",
        "notice_clear",
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

    /// D-78: a seat carries lengths of time and a computer no one uses says
    /// so by leaving its input out; a notice is checked before it is shown.
    #[test]
    fn a_seat_is_lengths_of_time_and_a_notice_is_bounded() {
        let away = CoreMessage::Seat {
            input_ms: None,
            shows: true,
            phone_ms: Some(4_000),
        };
        let body = serde_json::to_vec(&away).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            serde_json::json!({"t": "seat", "shows": true, "phone_ms": 4_000})
        );
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(away));

        let notice = Notice {
            device_id: "mac".into(),
            pane_id: "w1:p1".into(),
            title: "Fix login".into(),
            state: "needs_you".into(),
            status_code: Some("question".into()),
            detail: Some("Which branch?".into()),
            subject: None,
            place: "shop-api".into(),
        };
        assert!(notice.valid());
        assert_eq!(notice.tag(), "mac|w1:p1");
        let wrong = |change: fn(&mut Notice)| {
            let mut notice = notice.clone();
            change(&mut notice);
            notice
        };
        for wrong in [
            wrong(|notice| notice.device_id.clear()),
            wrong(|notice| notice.pane_id = "a\nb".into()),
            wrong(|notice| notice.state = "working".into()),
            wrong(|notice| notice.title = "x".repeat(MAX_TITLE_CHARS + 1)),
            wrong(|notice| notice.detail = Some("x".repeat(MAX_DETAIL_CHARS + 1))),
        ] {
            assert!(!wrong.valid(), "{wrong:?}");
        }
        let long = wrong(|notice| notice.title = "가".repeat(MAX_TITLE_CHARS + 1));
        assert!(long.clipped().valid());
    }
}
