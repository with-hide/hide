//! The one letter an agent gets when its turn was cut off under it (PRD
//! engine-bundle-4-herdr-removal D-22, B9): the move of Hide's Herdr state into
//! the core's own engine, or the machine going down under the engine, ended
//! a turn its hook last said was running or waiting on the operator. The
//! engine's restore resumes the conversation and says which resumes were cut
//! once the resumed agent's hook names its session (`Engine::hook_session`);
//! the letter goes once that agent is the one observed in its pane. An agent
//! that was resting gets none.
//!
//! "Once" is the letter's intent, `transition-cut:<pane>:<session>`: the
//! ledger takes one letter per sender and intent, so a restart that owes it
//! again converges on the first. What is owed is memory only, bounded by
//! [`OWED_LIMIT`], and an agent that does not show within [`OWED_FOR`] is
//! given up on in the log.

use std::collections::BTreeMap;

use serde_json::json;

use super::Runtime;
use super::hide_notes::{Note, NoteKind};

/// The Hide role that tells an agent its turn was cut off.
pub(crate) const TRANSITION_ROLE: &str = "transition";

/// The letter. Its first line is what the operator sees when it wakes the
/// agent.
const BODY: &str = "Hide's switch to its own engine cut off what you were doing. Check the conversation and carry on where it stopped.";

/// The most letters owed at once: one per pane a node holds.
const OWED_LIMIT: usize = hide_kit::panes::HOLDER_CAP;

/// How long a resumed agent may take to show in its pane.
const OWED_FOR: u64 = 10 * 60 * 1000;

#[derive(Default)]
pub(in crate::runtime) struct TransitionLetters {
    /// The panes whose resumed agent is owed the letter: its session, and
    /// when it was owed.
    owed: BTreeMap<String, (String, u64)>,
    /// A thread is sending them; it is the only one.
    sending: bool,
}

impl Runtime {
    /// Owes the agent that resumed `session` in this machine's pane `pane`
    /// the letter.
    pub(crate) fn owe_transition_letter(&mut self, pane: &str, session: &str, now_unix_ms: u64) {
        let owed = &mut self.transition_letters.owed;
        if owed.len() >= OWED_LIMIT && !owed.contains_key(pane) {
            crate::diagnostic!(json!({
                "component": "own_engine", "kind": "transition_letter.dropped",
                "pane_id": pane, "owed": owed.len(),
            }));
            return;
        }
        owed.insert(pane.to_owned(), (session.to_owned(), now_unix_ms));
    }

    /// Sends each owed letter whose agent is observed in its pane. Run by
    /// the coordinator's tick; a tick with nothing owed is one test.
    pub(crate) fn drive_transition_letters(&mut self, now_unix_ms: u64) {
        if self.transition_letters.owed.is_empty()
            || self.transition_letters.sending
            || self.effects_held()
        {
            return;
        }
        let mut notes = Vec::new();
        let observations = &self.delivery_observations;
        self.transition_letters
            .owed
            .retain(|pane, (session, since)| {
                if let Some(target) = observations.get(pane) {
                    notes.push(Note {
                        letter: format!("{pane}:{session}"),
                        target: target.clone(),
                        body: BODY.to_owned(),
                    });
                    return false;
                }
                if now_unix_ms.saturating_sub(*since) > OWED_FOR {
                    crate::diagnostic!(json!({
                        "component": "own_engine", "kind": "transition_letter.expired",
                        "pane_id": pane,
                    }));
                    return false;
                }
                true
            });
        if notes.is_empty() {
            return;
        }
        let kind = NoteKind {
            role: TRANSITION_ROLE,
            thread: "hide-transition-letters",
            intent: "transition-cut",
            component: "own_engine",
            unsent: "transition_letter.unsent",
        };
        self.transition_letters.sending = self.send_hide_notes(kind, notes, |runtime| {
            runtime.transition_letters.sending = false;
        });
    }
}

#[cfg(test)]
impl Runtime {
    pub(in crate::runtime) fn transition_letters_owed(&self) -> Vec<String> {
        self.transition_letters.owed.keys().cloned().collect()
    }
}
