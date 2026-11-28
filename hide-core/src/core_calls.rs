//! What two cores say to each other about their agents (PRD
//! device-connect-iroh D-71, D-72): a letter, its answer, and a read or a
//! start one asks of the other.
//!
//! These are the core's own types. The daemon carries them on the
//! connection between the two machines (`hide_peer_protocol::core`) and
//! hands what arrives back in; the core decides everything about them
//! (`runtime::core_calls`, `access::core_admit`). A machine is named here
//! by its connection id, which is what the daemon knows a connection by;
//! the core reads the machine's device id and standing from the paired
//! machines it was told (`crate::peer_machines`).

use std::time::Duration;

use serde_json::Value;

pub use crate::address::CoreItem;
use crate::delivery::worker::{Client, Effect, Prepared};

/// Who asks a read or a start of another core, as its own core names it:
/// an agent by its registration while it has one, its pane otherwise, and
/// the name it goes by there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    pub item: CoreItem,
    pub id: String,
    pub name: String,
}

/// One agent of a core by its registration there, and the name it goes by.
/// A letter is between agents, never panes (PRD letters-missions-simplify
/// D-21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub id: String,
    pub name: String,
}

impl From<Agent> for Party {
    fn from(agent: Agent) -> Self {
        Self {
            item: CoreItem::Agent,
            id: agent.id,
            name: agent.name,
        }
    }
}

/// A letter on its way from one core to another. `id` is the sending
/// core's own id for it: with the sending machine it names the letter on
/// the connection, and its answer names it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Letter {
    pub id: String,
    pub from: Agent,
    /// The recipient's registration on the receiving core, by its id or
    /// the name it registered under. Unread for a reply.
    pub to: String,
    /// For a reply, the id the receiving core gave the letter it answers:
    /// that letter went the other way, so the id is the receiver's own.
    pub reply_to: Option<String>,
    pub kind: String,
    pub intent: String,
    pub body: String,
    /// Sent with `--now`.
    pub now: bool,
}

/// What the receiving core made of a letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It keeps the letter for `recipient`, the agent as it names it.
    Taken { recipient: Agent },
    /// It keeps nothing of it, and why: the code a local send is refused
    /// with, or `no_recipient` when the agent it was for ended with no
    /// agent after it.
    Refused { reason: String },
}

/// A read, a typed send or a start an agent of one core asks of another.
#[derive(Clone, Debug)]
pub enum Asked {
    /// `hide agent list`, `hide agent show` or a `hide agent spawn` with no
    /// parent.
    Delivery(Box<crate::delivery::Command>),
    /// `hide pane read`, `hide agent read` or `hide pane send`.
    Pane(crate::pane_command::PaneCommand),
}

/// The connection to the other cores, as the core uses it. The daemon
/// hands one in at its start (`Core::set_core_link`); a core without one
/// reaches no other core.
pub trait CoreLink: Send + Sync {
    /// Asks the core of machine `key` for `asked` on behalf of `from`, and
    /// waits for its answer at most `within`: what that core answered, or
    /// the refusal's code. Called off the core's lock.
    fn call(
        &self,
        key: &str,
        from: &Party,
        asked: &Asked,
        within: Duration,
    ) -> Result<Value, String>;

    /// The letters that wait to go to another core changed. Called under
    /// the core's lock: it only signals.
    fn outbox_moved(&self);
}

/// A failure of this core's own letter store, which says nothing about the
/// letter: the sending core keeps it and sends it again.
pub(crate) fn passes(code: &str) -> bool {
    matches!(
        code,
        "delivery_unavailable" | "ledger_unavailable" | "delivery_timeout"
    )
}

/// The end a letter takes when no agent is there to take it.
pub(crate) const NO_RECIPIENT: &str = "no_recipient";

/// What the receiving core answers a letter with: `prepared` is what
/// `Core::prepare_core_letter` made of it. An error is this core's own
/// store failing, which says nothing of the letter, so nothing is answered
/// and the sending core asks again; every other refusal is the letter's
/// outcome, which ends it for its sender.
pub fn take(prepared: Result<Taking, String>, within: Duration) -> Result<Outcome, String> {
    match prepared {
        Ok(taking) => taking.run(within),
        Err(code) if passes(&code) => Err(code),
        Err(reason) => Ok(Outcome::Refused { reason }),
    }
}

