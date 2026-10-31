//! Letters held because their recipient's session cannot be read.
//!
//! The doorbell holds a letter to an agent whose read reports turns until a
//! read settles what its turn waits for (`session_unread`). When the read
//! refused for a reason reading again cannot clear (`Observation::unreadable`,
//! the reader's code), that hold would last until the letter expires, so the
//! sender is told once, by `hide:delivery`, why and what it can do. The letter
//! stays pending: a read that lands rings it as usual, and a `--force` letter
//! is typed without waiting for one.
//!
//! "Once" is the note's intent, `session-unreadable:<letter>`: the ledger
//! takes one letter per sender and intent, so the note a restart sends again
//! converges on the first. Memory here only keeps a pass from asking twice.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use crate::delivery::doorbell::Turn;
use crate::delivery::ledger::State;

/// The Hide role that tells a sender its letter waits on a session Hide
/// cannot read.
pub(crate) const DELIVERY_ROLE: &str = "delivery";

/// How long one note may take to land.
const NOTE_CALL: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Default)]
pub(in crate::runtime) struct UnreadableLetters {
    /// A published ledger or a pane's unreadable reason moved; the tick looks
    /// only then.
    dirty: bool,
    /// Letters whose sender was told, while they stay pending.
    noted: BTreeSet<String>,
    /// A thread is telling senders; it is the only one.
    noting: bool,
}

impl UnreadableLetters {
    pub(in crate::runtime) fn touch(&mut self) {
        self.dirty = true;
    }
}

struct Note {
    letter: String,
    target: Observation,
    body: String,
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

    /// Tells each sender whose pending letter waits on a session Hide cannot
    /// read, once per letter. Run by the coordinator's tick; a tick with
    /// nothing moved is one flag test.
    pub(crate) fn drive_unreadable_letters(&mut self) {
        if !self.unreadable_letters.dirty || self.unreadable_letters.noting || self.effects_held() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(Arc::clone) else {
            return;
        };
        self.unreadable_letters.dirty = false;
        let pending = ledger
            .letters
            .iter()
            .filter(|letter| letter.state == State::Pending)
            .map(|letter| letter.id.as_str())
            .collect::<BTreeSet<_>>();
        self.unreadable_letters
            .noted
            .retain(|id| pending.contains(id.as_str()));
        let mut notes = Vec::new();
        for letter in &ledger.letters {
            if letter.state != State::Pending
                || letter.forced
                || self.unreadable_letters.noted.contains(&letter.id)
            {
                continue;
            }
            let Some(reason) = self
                .delivery_unreadable(&letter.recipient)
                .map(str::to_owned)
            else {
                continue;
            };
            self.unreadable_letters.noted.insert(letter.id.clone());
            // Hide's own roles and the operator read no letters.
            if letter.sender.code_owned() {
                continue;
            }
            let Some(target) = self
                .delivery_observations
                .get(&letter.sender.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.sender))
                .cloned()
            else {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "delivery.unreadable_note_unsent",
                    "letter_id": letter.id, "code": "sender_unavailable",
                }));
                continue;
            };
            crate::diagnostic!(json!({
                "component": "delivery", "kind": "delivery.recipient_unreadable",
                "letter_id": letter.id, "pane_id": letter.recipient.pane_id, "reason": &reason,
            }));
            notes.push(Note {
                body: note_body(&letter.recipient.name, &letter.kind, &letter.id, &reason),
                letter: letter.id.clone(),
                target,
            });
        }
        if notes.is_empty() {
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            // A runtime without a worker (a test) sends nothing.
            return;
        };
        let (client, authority, actor) = match self.hide_delivery(DELIVERY_ROLE) {
            Ok(delivery) => delivery,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "delivery.unreadable_note_unsent",
                    "count": notes.len(), "code": code,
                }));
                return;
            }
        };
        self.unreadable_letters.noting = true;
        let runtime = context.runtime.clone();
        let spawned = std::thread::Builder::new()
            .name("hide-unreadable-notes".to_owned())
            .spawn(move || {
                // A send that panics must not leave the runtime thinking a
                // thread still tells senders, or no note would go again.
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if let Some(runtime) = self.0.upgrade() {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            guard.unreadable_letters.noting = false;
                        }
                    }
                }
                let _ending = Ending(runtime);
                for note in notes {
                    let prepared = crate::delivery::worker::Prepared::new(
                        client.clone(),
                        crate::delivery::worker::Authority {
                            caller: authority.caller.clone(),
                            context: authority.context.clone(),
                        },
                        actor.clone(),
                        Some(note.target.clone()),
                        crate::delivery::Command::Send {
                            target: note.target.actor.pane_id.clone(),
                            intent: format!("session-unreadable:{}", note.letter),
                            body: note.body,
                            // Hide reads no answer, and the note wakes no one.
                            kind: "progress".into(),
                            now: false,
                            force: false,
                        },
                    );
                    if let Err(code) = prepared.run(NOTE_CALL) {
                        crate::diagnostic!(json!({
                            "component": "delivery", "kind": "delivery.unreadable_note_unsent",
                            "letter_id": note.letter, "code": code,
                        }));
                    }
                }
            });
        if let Err(error) = spawned {
            self.unreadable_letters.noting = false;
            crate::diagnostic!(json!({
                "component": "delivery", "kind": "delivery.unreadable_note_unsent",
                "code": "thread_not_started", "message": error.to_string(),
            }));
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
