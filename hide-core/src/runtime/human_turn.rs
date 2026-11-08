//! How the operator settles a person's turn an agent declared (PRD
//! ask-human): Done sends the agent one letter and takes the declaration
//! down (B18), and the operator's own message to the agent takes it down
//! with no letter (B22). The ledger worker writes both, never under the
//! runtime's lock.
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use serde_json::json;

use super::Runtime;
use crate::coordination::human_turn::{self, HumanTurn, Settled};
use crate::delivery::worker::Prepared;

/// How long Done's letter waits on the ledger worker; the settle waits as
/// every coordination write does.
const SETTLE_CALL: Duration = Duration::from_secs(10);

/// What a settle sends from its own thread.
struct Settle {
    letter: Option<Prepared>,
    clear: Clear,
    declaration: String,
}

/// Takes a declaration out of the settles under way when dropped.
struct Release(
    Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    String,
);

impl Drop for Release {
    fn drop(&mut self) {
        let mut under_way = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        under_way.remove(&self.1);
    }
}

/// The declaration's settle, as the operator's own identity.
struct Clear {
    client: crate::delivery::worker::Client,
    authority: crate::delivery::worker::Authority,
    actor: crate::delivery::Actor,
    mutation: crate::coordination::Mutation,
}

impl Runtime {
    /// Reads the standing person's turns from `ledger`, each by the pane its
    /// agent runs in now, a device's included; whether any moved, so the
    /// rows are laid again.
    pub(super) fn read_human_turns(&mut self, ledger: &crate::delivery::ledger::Ledger) -> bool {
        let laid: std::collections::HashMap<_, _> = ledger
            .human_turns
            .iter()
            .filter_map(|(agent, turn)| {
                let record = ledger
                    .agents
                    .iter()
                    .find(|record| &record.id == agent && !record.ended)?;
                Some((
                    record.actor.pane_id.clone(),
                    crate::model::DeclaredHumanTurn {
                        id: turn.id.clone(),
                        kind: turn.kind,
                        place: turn.place.clone(),
                        device: self.device_label(&record.actor.device_id),
                        since_unix_ms: turn.declared_at_unix_ms,
                    },
                ))
            })
            .collect();
        if laid == self.human_turns_laid {
            return false;
        }
        self.human_turns_laid = laid;
        true
    }

    /// Lays each standing person's turn on its agent's row, in a field of
    /// its own (PRD ask-human D-07), before the lineage and the read axis
    /// read the rows.
    pub(super) fn lay_human_turns(&self, agents: &mut [crate::model::SidebarAgentSnapshot]) {
        for agent in agents {
            agent.declared_human_turn = self.human_turns_laid.get(&agent.pane_id).cloned();
        }
    }

    /// The operator pressed Done on declaration `id` (B18): the declaring
    /// agent gets one letter saying what was done, asleep or awake, and the
    /// declaration goes. A Done for a declaration that no longer stands (a
    /// second press, another surface's press, a newer declaration) does
    /// nothing; the letter's intent and the settle by id make every repeat
    /// converge on one letter.
    pub(super) fn human_turn_done(&mut self, id: &str) -> bool {
        let (record, turn) = match self.standing_human_turn(|_, turn| turn.id == id) {
            Ok(Some(standing)) => standing,
            Ok(None) => {
                crate::diagnostic!(json!({
                    "component": "coordination",
                    "kind": "ask_human.done_stale",
                    "declaration_id": id,
                }));
                return false;
            }
            Err(code) => {
                ledger_unreadable("done", Some(id), &code);
                return false;
            }
        };
        if self.settling(&turn.id) {
            return false;
        }
        let letter = match self.operator_prepare_waking(
            &record.actor.pane_id,
            crate::delivery::Command::Send {
                target: record.actor.pane_id.clone(),
                intent: human_turn::done_intent(&turn),
                body: human_turn::done_letter(&turn),
                // A report waits for no answer; the operator's Done still
                // wakes a sleeping agent (`Letter::wakes_a_sleeper`).
                kind: "report".into(),
                now: false,
                force: false,
            },
        ) {
            Ok(letter) => Some(letter),
            // The agent's pane is gone, neither drawn nor asleep, so no
            // letter can reach it; the operator still said the turn is done,
            // and it goes. A pane still drawn but not yet seen, as right
            // after a restart, keeps the turn for another press below.
            Err(code)
                if code == "target_unavailable" && !self.pane_listed(&record.actor.pane_id) =>
            {
                crate::diagnostic!(json!({
                    "component": "coordination",
                    "kind": "ask_human.done_unaddressed",
                    "declaration_id": turn.id,
                    "agent_id": record.id,
                    "pane_id": record.actor.pane_id,
                }));
                None
            }
            Err(code) => {
                // Nothing reached the agent, so the declaration stays for
                // the operator to press again.
                self.settle_failed(&turn, "letter", &code);
                return false;
            }
        };
        self.settle(Settle {
            letter,
            clear: match self.operator_settle(&turn, Settled::Done) {
                Ok(clear) => clear,
                Err(code) => {
                    self.settle_failed(&turn, "clear", &code);
                    return false;
                }
            },
            declaration: turn.id,
        });
        false
    }

