//! Letters held because their recipient's session cannot be read.
//!
//! The doorbell holds a letter to an agent whose read reports turns until a
//! read settles what its turn waits for (`session_unread`). When the read
//! refused for a reason reading again cannot clear (`Observation::unreadable`,
//! the reader's code), that hold would last until the letter expires, so the
//! sender's cell about the letter says why and what it can do, standing while
//! the read refuses (`runtime::status_cells`). The letter stays pending: a
//! read that lands rings it as usual, and a `--force` letter is typed without
//! waiting for one. The cell goes once the letter is no longer held so: the
//! session reads again, or the letter was delivered or ended.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use super::Runtime;
use crate::delivery::doorbell::Turn;
use crate::delivery::ledger::State;

/// The Hide role that tells a sender its letter waits on a session Hide
/// cannot read.
pub(crate) const DELIVERY_ROLE: &str = "delivery";

#[derive(Default)]
pub(in crate::runtime) struct UnreadableLetters {
    /// A published ledger or a pane's unreadable reason moved; the tick looks
    /// only then.
    dirty: bool,
    /// Letters whose sender's cell says they are held so, now.
    noted: BTreeSet<String>,
}

impl UnreadableLetters {
    pub(in crate::runtime) fn touch(&mut self) {
        self.dirty = true;
    }
}

impl Runtime {
    /// The recipient's reason its session cannot be read, when `recipient`
    /// is the agent observed in its pane and its turn is unread for it.
    pub(crate) fn delivery_unreadable(&self, recipient: &crate::delivery::Actor) -> Option<&str> {
        self.delivery_observations
            .get(&recipient.pane_id)
            .filter(|observation| observation.actor.same_identity(recipient))
            .filter(|observation| observation.turn == Turn::Unread)
            .and_then(|observation| observation.unreadable.as_deref())
    }

    /// Keeps the cell of each sender whose pending letter waits on a session
    /// Hide cannot read, standing while it does, and takes it back once the
    /// letter is no longer held so. Run by the coordinator's tick; a tick
    /// with nothing moved is one flag test.
    pub(crate) fn drive_unreadable_letters(&mut self) {
        if !self.unreadable_letters.dirty || self.effects_held() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(Arc::clone) else {
            return;
        };
        self.unreadable_letters.dirty = false;
        let mut held = BTreeSet::new();
        let mut notes = Vec::new();
        for letter in &ledger.letters {
            // Hide's own roles and the operator read no cells.
            if letter.state != State::Pending || letter.forced || letter.sender.code_owned() {
                continue;
            }
            let Some(reason) = self
                .delivery_unreadable(&letter.recipient)
                .map(str::to_owned)
            else {
                continue;
            };
            let Some(target) = self
                .delivery_observations
                .get(&letter.sender.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.sender))
                .cloned()
            else {
                if !self.unreadable_letters.noted.contains(&letter.id) {
                    crate::diagnostic!(json!({
                        "component": "delivery", "kind": "delivery.unreadable_note_unsent",
                        "letter_id": letter.id, "code": "sender_unavailable",
                    }));
                }
                continue;
            };
            if !self.unreadable_letters.noted.contains(&letter.id) {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "delivery.recipient_unreadable",
                    "letter_id": letter.id, "pane_id": letter.recipient.pane_id, "reason": &reason,
                }));
            }
            held.insert(letter.id.clone());
            notes.push((
                target.actor,
                super::status_cells::letter_subject(&letter.id),
                note_body(&letter.recipient.name, &letter.kind, &letter.id, &reason),
                // The held letter is the fact: its revision is the letter's.
                letter.created_at_unix_ms,
            ));
        }
        let gone: Vec<String> = self
            .unreadable_letters
            .noted
            .difference(&held)
            .cloned()
            .collect();
        for id in gone {
            self.retract_standing_status(&super::status_cells::letter_subject(&id));
        }
        self.unreadable_letters.noted = held;
        for (sender, subject, body, revision) in notes {
            // Refused for room: the next pass tries again.
            if self
                .post_status(&sender, subject, body, false, revision)
                .is_err()
            {
                self.unreadable_letters.dirty = true;
            }
        }
    }
}

/// What Hide tells a sender whose letter waits on a session it cannot read:
/// who, why, that the letter still waits, and what to do now.
fn note_body(recipient: &str, kind: &str, letter: &str, reason: &str) -> String {
    format!(
        "Your {kind} letter {letter} to {recipient} is not delivered: Hide cannot read {recipient}'s session ({reason}), so it cannot tell when the letter would not interrupt.\nThe letter stays pending and is delivered once the session can be read. To type it in now, send it again with `hide request send {recipient} --force`."
    )
}

#[cfg(test)]
impl Runtime {
    pub(in crate::runtime) fn unreadable_letters_noted(&self) -> Vec<String> {
        self.unreadable_letters.noted.iter().cloned().collect()
    }
}
