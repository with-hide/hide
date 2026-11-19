//! What this core does with an event of its node on another machine (PRD
//! device-connect-iroh D-54, D-66, B12).
//!
//! The node keeps what a pane sent while this core was away and sends it
//! once this core names its place ([`crate::peer_dial`]); each event is
//! recorded durably and acknowledged before it comes here
//! (`hide_peer::inbox`). An event is a letter a pane's agent sent: it is
//! run as the command that agent would have run had this core been there,
//! and the letter's intent makes a second run the same letter.
//!
//! The node proved the terminal the letter was written in, and which agent
//! that is, is this core's knowledge (`Runtime::prepare_kept_letter`): the
//! letter is applied as that agent whether or not its pane still runs. An
//! event is given up only when what it names is gone: its recipient, or a
//! sender this core never knew.
//!
//! A pane's end and an agent's completion are not events of this build:
//! they reach the core as the node's state when its link is read again,
//! which is already the same however often it is read.

use std::sync::Arc;
use std::time::Duration;

use hide_peer::inbox::Applied;
use hide_peer::protocol::event::Event;
use serde::Deserialize;

use crate::core::CoreHandle;

/// How long a recorded event waits for what it needs of the core: the
/// host's panes read again, for a letter of an agent the core registered
/// nowhere, or the core's letters, while they cannot be written.
pub const PANES_WITHIN: Duration = Duration::from_secs(60);
/// How often the core is asked again meanwhile.
pub const LOOK_AGAIN: Duration = Duration::from_millis(250);
/// How long the core has to take one letter.
const LETTER_WITHIN: Duration = Duration::from_secs(5);

/// What takes the events of this core's node on the machine `device`.
pub trait Events: Send + Sync {
    /// Applies one event. An event the core cannot apply yet is answered
    /// [`Applied::Later`] and tried again, until `last` says its wait is
    /// over.
    fn apply(&self, device: &str, event: &Event, last: bool) -> Applied;
}

/// A letter as the node kept it (`crate::guest_node::away`).
#[derive(Deserialize)]
struct Letter {
    /// The pane that sent it, as the node names it.
    pane: String,
    /// The terminal the node proved its sender ran in.
    terminal: String,
    command: hide_core::delivery::Command,
}

/// Whether the core may answer otherwise when asked again: it does not
/// know the sender yet, or cannot write a letter now.
fn passes(code: &str) -> bool {
    matches!(
        code,
        "sender_unknown"
            | "delivery_unavailable"
            | "ledger_unavailable"
            | "capacity"
            | "delivery_timeout"
    )
}

pub struct Letters {
    core: Arc<CoreHandle>,
}

impl Letters {
    pub fn new(core: Arc<CoreHandle>) -> Self {
        Self { core }
    }

    /// Runs `letter` as its sender's own command, or says why not.
    fn send(&self, device: &str, letter: &Letter) -> Result<(), String> {
        self.core
            .prepare_kept_letter(
                device,
                &letter.pane,
                &letter.terminal,
                letter.command.clone(),
            )?
            .run(LETTER_WITHIN)
            .map(drop)
    }
}

impl Events for Letters {
    fn apply(&self, device: &str, event: &Event, last: bool) -> Applied {
        if event.kind != "letter" {
            return Applied::Unappliable(format!("this build applies no {} event", event.kind));
        }
        let letter: Letter = match serde_json::from_value(event.body.clone()) {
            Ok(letter) => letter,
            Err(error) => return Applied::Unappliable(format!("unreadable letter: {error}")),
        };
        match self.send(device, &letter) {
            Ok(()) => Applied::Done,
            Err(code) if passes(&code) && !last => Applied::Later,
            Err(code) => Applied::Unappliable(code),
        }
    }
}