/// A letter of another core, admitted: either this core already keeps it,
/// and answers as it stands, or it is ready to be kept.
pub enum Taking {
    Kept(Outcome),
    Keep(Box<Keeping>),
}

/// A letter of another core ready to be kept.
pub struct Keeping {
    pub(crate) prepared: Prepared,
    pub(crate) client: Client,
    /// The sending core's id for the letter.
    pub(crate) core_id: String,
    pub(crate) recipient: Agent,
}

impl Taking {
    /// Keeps the letter, off the core's lock. An error is this core's own
    /// store failing, after which the letter is asked again; every other
    /// refusal is the letter's outcome.
    pub fn run(self, within: Duration) -> Result<Outcome, String> {
        let keeping = match self {
            Self::Kept(outcome) => return Ok(outcome),
            Self::Keep(keeping) => *keeping,
        };
        let Keeping {
            prepared,
            client,
            core_id,
            recipient,
        } = keeping;
        let kept = prepared.run(within).and_then(|letter| {
            let id = letter["id"]
                .as_str()
                .ok_or("letter_unavailable")?
                .to_owned();
            // The same letter asked again names the same id, so a name
            // lost between the two is written by the next ask.
            client.submit(Effect::CoreName { id, core_id }, within)?;
            Ok(letter["state"] == NO_RECIPIENT)
        });
        match kept {
            // Its agent ended with no agent after it, or is closed: the
            // store kept the letter only to end it.
            Ok(true) => Ok(Outcome::Refused {
                reason: NO_RECIPIENT.into(),
            }),
            Ok(false) => Ok(Outcome::Taken { recipient }),
            Err(code) if passes(&code) => Err(code),
            Err(reason) => Ok(Outcome::Refused { reason }),
        }
    }
}

/// What another core answered for a letter of this one, or what this core
/// found of a letter it cannot send, ready to be written.
pub struct Settle {
    pub(crate) client: Client,
    pub(crate) effect: Effect,
}

impl Settle {
    /// Writes it, off the core's lock.
    pub fn run(self, within: Duration) -> Result<(), String> {
        self.client.submit(self.effect, within).map(drop)
    }
}

/// The letters that wait to go to one machine's core, and the ends of
/// those this core cannot send (`Runtime::core_outbox`).
#[derive(Default)]
pub struct Outbox {
    pub letters: Vec<Letter>,
    pub(crate) ended: Vec<Settle>,
}

impl Outbox {
    /// Writes the ends, off the core's lock, and gives the letters to send.
    /// An end the store did not take is found again at the next ask.
    pub fn settled(self, within: Duration) -> Vec<Letter> {
        for end in self.ended {
            let _ = end.run(within);
        }
        self.letters
    }
}

/// A read or a start another core asked for, admitted and ready to run.
pub enum Call {
    Delivery(Box<Prepared>),
    Pane(Box<crate::pane_command::Prepared>),
}

impl Call {
    /// Runs it, off the core's lock: its answer, or the refusal's code.
    pub fn run(self, within: Duration) -> Result<Value, String> {
        match self {
            Self::Delivery(prepared) => prepared.run(within),
            Self::Pane(prepared) => prepared.run(),
        }
    }
}

/// A command of this core's agent that another core runs (`Prepared::run`,
/// `pane_command::Prepared::run`).
pub(crate) struct Forward {
    pub(crate) link: std::sync::Arc<dyn CoreLink>,
    /// The other machine's connection id.
    pub(crate) key: String,
    pub(crate) from: Party,
    pub(crate) asked: Asked,
}

impl Forward {
    /// The same ask, by `from`.
    pub(crate) fn asked_by(mut self, from: Party) -> Self {
        self.from = from;
        self
    }

    pub(crate) fn run(&self, within: Duration) -> Result<Value, String> {
        self.link.call(&self.key, &self.from, &self.asked, within)
    }
}
