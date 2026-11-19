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

/// One agent of a core, as that core names it to another: its registration
/// while it has one, its pane otherwise, and the name it goes by there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Party {
    pub item: CoreItem,
    pub id: String,
    pub name: String,
}

/// A letter on its way from one core to another. `id` is the sending
/// core's own id for it: with the sending machine it names the letter on
/// the connection, and its answer names it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Letter {
    pub id: String,
    pub from: Party,
    /// The recipient as the receiving core knows it: an agent's id or name,
    /// or a pane. Unread for a reply.
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
    Taken { recipient: Party },
    /// It keeps nothing of it, and why: the code a local send is refused
    /// with.
    Refused { reason: String },
}

/// A read or a start an agent of one core asks of another.
#[derive(Clone, Debug)]
pub enum Asked {
    /// `hide agent list`, `hide agent show` or a `hide agent spawn` with no
    /// parent.
    Delivery(Box<crate::delivery::Command>),
    /// `hide pane read` or `hide agent read`.
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
fn passes(code: &str) -> bool {
    matches!(
        code,
        "delivery_unavailable" | "ledger_unavailable" | "delivery_timeout"
    )
}

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

/// A letter of another core, admitted and ready to be kept.
pub struct Taking {
    pub(crate) prepared: Prepared,
    pub(crate) client: Client,
    /// The sending core's id for the letter.
    pub(crate) core_id: String,
    pub(crate) recipient: Party,
}

impl Taking {
    /// Keeps the letter, off the core's lock. An error is this core's own
    /// store failing, after which the letter is asked again; every other
    /// refusal is the letter's outcome.
    pub fn run(self, within: Duration) -> Result<Outcome, String> {
        let kept = self.prepared.run(within).and_then(|letter| {
            let id = letter["id"]
                .as_str()
                .ok_or("letter_unavailable")?
                .to_owned();
            // The same letter asked again names the same id, so a name
            // lost between the two is written by the next ask.
            self.client.submit(
                Effect::CoreName {
                    id,
                    core_id: self.core_id,
                },
                within,
            )
        });
        match kept {
            Ok(_) => Ok(Outcome::Taken {
                recipient: self.recipient,
            }),
            Err(code) if passes(&code) => Err(code),
            Err(reason) => Ok(Outcome::Refused { reason }),
        }
    }
}

/// What another core answered for a letter of this one, ready to be
/// written.
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
    pub(crate) fn run(&self, within: Duration) -> Result<Value, String> {
        self.link.call(&self.key, &self.from, &self.asked, within)
    }
}
