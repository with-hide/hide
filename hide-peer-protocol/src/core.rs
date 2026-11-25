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
//! the computer someone is at, and, from a machine to a machine it runs
//! work on, the names that work goes by.
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
    /// A phone was paired with the sending computer, told once to each
    /// connected computer that may open it, so a pairing no one there made is
    /// seen (PRD device-connect-iroh D-70). The phone's name is all of it:
    /// nothing of the code, the link or the phone's keys.
    PhonePaired { phone: String },
    /// The names the sending core shows its work on the receiving machine
    /// by, for a machine it may open, whole each time a name moves and
    /// once on each stream (PRD device-connect-iroh B10): each pane by the
    /// id the receiving machine's tap lists it under, with its agent's
    /// label or else its tab's name. A pane left out has no name, and the
    /// receiving machine words its row by its folder.
    ///
    /// `work` is every such pane with what its row and its screen show
    /// there besides (B10): the name when it has one, the branch of the
    /// checkout it stands in and the mark its own rows are drawn with. A
    /// reader that has it reads it in place of `names`; `names` stays for a
    /// build before it, which takes the message without `work` and words a
    /// pane with no name by its folder, with no mark.
    WorkNames {
        names: Vec<WorkName>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        work: Vec<Work>,
    },
}

/// One pane of the sending machine's work on the receiving one and the name
/// its own core shows it by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkName {
    pub pane: String,
    pub name: String,
}

/// One pane of the sending machine's work on the receiving one, as the
/// sending core shows it (B10): its name, else the branch its plain
/// terminal stands on, and the mark its row is drawn with there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub pane: String,
    /// Its agent's label or its tab's given name, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The branch of the checkout it stands in, when that is a repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The mark its row is drawn with on the sending machine (`quiet`,
    /// `going`, `turn`, ...): a word the receiving machine draws when its
    /// build knows it and leaves out when not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    /// The agent that runs in it (`claude`, `codex`, ...), whose logo its
    /// row draws; absent for a plain terminal. A build before it leaves it
    /// out, and the receiving machine draws the row without a logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// The longest branch a [`Work`] carries: git refuses a longer ref name on
/// most systems.
pub const MAX_WORK_BRANCH_BYTES: usize = 255;
/// The longest mark a [`Work`] carries; every mark is one short word.
pub const MAX_WORK_MARK_BYTES: usize = 16;
/// The longest agent kind a [`Work`] carries: an agent definition's id.
pub const MAX_WORK_AGENT_BYTES: usize = 32;

/// Whether `agent` is an agent kind a [`Work`] carries: an agent
/// definition's id, lower-case letters, digits and `-`, not starting with
/// `-`. It is `hide_agent_adapter::definition::id_is_valid`, which this
/// crate cannot link; a test in hided holds the two equal, since a sender
/// that passed a kind this refuses would lose its whole list.
pub fn work_agent_is_carried(agent: &str) -> bool {
    (1..=MAX_WORK_AGENT_BYTES).contains(&agent.len())
        && agent
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !agent.starts_with('-')
}

/// The most panes one [`CoreMessage::WorkNames`] names: the most a host
/// keeps of one guest (`panes.json`).
pub const MAX_WORK_NAMES: usize = 128;
/// The longest name a [`WorkName`] carries, as a told row's title is.
pub const MAX_WORK_NAME_CHARS: usize = 160;
const MAX_WORK_PANE_BYTES: usize = 255;

/// Whether `names` is a list a receiving machine keeps: bounded, no pane
/// twice, every pane id one the tap carries, and every name one line that
/// is not blank.
pub fn valid_work_names(names: &[WorkName]) -> bool {
    let mut panes = std::collections::HashSet::new();
    names.len() <= MAX_WORK_NAMES
        && names.iter().all(|named| {
            !named.pane.is_empty()
                && named.pane.len() <= MAX_WORK_PANE_BYTES
                && panes.insert(named.pane.as_str())
                && !named.name.trim().is_empty()
                && named.name.chars().count() <= MAX_WORK_NAME_CHARS
                && !named.name.chars().any(char::is_control)
        })
}

