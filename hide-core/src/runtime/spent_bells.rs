//! Letters no bell can bring any more (`docs/delivery.md`, Input an agent
//! did not take): their three bells are spent and none was taken, so waiting
//! for the 60-minute deadline would only keep the sender waiting on a letter
//! that cannot arrive (with-hide/hide#2180). Such a letter ends undelivered
//! at once, as one whose recipient's wake failed does, and its sender gets
//! one letter from `hide:input` naming the recipient, its pane, why and what
//! to do now.
//!
//! A letter is judged only once the result of its last bell is recorded
//! (`Letter::bells_spent`), never while that bell is on its way. A bell is
//! not taken when it typed nothing and the agent rests, or when the agent
//! rested the record wait (`typed::RECORD_WAIT_MS`) since the last bell
//! that was or may have been typed, recorded none of them, and a fresh read
//! of its session found none either, or recorded a bell line no prompt hook
//! of Hide's handed the letter with (`hook_not_taken`): a recorded line
//! leaves a letter to its next bell, and a spent letter has none. An agent
//! whose record Hide cannot hear
//! (hooks not connected) is never judged here: its sender is told the input
//! cannot be confirmed, and the letter waits for `hide inbox` as before.
//! The sender of a letter judged here hears of it only from here
//! (`Runtime::spent_bells_tell`), not from the unconfirmed-input notice too.
//!
//! What is owed is memory: the store ends the letter on its next pass
//! (`Runtime::delivery_undeliverable`), and the note's intent,
//! `letter-undelivered:<letter>`, makes a note a restart sends again the
//! same letter. Bounded by the open letters.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use super::Runtime;
use super::hide_notes::{Note, NoteKind};
use super::typed_input::Reread;
use crate::agent_state::AgentActivity;
use crate::delivery::ledger::{BELLS, Letter, State};
use crate::delivery::typed::{self, Standing, TypedInput};

#[derive(Default)]
pub(in crate::runtime) struct SpentBells {
    /// Letters judged undeliverable, with why and the pane they were rung
    /// in, until the store ended them and their sender was told.
    failed: BTreeMap<String, Spent>,
    /// A thread is telling senders; it is the only one.
    noting: bool,
}

struct Spent {
    reason: String,
    pane: String,
    noted: bool,
}

impl Runtime {
    /// The letters that cannot be delivered now, with why, for the delivery
    /// store's pass: those a failed wake leaves and those no bell can bring.
    pub(crate) fn delivery_undeliverable(&self) -> BTreeMap<String, String> {
        let mut failed = self.delivery_wake_failures();
        for (id, spent) in &self.spent_bells.failed {
            failed
                .entry(id.clone())
                .or_insert_with(|| spent.reason.clone());
        }
        failed
    }

    /// Judges the pending letters whose bells are spent, and tells each
    /// sender whose letter the store ended undelivered. Run by the
    /// coordinator's tick; memory only.
    pub(crate) fn drive_spent_bells(&mut self, now: u64) {
        if self.effects_held() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(Arc::clone) else {
            return;
        };
        self.spent_bells.failed.retain(|id, spent| {
            ledger
                .letters
                .iter()
                .find(|letter| &letter.id == id)
                .is_some_and(|letter| {
                    letter.state == State::Pending
                        || (letter.state == State::Undelivered && !spent.noted)
                })
        });
        for letter in &ledger.letters {
            if self.spent_bells.failed.contains_key(&letter.id) {
                continue;
            }
            let Some(reason) = self.spent_bell(letter, now) else {
                continue;
            };
            crate::diagnostic!(json!({
                "component": "delivery", "kind": "doorbell.spent",
                "letter_id": letter.id, "pane_id": letter.recipient.pane_id, "reason": reason,
            }));
            self.spent_bells.failed.insert(
                letter.id.clone(),
                Spent {
                    reason,
                    pane: letter.recipient.pane_id.clone(),
                    noted: false,
                },
            );
        }
        self.note_spent_bells(&ledger);
    }

    /// Whether this judgment, not the unconfirmed-input notice
    /// (`Runtime::typed_input_work`), tells the sender of `letter`, rung in
    /// `pane`, what became of a bell that was typed: its last bell ended
    /// (`Letter::bells_spent`) and its recipient's record can be heard, as
    /// its source says or as the record of its bell line shows. An agent
    /// whose record cannot be heard gets the notice that says so.
    pub(super) fn spent_bells_tell(&self, letter: &Letter, pane: &str) -> bool {
        letter.bells_spent()
            && (self.input_source(&letter.recipient.kind, pane).is_ok()
                || self.bell_line_recorded(letter))
    }

