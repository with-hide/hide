//! The one line an agent gets when a heal brought it back after its work was
//! cut under it (PRD letters-missions-simplify D-06, D-18, D-19, B16, B17).
//!
//! An engine switch, a reboot, a lost holder and a device's reboot or link
//! return all end in the one heal: the engine resumes every agent whose
//! pane was cut and answers, once its hook names its session, that it
//! healed it (`Engine::hook_session`). The core then reads whether the agent
//! was at work when its engine went, from what it holds of the pane's last
//! turn (`HookFacts::take_cut`), which covers a device's panes as much as
//! this machine's. Only an agent that was at work and is a mission's
//! participant or another agent's child is owed the line; an agent at rest
//! is only reopened, and the operator's own agents are never typed into. The
//! line is the same whatever cut the work, in the operator's language.
//!
//! "Once" is the letter's intent, `heal:<pane>:<session>`: the ledger takes
//! one letter per sender and intent, so a heal that owes it again converges
//! on the first. What is owed is memory only, bounded by [`OWED_LIMIT`], and
//! an agent that does not show within [`OWED_FOR`] is given up on in the log.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use super::Runtime;
use super::hide_notes::{Note, NoteKind};
use crate::model::InterfaceLanguage;

/// The Hide role that tells a healed agent to go on.
pub(crate) const HEAL_ROLE: &str = "heal";

/// The most letters owed at once: one per pane a node holds.
const OWED_LIMIT: usize = hide_kit::panes::HOLDER_CAP;

/// How long a resumed agent may take to show in its pane.
const OWED_FOR: u64 = 10 * 60 * 1000;

#[derive(Default)]
pub(in crate::runtime) struct HealLetters {
    /// The panes whose resumed agent is owed the line, by the id the core
    /// names them with: its session, and when it was owed.
    owed: BTreeMap<String, (String, u64)>,
    /// A thread is sending them; it is the only one.
    sending: bool,
    /// The panes each machine's engine names as being healed in its last
    /// read (`ProjectionState::healing`), by the machine's own pane ids.
    healing: BTreeMap<String, BTreeSet<String>>,
}

/// The line, in `language`. Its first line is what the operator sees when
/// it wakes the agent.
fn body(language: InterfaceLanguage) -> &'static str {
    match language {
        InterfaceLanguage::Korean => "끊겼던 일을 이어서 하세요.",
        InterfaceLanguage::Japanese => "中断された作業を続けてください。",
        InterfaceLanguage::SimplifiedChinese => "请继续之前被中断的工作。",
        InterfaceLanguage::English => "Carry on with the work that was cut off.",
    }
}

impl Runtime {
    /// The agent in `caller`'s pane on `device` named its session `session`
    /// after its hook reported: `healed` when a heal resumed it there. A
    /// healed agent that was at work when its engine went is owed the line;
    /// one the engine did not heal was never cut, whatever the pane's last
    /// turn said, since its holder kept it running.
    pub(crate) fn hook_after_heal(
        &mut self,
        device: &str,
        caller: &str,
        session: &str,
        healed: bool,
        now_unix_ms: u64,
    ) {
        let pane = crate::address::named_pane_id(self.node.as_str(), device, caller);
        let mut facts = self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !healed {
            facts.forget_cut(&pane);
            return;
        }
        let cut = facts.take_cut(&pane);
        drop(facts);
        crate::diagnostic!(json!({
            "component": "heal", "kind": "heal.resumed",
            "pane_id": pane, "device": device, "cut": cut,
        }));
        if cut {
            self.owe_heal_letter(&pane, session, now_unix_ms);
        }
    }

    /// Owes the agent that resumed `session` in `pane` the line.
    fn owe_heal_letter(&mut self, pane: &str, session: &str, now_unix_ms: u64) {
        let owed = &mut self.heal_letters.owed;
        if owed.len() >= OWED_LIMIT && !owed.contains_key(pane) {
            crate::diagnostic!(json!({
                "component": "heal", "kind": "heal_letter.dropped",
                "pane_id": pane, "owed": owed.len(),
            }));
            return;
        }
        owed.insert(pane.to_owned(), (session.to_owned(), now_unix_ms));
    }