    /// The operator submitted a message of their own to the agent in
    /// `pane_id`, from its pane or the phone's conversation (B22): its
    /// declaration goes, and no letter is sent, since the message says it.
    pub(super) fn human_turn_spoke(&self, pane_id: &str) {
        let turn = match self.standing_human_turn(|record, _| record.actor.pane_id == pane_id) {
            Ok(Some((_, turn))) => turn,
            Ok(None) => return,
            Err(code) => return ledger_unreadable("spoke", None, &code),
        };
        match self.operator_settle(&turn, Settled::Spoke) {
            Ok(clear) => self.settle(Settle {
                letter: None,
                clear,
                declaration: turn.id,
            }),
            Err(code) => self.settle_failed(&turn, "clear", &code),
        }
    }

    /// Whether declaration `id` already has a settle under way.
    pub(super) fn settling(&self, id: &str) -> bool {
        self.human_turns_settling
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(id)
    }

    /// The live registration and declaration `matches` picks; an error
    /// when the core could not read the ledger, never no declaration.
    fn standing_human_turn(
        &self,
        matches: impl Fn(&crate::coordination::AgentRecord, &HumanTurn) -> bool,
    ) -> Result<Option<(crate::coordination::AgentRecord, HumanTurn)>, String> {
        let ledger = self.delivery_state()?;
        Ok(ledger.human_turns.iter().find_map(|(agent, turn)| {
            let record = ledger
                .agents
                .iter()
                .find(|record| &record.id == agent && !record.ended)?;
            matches(record, turn).then(|| (record.clone(), turn.clone()))
        }))
    }

    /// The settle of `turn`, as the operator's own identity.
    fn operator_settle(&self, turn: &HumanTurn, by: Settled) -> Result<Clear, String> {
        let (client, authority, actor) = self.operator_authority()?;
        Ok(Clear {
            client,
            authority,
            actor,
            mutation: crate::coordination::Mutation::SettleHuman {
                id: turn.id.clone(),
                by,
            },
        })
    }

    fn settle_failed(&self, turn: &HumanTurn, step: &str, code: &str) {
        crate::diagnostic!(json!({
            "component": "coordination",
            "kind": "ask_human.settle_failed",
            "declaration_id": turn.id,
            "step": step,
            "code": code,
        }));
    }

    /// Runs the letter, then the settle, on a thread of their own. The
    /// settle follows a letter only once the letter is saved, so a Done
    /// whose letter failed leaves the declaration for another press.
    fn settle(&self, settle: Settle) {
        let settling = Arc::clone(&self.human_turns_settling);
        // The set holds only ids, so a panic elsewhere leaves it sound.
        let mut under_way = settling.lock().unwrap_or_else(PoisonError::into_inner);
        if !under_way.insert(settle.declaration.clone()) {
            return;
        }
        drop(under_way);
        let declaration = settle.declaration.clone();
        // Released on every exit of the thread, a panic included, or here
        // when no thread starts.
        let release = Release(settling, declaration.clone());
        let spawned = std::thread::Builder::new()
            .name("hide-human-turn".into())
            .spawn(move || {
                let _release = release;
                Self::write_settle(settle);
            });
        if let Err(error) = spawned {
            crate::diagnostic!(json!({
                "component": "coordination",
                "kind": "ask_human.settle_failed",
                "declaration_id": declaration,
                "step": "thread",
                "code": error.to_string(),
            }));
        }
    }

    fn write_settle(settle: Settle) {
        if let Some(letter) = settle.letter
            && let Err(code) = letter.run(SETTLE_CALL)
        {
            crate::diagnostic!(json!({
                "component": "coordination",
                "kind": "ask_human.settle_failed",
                "declaration_id": settle.declaration,
                "step": "letter",
                "code": code,
            }));
            return;
        }
        let Clear {
            client,
            authority,
            actor,
            mutation,
        } = settle.clear;
        if let Err(code) =
            crate::coordination::executor::mutate(&client, &authority, &actor, mutation)
        {
            crate::diagnostic!(json!({
                "component": "coordination",
                "kind": "ask_human.settle_failed",
                "declaration_id": settle.declaration,
                "step": "clear",
                "code": code,
            }));
        }
    }
}

/// A settle that could not read the ledger settles nothing, and says so.
fn ledger_unreadable(step: &str, declaration: Option<&str>, code: &str) {
    crate::diagnostic!(json!({
        "component": "coordination",
        "kind": "ask_human.ledger_unreadable",
        "step": step,
        "declaration_id": declaration,
        "code": code,
    }));
}