    /// Why no bell can bring `letter` any more, or `None` while one can,
    /// one is on its way or may still be taken, or its record cannot be
    /// heard.
    fn spent_bell(&self, letter: &Letter, now: u64) -> Option<String> {
        if letter.state != State::Pending || letter.handed || !letter.bells_spent() {
            return None;
        }
        let observation = self
            .delivery_observations
            .get(&letter.recipient.pane_id)
            .filter(|observation| observation.actor.same_identity(&letter.recipient))?;
        let failure = letter.bell_failure.as_ref();
        // The last bell that was, or may have been, typed.
        let typed_at = letter
            .bell_typed_at_unix_ms
            .into_iter()
            .chain(
                failure
                    .filter(|failure| failure.maybe_typed)
                    .map(|failure| failure.at_unix_ms),
            )
            .max();
        let Some(typed_at) = typed_at else {
            // No bell was typed: each ended failed, or held after its
            // reservation, and the last says why. An agent that works now
            // may still take the letter from a hook of its turn.
            let resting = observation
                .projected_turn()
                .is_some_and(|projected| projected.turn.activity != AgentActivity::Working);
            return failure
                .filter(|_| resting)
                .map(|failure| failure.code.clone());
        };
        // A bell line the record holds reached the agent, but its prompt
        // hook did not hand it the letter: with no bell left, nothing will.
        let recorded = self.bell_line_recorded(letter);
        let reason = if recorded {
            NOT_TAKEN.to_owned()
        } else {
            failure.map_or_else(|| NOT_RECORDED.to_owned(), |failure| failure.code.clone())
        };
        if !self.spent_bells_tell(letter, &observation.actor.pane_id) {
            return None;
        }
        // A typed bell's letter with no record waits for the fresh session
        // read the unconfirmed-input judgment asks once its record wait ran
        // out; a recorded line is already the record that read would find.
        if letter.bell_typed_at_unix_ms.is_some()
            && !recorded
            && self.reread(&letter.id, BELLS, &observation.actor.pane_id, now) != Reread::Settled
        {
            return None;
        }
        let input = TypedInput {
            attempts: BELLS,
            typed_at_unix_ms: typed_at,
            ..TypedInput::default()
        };
        matches!(
            typed::standing(&input, Some(observation), now),
            Standing::Unconfirmed { .. }
        )
        .then_some(reason)
    }

    /// Tells each sender whose letter the store ended undelivered here who
    /// did not get it and why, once. A sender that is Hide's or the
    /// operator's own reads no letters.
    fn note_spent_bells(&mut self, ledger: &crate::delivery::ledger::Ledger) {
        if self.spent_bells.noting {
            return;
        }
        let mut notes = Vec::new();
        let due = self
            .spent_bells
            .failed
            .iter_mut()
            .filter(|(_, spent)| !spent.noted)
            .filter_map(|(id, spent)| {
                let letter = ledger.letters.iter().find(|letter| &letter.id == id)?;
                (letter.state == State::Undelivered).then(|| {
                    spent.noted = true;
                    (spent.reason.clone(), spent.pane.clone(), letter)
                })
            })
            .collect::<Vec<_>>();
        for (reason, pane, letter) in due {
            if letter.sender.code_owned() {
                continue;
            }
            let target = self
                .delivery_observations
                .get(&letter.sender.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.sender))
                .cloned()
                .or_else(|| {
                    self.asleep_route(&letter.sender).map(|_| {
                        super::agent_sleep::letters::asleep_observation(letter.sender.clone())
                    })
                });
            let Some(target) = target else {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "doorbell.spent_note_unsent",
                    "letter_id": letter.id, "code": "sender_unavailable",
                }));
                continue;
            };
            notes.push(Note {
                body: note_body(
                    &letter.recipient.name,
                    &pane,
                    &letter.kind,
                    &letter.id,
                    &reason,
                ),
                letter: letter.id.clone(),
                target,
            });
        }
        if notes.is_empty() {
            return;
        }
        let kind = NoteKind {
            role: typed::INPUT_ROLE,
            thread: "hide-spent-bell-notes",
            intent: "letter-undelivered",
            component: "delivery",
            unsent: "doorbell.spent_note_unsent",
            now: true,
        };
        self.spent_bells.noting = self.send_hide_notes(kind, notes, |runtime| {
            runtime.spent_bells.noting = false;
        });
    }
}

/// The reason of a letter whose bells were typed and none recorded.
const NOT_RECORDED: &str = "not_recorded";
/// The reason of a letter whose bell line the agent's record holds, while no
/// prompt hook of Hide's handed it the letter.
const NOT_TAKEN: &str = "hook_not_taken";

/// What Hide tells a sender whose letter no bell can bring: who, in which
/// pane, why in plain words with the doorbell's code (`delivery::doorbell`,
/// `delivery::ladder`), and what to do now.
fn note_body(recipient: &str, pane: &str, kind: &str, letter: &str, reason: &str) -> String {
    let why = match reason {
        NOT_RECORDED => "Hide typed its bell and the agent recorded none of them",
        NOT_TAKEN => {
            "its bell reached the agent, but Hide's prompt hook there did not hand it the letter"
        }
        "engine_refused" => "its pane refused the typing",
        "engine_unreachable" => "its machine could not be reached to type it",
        "herdr_call_failed" => "Hide lost the answer to its typing, and the agent recorded none",
        crate::delivery::doorbell::INTERRUPTED => {
            "Hide stopped while typing its last bell, and the agent recorded none"
        }
        "herdr_format" => "the answer to its typing could not be read, and the agent recorded none",
        "herdr_parameters" => "the bell could not be put into the engine's terms",
        "doorbell_reservation_failed" | "doorbell_reservation_format" => {
            "the delivery store did not save its bell"
        }
        _ => "Hide could not type its bell",
    };
    // A bell its agent read without the letter would be read so again, so
    // the next step is the agent's hooks, not its screen.
    let next = if reason == NOT_TAKEN {
        "Give the work to another agent: a letter sent there again ends the same way until the operator fixes Hide's hooks in that pane.".to_owned()
    } else {
        format!(
            "Read its screen with `hide pane read {pane}`; once it shows its input box, send the letter again with a new --intent, or give the work to another agent."
        )
    };
    format!(
        "{recipient} did not get your {kind} letter {letter}: {why} ({reason}), and its {BELLS} bells are spent, so Hide does not ring it again.\n{next}"
    )
}
