//! Typed input awaiting the agent's own record (`delivery::typed`): which
//! briefs and bells are unconfirmed, who is told, what is typed again, and
//! which rows show it to the operator. Everything here reads what the
//! runtime already holds, the published ledger and the delivery
//! observations, under its lock and without I/O; the doorbell's thread
//! sends the notices and types the input off the lock.

use std::collections::BTreeMap;

use crate::coordination::{Mutation, SpawnMode, SpawnRecord};
use crate::delivery::typed::{self, InputKind, Standing, TypedInput};
use crate::delivery::worker::Effect;
use crate::model::AskSubject;

use super::Runtime;
use super::delivery::Observation;

/// Which typed input a piece of work is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InputKey {
    Spawn(String),
    Letter(String),
}

/// A notice owed to the agent that typed the input, or wrote the letter.
#[derive(Clone)]
pub(crate) struct Notice {
    pub target: Observation,
    pub intent: String,
    pub body: String,
    pub attempt: u8,
}

/// One unconfirmed input the doorbell's thread has work for.
#[derive(Clone)]
pub(crate) struct InputWork {
    pub key: InputKey,
    /// The pane the input was typed into, as Hide observes it now.
    pub observation: Observation,
    pub notice: Option<Notice>,
    /// The brief to type again, once someone answered the agent's screen.
    pub retype: Option<String>,
}

/// An input shown to the operator: the pane whose row shows it, since when,
/// and what it says.
pub(crate) type InputAsk = (String, u64, AskSubject);

impl Runtime {
    /// The pane a spawn's agent was started in, as the delivery observations
    /// key it: bare on this machine, in full on a device.
    fn spawn_pane_id(&self, spawn: &SpawnRecord) -> Option<String> {
        let pane = spawn.pane.as_deref()?;
        Some(match spawn.machine.as_deref() {
            Some(machine) if machine != self.node.as_str() => {
                crate::session_sync::remote_pane_id(machine, pane)
            }
            _ => pane.to_owned(),
        })
    }

    /// The observation of the pane a spawn's agent runs in, while it still
    /// runs the terminal the spawn recorded (a pane id can name another pane
    /// later).
    fn spawn_observation(&self, spawn: &SpawnRecord) -> Option<&Observation> {
        let observation = self
            .delivery_observations
            .get(&self.spawn_pane_id(spawn)?)?;
        match (&spawn.terminal, &observation.terminal_id) {
            (Some(recorded), Some(current)) if recorded != current => None,
            _ => Some(observation),
        }
    }

    /// A prompt hook in `pane_id` reported the digest of the prompt its agent
    /// took. When that is the brief a spawn typed there and is still waiting
    /// for, the brief is confirmed. It is judged before the caller's native
    /// identity is required, because Codex reports its session on that very
    /// turn and Herdr may not show it yet. Queued, never waited for: this
    /// runs under the runtime lock.
    pub(super) fn confirm_brief_by_hook(&self, pane_id: &str, digest: &str) {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        let Some(spawn) = ledger.spawns.iter().find(|spawn| {
            spawn
                .brief_input
                .as_ref()
                .is_some_and(|input| input.confirmed_at_unix_ms.is_none())
                && self.spawn_pane_id(spawn).as_deref() == Some(pane_id)
                && self.spawn_observation(spawn).is_some()
                && spawn.brief.as_deref().is_some_and(|brief| {
                    hide_agent_hooks::delivery::matches_prompt_digest(brief, digest)
                })
        }) else {
            return;
        };
        self.post_brief_confirmed(&spawn.id, "hook");
    }

    /// Queues the store's record that the brief of spawn `id` was taken.
    fn post_brief_confirmed(&self, id: &str, by: &'static str) {
        let posted =
            self.hide_delivery(typed::INPUT_ROLE)
                .and_then(|(client, authority, actor)| {
                    client.post(Effect::Agents {
                        authority,
                        actor,
                        mutation: Box::new(Mutation::BriefConfirmed { id: id.to_owned() }),
                    })
                });
        crate::diagnostic!(serde_json::json!({
            "component": "delivery", "kind": "input.confirmed", "spawn": id, "by": by,
            "queued": posted.is_ok(), "code": posted.err(),
        }));
    }

