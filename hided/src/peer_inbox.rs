//! What this core does with an event of its node on another machine (PRD
//! device-connect-iroh D-54, D-66, B12).
//!
//! The node keeps what a pane sent while this core was away and sends it
//! once this core names its place ([`crate::peer_dial`]); each event is
//! recorded durably and acknowledged before it comes here
//! (`hide_peer::inbox`). An event is a letter a pane's agent sent: it is
//! run as the command that pane would have run had this core been there,
//! and the letter's intent makes a second run the same letter.
//!
//! A pane's end and an agent's completion are not events of this build:
//! they reach the core as the node's state when its link is read again,
//! which is already the same however often it is read.

use std::sync::Arc;
use std::time::Duration;

use hide_core::address;
use hide_core::workspace_control::Query;
use hide_peer::inbox::Applied;
use hide_peer::protocol::event::Event;
use serde::Deserialize;

use crate::core::CoreHandle;

/// How long a recorded event waits for the core to have read the host's
/// panes again: a letter is its pane's agent's, and that agent is known
/// only once the node's link has brought its panes back.
pub const PANES_WITHIN: Duration = Duration::from_secs(60);
/// How often the core is asked again meanwhile.
pub const LOOK_AGAIN: Duration = Duration::from_millis(250);
/// How long the core has to take one letter.
const LETTER_WITHIN: Duration = Duration::from_secs(5);

/// What takes the events of this core's node on the machine `device`.
pub trait Events: Send + Sync {
    /// Applies one event. An event whose target the core does not know
    /// yet is answered [`Applied::Later`] and tried again, until `last`
    /// says its wait is over: then the target is gone.
    fn apply(&self, device: &str, event: &Event, last: bool) -> Applied;
}

/// A letter as the node kept it (`crate::guest_node::away`).
#[derive(Deserialize)]
struct Letter {
    /// The pane that sent it, as the node names it.
    pane: String,
    command: hide_core::delivery::Command,
}

enum Refused {
    /// The core does not know the pane or its agent, which it may not
    /// have read yet.
    PaneUnknown(String),
    /// The core answered the letter itself.
    Final(String),
}

pub struct Letters {
    core: Arc<CoreHandle>,
}

impl Letters {
    pub fn new(core: Arc<CoreHandle>) -> Self {
        Self { core }
    }

    /// Runs `letter` as the pane's own command, or says why not.
    fn send(&self, device: &str, letter: &Letter) -> Result<(), Refused> {
        let pane = address::scoped_pane_id(device, &letter.pane);
        // The core knows the pane, and the agent in it, only once the
        // node's link has brought the host's panes back.
        let unknown = |reason: String| Refused::PaneUnknown(reason);
        let context = self
            .core
            .workspace_query(device, &pane, Query::Info)
            .map_err(|refusal| unknown(refusal.reason.to_owned()))?
            .context;
        self.core
            .prepare_delivery(
                &context.device_id,
                &pane,
                &context,
                None,
                letter.command.clone(),
            )
            .map_err(|code| match code.as_str() {
                "agent_pane_required" | "caller_context_changed" => unknown(code),
                _ => Refused::Final(code),
            })?
            .run(LETTER_WITHIN)
            .map(drop)
            .map_err(Refused::Final)
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
        if !matches!(letter.command, hide_core::delivery::Command::Send { .. }) {
            return Applied::Unappliable("a kept letter is a send".to_owned());
        }
        match self.send(device, &letter) {
            Ok(()) => Applied::Done,
            Err(Refused::Final(code)) => Applied::Unappliable(code),
            // Past the wait the pane, or its agent, is gone: the letter
            // has no one to be from.
            Err(Refused::PaneUnknown(reason)) if last => Applied::Unappliable(reason),
            Err(Refused::PaneUnknown(_)) => Applied::Later,
        }
    }
}