    /// Keeps the panes `device`'s engine named healing in its last read.
    pub(crate) fn note_healing(&mut self, device: &str, panes: &[String]) {
        let healing = &mut self.heal_letters.healing;
        if panes.is_empty() {
            healing.remove(device);
        } else {
            healing.insert(device.to_owned(), panes.iter().cloned().collect());
        }
    }

    /// Whether the agent `record` names is being healed: its engine has yet
    /// to bring it back, or it is back and still owed its line. Nothing it
    /// does or leaves undone in that time is its own (B19).
    pub(crate) fn healing(&self, record: &crate::coordination::AgentRecord) -> bool {
        self.heal_letters
            .healing
            .get(&record.machine)
            .is_some_and(|panes| panes.contains(&record.pane))
            || self
                .heal_letters
                .owed
                .contains_key(&record.pane_on(self.node.as_str()))
    }

    /// Sends the line to each owed agent observed in its pane that is a
    /// mission's participant or a child ([`Self::heal_notes_due`]). Run by
    /// the coordinator's tick; a tick with nothing owed is one test.
    pub(crate) fn drive_heal_letters(&mut self, now_unix_ms: u64) {
        if self.heal_letters.owed.is_empty() || self.heal_letters.sending || self.effects_held() {
            return;
        }
        let notes = self.heal_notes_due(now_unix_ms);
        if notes.is_empty() {
            return;
        }
        let kind = NoteKind {
            role: HEAL_ROLE,
            thread: "hide-heal-letters",
            intent: "heal",
            component: "heal",
            unsent: "heal_letter.unsent",
            now: false,
        };
        self.heal_letters.sending = self.send_hide_notes(kind, notes, |runtime| {
            runtime.heal_letters.sending = false;
        });
    }

    /// Settles each owed line whose agent is observed in its pane: the
    /// note to send when it is a mission's participant or a child, nothing
    /// when it is the operator's own. An agent that does not show within
    /// [`OWED_FOR`] is given up on.
    pub(in crate::runtime) fn heal_notes_due(&mut self, now_unix_ms: u64) -> Vec<Note> {
        let mut notes = Vec::new();
        let mut owed = std::mem::take(&mut self.heal_letters.owed);
        let body = body(self.interface_language());
        owed.retain(|pane, (session, since)| {
            if let Some(target) = self.delivery_observations.get(pane) {
                if self.participant_or_child(&target.actor) {
                    notes.push(Note {
                        letter: format!("{pane}:{session}"),
                        target: target.clone(),
                        body: body.to_owned(),
                    });
                } else {
                    crate::diagnostic!(json!({
                        "component": "heal", "kind": "heal_letter.operator_root",
                        "pane_id": pane,
                    }));
                }
                return false;
            }
            if now_unix_ms.saturating_sub(*since) > OWED_FOR {
                crate::diagnostic!(json!({
                    "component": "heal", "kind": "heal_letter.expired",
                    "pane_id": pane,
                }));
                return false;
            }
            true
        });
        self.heal_letters.owed = owed;
        notes
    }

    /// Whether the agent `actor` runs is another agent's child or a
    /// participant of an open mission (its Conductor; a worker or checker
    /// is the Conductor's child): the ones a heal tells to go on (D-19).
    fn participant_or_child(&self, actor: &crate::delivery::Actor) -> bool {
        let Some(record) = self.delivery_ledger.as_ref().ok().and_then(|ledger| {
            ledger
                .agents
                .iter()
                .find(|record| !record.ended && record.actor.same_identity(actor))
        }) else {
            return false;
        };
        record.parent.is_some()
            || self.missions.book.missions.iter().any(|mission| {
                mission.status.open()
                    && (mission.conductor.agent_id.as_deref() == Some(record.id.as_str())
                        || mission.conductor.actor.same_identity(actor))
            })
    }
}

#[cfg(test)]
impl Runtime {
    pub(in crate::runtime) fn heal_letters_owed(&self) -> Vec<String> {
        self.heal_letters.owed.keys().cloned().collect()
    }
}