    /// The briefs a session read of `pane_id` found among the user messages
    /// it recorded, as digests (`labels::facts::SessionFacts::prompt_digests`):
    /// a brief still waiting there is confirmed.
    pub(crate) fn confirm_briefs_by_session(&self, pane_id: &str, digests: &[String]) {
        if digests.is_empty() {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        for spawn in ledger.spawns.iter().filter(|spawn| {
            spawn
                .brief_input
                .as_ref()
                .is_some_and(|input| input.confirmed_at_unix_ms.is_none())
                && self.spawn_pane_id(spawn).as_deref() == Some(pane_id)
                && self.spawn_observation(spawn).is_some()
        }) {
            let Some(brief) = spawn.brief.as_deref() else {
                continue;
            };
            if digests
                .iter()
                .any(|digest| hide_agent_hooks::delivery::matches_prompt_digest(brief, digest))
            {
                self.post_brief_confirmed(&spawn.id, "session");
            }
        }
    }

    /// The unconfirmed inputs the doorbell has work for now: a notice owed,
    /// or a brief to type again. Bounded by the spawns and open letters the
    /// ledger keeps.
    pub(crate) fn typed_input_work(&self, now: u64) -> Vec<InputWork> {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Vec::new();
        };
        let mut work = Vec::new();
        for spawn in &ledger.spawns {
            let (Some(input), Some(brief)) = (spawn.brief_input.as_ref(), spawn.brief.as_ref())
            else {
                continue;
            };
            let Some(observation) = self.spawn_observation(spawn) else {
                continue;
            };
            if !matches!(
                typed::standing(input, Some(observation), now),
                Standing::Unconfirmed { .. }
            ) {
                continue;
            }
            let notice = typed::notice_due(input)
                .then(|| self.notice_to(&spawn.parent, ledger))
                .flatten()
                .map(|target| Notice {
                    intent: format!("input-unconfirmed:{}:{}", spawn.id, input.attempts),
                    body: typed::notice_body(
                        InputKind::Brief,
                        &spawn.name,
                        &observation.actor.pane_id,
                        None,
                    ),
                    attempt: input.attempts,
                    target,
                });
            let retype = (spawn.briefed && typed::retype_due(input, observation, now))
                .then(|| brief.clone());
            if notice.is_some() || retype.is_some() {
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice,
                    retype,
                });
            }
        }
        for letter in &ledger.letters {
            let Some(input) = letter.typed_input() else {
                continue;
            };
            let Some(observation) = self
                .delivery_observations
                .get(&letter.recipient.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.recipient))
            else {
                continue;
            };
            if !typed::notice_due(&input)
                || !matches!(
                    typed::standing(&input, Some(observation), now),
                    Standing::Unconfirmed { .. }
                )
            {
                continue;
            }
            let Some(target) = self.observed_sender(&letter.sender) else {
                continue;
            };
            work.push(InputWork {
                key: InputKey::Letter(letter.id.clone()),
                observation: observation.clone(),
                notice: Some(Notice {
                    intent: format!("input-unconfirmed:{}:{}", letter.id, input.attempts),
                    body: typed::notice_body(
                        InputKind::Letter,
                        &letter.recipient.name,
                        &observation.actor.pane_id,
                        Some(&letter.id),
                    ),
                    attempt: input.attempts,
                    target,
                }),
                retype: None,
            });
        }
        work
    }

    /// The observed pane of the registration `parent` names, the agent told
    /// of its child's unconfirmed brief.
    fn notice_to(
        &self,
        parent: &str,
        ledger: &crate::delivery::ledger::Ledger,
    ) -> Option<Observation> {
        let actor = crate::coordination::resolve_actor(ledger, parent)?;
        self.observed_sender(actor)
    }

    /// The pane `actor` runs in while it still runs there; a code-owned
    /// identity, which has no pane, is told nothing.
    fn observed_sender(&self, actor: &crate::delivery::Actor) -> Option<Observation> {
        if actor.code_owned() {
            return None;
        }
        self.delivery_observations
            .get(&actor.pane_id)
            .filter(|observation| observation.actor.same_identity(actor))
            .cloned()
    }

    /// The rows that show unconfirmed input to the operator, by pane: once
    /// no parent can clear it, its parent had its grace, or no typing is
    /// left (`typed::shown_to_operator`).
    pub(crate) fn input_asks(&self, now: u64) -> Vec<InputAsk> {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Vec::new();
        };
        let mut asks = BTreeMap::new();
        for spawn in &ledger.spawns {
            let Some(input) = spawn
                .brief_input
                .as_ref()
                .filter(|input| input.confirmed_at_unix_ms.is_none())
            else {
                continue;
            };
            let Some(observation) = self.spawn_observation(spawn) else {
                continue;
            };
            let Standing::Unconfirmed { since } = typed::standing(input, Some(observation), now)
            else {
                continue;
            };
            let has_parent = spawn.mode == SpawnMode::Delegation
                && self.notice_to(&spawn.parent, ledger).is_some();
            if typed::shown_to_operator(input, has_parent, now) {
                asks.entry(observation.actor.pane_id.clone())
                    .or_insert((since, subject(InputKind::Brief, input)));
            }
        }
        for letter in &ledger.letters {
            let Some(input) = letter.typed_input() else {
                continue;
            };
            let Some(observation) = self
                .delivery_observations
                .get(&letter.recipient.pane_id)
                .filter(|observation| observation.actor.same_identity(&letter.recipient))
            else {
                continue;
            };
            let Standing::Unconfirmed { since } = typed::standing(&input, Some(observation), now)
            else {
                continue;
            };
            let has_sender = self.observed_sender(&letter.sender).is_some();
            if typed::shown_to_operator(&input, has_sender, now) {
                asks.entry(observation.actor.pane_id.clone())
                    .or_insert((since, subject(InputKind::Letter, &input)));
            }
        }
        asks.into_iter()
            .map(|(pane, (since, subject))| (pane, since, subject))
            .collect()
    }
}

impl Runtime {
    /// Keeps the rows that show unconfirmed input current, on each doorbell
    /// pass: a change relays the core's questions on the rows shown.
    /// Whether any row moved.
    pub(super) fn publish_input_asks(&mut self, now: u64) -> bool {
        let asks = self.input_asks(now);
        if asks == self.plans.input_asks {
            return false;
        }
        for (pane, _, subject) in &asks {
            if !self
                .plans
                .input_asks
                .iter()
                .any(|(shown, ..)| shown == pane)
            {
                crate::diagnostic!(serde_json::json!({"component":"delivery",
                    "kind":"input.shown","pane_id":pane,"subject":subject}));
            }
        }
        self.plans.input_asks = asks;
        self.plans.input_edit += 1;
        self.refresh_plan_reviews()
    }
}

fn subject(input: InputKind, typed_input: &TypedInput) -> AskSubject {
    AskSubject::InputUnconfirmed {
        input,
        spent: typed_input.attempts >= typed::INPUT_ATTEMPTS,
    }
}