/// Whether `work` is a list a receiving machine keeps: bounded as
/// [`valid_work_names`] bounds the names, no pane twice, every name one line
/// that is not blank, every branch one line, every mark one lower-case
/// word, and every agent one an agent definition could be named
/// ([`work_agent_is_carried`]).
pub fn valid_work(work: &[Work]) -> bool {
    let mut panes = std::collections::HashSet::new();
    work.len() <= MAX_WORK_NAMES
        && work.iter().all(|told| {
            !told.pane.is_empty()
                && told.pane.len() <= MAX_WORK_PANE_BYTES
                && panes.insert(told.pane.as_str())
                && told.name.as_deref().is_none_or(|name| {
                    !name.trim().is_empty()
                        && name.chars().count() <= MAX_WORK_NAME_CHARS
                        && !name.chars().any(char::is_control)
                })
                && told.branch.as_deref().is_none_or(|branch| {
                    !branch.trim().is_empty()
                        && branch.len() <= MAX_WORK_BRANCH_BYTES
                        && !branch.chars().any(char::is_control)
                })
                && told.mark.as_deref().is_none_or(|mark| {
                    !mark.is_empty()
                        && mark.len() <= MAX_WORK_MARK_BYTES
                        && mark
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                })
                && told.agent.as_deref().is_none_or(work_agent_is_carried)
        })
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
/// The longest phone name a [`CoreMessage::PhonePaired`] carries: a
/// computer names a phone by its kind and a number (`iPhone 2`).
pub const MAX_PHONE_NAME_CHARS: usize = 64;
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

/// Whether `phone` can be shown as a [`CoreMessage::PhonePaired`]'s name:
/// not empty, bounded, and no control character.
pub fn valid_phone_name(phone: &str) -> bool {
    !phone.is_empty()
        && phone.chars().count() <= MAX_PHONE_NAME_CHARS
        && !phone.chars().any(char::is_control)
}

impl CoreMessage {
    pub const TYPES: [&'static str; 12] = [
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
        "phone_paired",
        "work_names",
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

    /// D-70: a new phone crosses as its name alone, which is shown only
    /// while it is short and has no control character.
    #[test]
    fn a_new_phone_is_its_name_and_a_name_past_the_bounds_is_not_shown() {
        let paired = CoreMessage::PhonePaired {
            phone: "iPhone 2".to_owned(),
        };
        let body = serde_json::to_vec(&paired).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"t": "phone_paired", "phone": "iPhone 2"})
        );
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(paired));
        assert!(valid_phone_name("iPhone 2"));
        assert!(valid_phone_name(&"폰".repeat(MAX_PHONE_NAME_CHARS)));
        for name in [
            String::new(),
            "폰".repeat(MAX_PHONE_NAME_CHARS + 1),
            "iPhone\n2".to_owned(),
        ] {
            assert!(!valid_phone_name(&name), "{name:?}");
        }
    }

    /// B10: a machine's work on another goes by the names its core shows
    /// it by, and a list past the bounds is not kept.
    #[test]
    fn work_names_cross_by_pane_and_a_list_past_the_bounds_is_refused() {
        let named = |pane: &str, name: &str| WorkName {
            pane: pane.to_owned(),
            name: name.to_owned(),
        };
        let told = CoreMessage::WorkNames {
            names: vec![named("w1:p1", "로그인 고치기")],
            work: Vec::new(),
        };
        let body = serde_json::to_vec(&told).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            serde_json::json!({"t": "work_names", "names": [{"pane": "w1:p1", "name": "로그인 고치기"}]})
        );
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(told));
        assert!(valid_work_names(&[]));
        assert!(valid_work_names(&[
            named("w1:p1", "a"),
            named("w1:p2", "a")
        ]));
        for wrong in [
            vec![named("w1:p1", "a"), named("w1:p1", "b")],
            vec![named("", "a")],
            vec![named("w1:p1", "  ")],
            vec![named("w1:p1", "a\nb")],
            vec![named("w1:p1", &"가".repeat(MAX_WORK_NAME_CHARS + 1))],
            (0..=MAX_WORK_NAMES)
                .map(|n| named(&format!("w1:p{n}"), "a"))
                .collect(),
        ] {
            assert!(!valid_work_names(&wrong), "{wrong:?}");
        }
    }

    /// B10: each pane's branch and mark cross beside its name, a list past
    /// the bounds is not kept, and the builds on either side of the change
    /// read each other: an earlier build's message is read with no `work`,
    /// and an earlier reader takes a later message by its `names`.
    #[test]
    fn work_crosses_with_its_branch_and_mark_and_reads_across_builds() {
        let work =
            |pane: &str, name: Option<&str>, branch: Option<&str>, mark: Option<&str>| Work {
                pane: pane.to_owned(),
                name: name.map(str::to_owned),
                branch: branch.map(str::to_owned),
                mark: mark.map(str::to_owned),
                agent: None,
            };
        let told = CoreMessage::WorkNames {
            names: vec![WorkName {
                pane: "w1:p1".to_owned(),
                name: "로그인 고치기".to_owned(),
            }],
            work: vec![
                work("w1:p1", Some("로그인 고치기"), Some("main"), Some("going")),
                work("w1:p2", None, Some("m75-relay"), Some("quiet")),
            ],
        };
        let body = serde_json::to_vec(&told).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            serde_json::json!({"t": "work_names",
            "names": [{"pane": "w1:p1", "name": "로그인 고치기"}],
            "work": [
                {"pane": "w1:p1", "name": "로그인 고치기", "branch": "main", "mark": "going"},
                {"pane": "w1:p2", "branch": "m75-relay", "mark": "quiet"},
            ]})
        );
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(told));

        // An earlier build sends `names` alone: read, with no work.
        let earlier = br#"{"t":"work_names","names":[{"pane":"w1:p1","name":"a"}]}"#;
        assert_eq!(
            CoreMessage::parse(earlier).unwrap(),
            Some(CoreMessage::WorkNames {
                names: vec![WorkName {
                    pane: "w1:p1".to_owned(),
                    name: "a".to_owned()
                }],
                work: Vec::new(),
            })
        );
        // An earlier reader, which knew `names` alone, reads a later message.
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(tag = "t", rename_all = "snake_case")]
        enum Earlier {
            WorkNames { names: Vec<WorkName> },
        }
        assert_eq!(
            serde_json::from_slice::<Earlier>(&body).unwrap(),
            Earlier::WorkNames {
                names: vec![WorkName {
                    pane: "w1:p1".to_owned(),
                    name: "로그인 고치기".to_owned()
                }]
            }
        );

        assert!(valid_work(&[]));
        assert!(valid_work(&[
            work("w1:p1", None, None, None),
            work("w1:p2", Some("a"), Some("feat/x"), Some("look")),
        ]));
        for wrong in [
            vec![
                work("w1:p1", None, None, None),
                work("w1:p1", None, None, None),
            ],
            vec![work("", None, None, None)],
            vec![work("w1:p1", Some(" "), None, None)],
            vec![work(
                "w1:p1",
                Some(&"가".repeat(MAX_WORK_NAME_CHARS + 1)),
                None,
                None,
            )],
            vec![work("w1:p1", None, Some("a\nb"), None)],
            vec![work(
                "w1:p1",
                None,
                Some(&"b".repeat(MAX_WORK_BRANCH_BYTES + 1)),
                None,
            )],
            vec![work("w1:p1", None, None, Some("Going"))],
            vec![work("w1:p1", None, None, Some(""))],
            vec![work(
                "w1:p1",
                None,
                None,
                Some(&"a".repeat(MAX_WORK_MARK_BYTES + 1)),
            )],
            (0..=MAX_WORK_NAMES)
                .map(|n| work(&format!("w1:p{n}"), None, None, None))
                .collect(),
        ] {
            assert!(!valid_work(&wrong), "{wrong:?}");
        }
    }

    /// B10: a pane's agent crosses with its work, so the receiving machine
    /// draws its logo; a build before it sends none and reads a later
    /// message without it, and an agent no definition could be named by
    /// keeps the list out.
    #[test]
    fn work_crosses_with_its_agent_and_reads_across_builds() {
        let work = |pane: &str, agent: Option<&str>| Work {
            pane: pane.to_owned(),
            name: None,
            branch: Some("main".to_owned()),
            mark: Some("going".to_owned()),
            agent: agent.map(str::to_owned),
        };
        let told = CoreMessage::WorkNames {
            names: Vec::new(),
            work: vec![work("w1:p1", Some("claude")), work("w1:p2", None)],
        };
        let body = serde_json::to_vec(&told).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["work"],
            serde_json::json!([
                {"pane": "w1:p1", "branch": "main", "mark": "going", "agent": "claude"},
                {"pane": "w1:p2", "branch": "main", "mark": "going"},
            ])
        );
        assert_eq!(CoreMessage::parse(&body).unwrap(), Some(told));

        // An earlier build's work has no agent: read, with none.
        let earlier = br#"{"t":"work_names","names":[],"work":[{"pane":"w1:p1","mark":"quiet"}]}"#;
        let Some(CoreMessage::WorkNames { work: read, .. }) = CoreMessage::parse(earlier).unwrap()
        else {
            panic!("work names were not read");
        };
        assert_eq!(read[0].agent, None);
        // An earlier reader, whose work had no agent, reads a later message.
        #[derive(Debug, Deserialize, PartialEq)]
        struct EarlierWork {
            pane: String,
            mark: Option<String>,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(tag = "t", rename_all = "snake_case")]
        enum Earlier {
            WorkNames { work: Vec<EarlierWork> },
        }
        let Earlier::WorkNames { work: earlier_read } =
            serde_json::from_slice::<Earlier>(&body).unwrap();
        assert_eq!(earlier_read[0].pane, "w1:p1");

        assert!(valid_work(&[
            work("w1:p1", Some("cursor-agent")),
            work("w1:p2", Some("qwen3"))
        ]));
        for agent in [
            "",
            "Claude",
            "claude code",
            "a\nb",
            "-claude",
            "my_agent",
            &"a".repeat(MAX_WORK_AGENT_BYTES + 1),
        ] {
            assert!(!valid_work(&[work("w1:p1", Some(agent))]), "{agent:?}");
        }
    }

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
