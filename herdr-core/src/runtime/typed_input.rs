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
    /// The agent's session file records the brief among its user messages:
    /// the store is told, and nothing else is done for it.
    pub recorded: bool,
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

    /// The unconfirmed inputs the doorbell has work for now: a notice owed,
    /// or a brief to type again. Bounded by the spawns and open letters the
    /// ledger keeps.
    pub(crate) fn typed_input_work(&self, now: u64) -> Vec<InputWork> {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Vec::new();
        };
        let mut work = Vec::new();
        for spawn in &ledger.spawns {
            let (Some(input), Some(brief)) = (
                spawn
                    .brief_input
                    .as_ref()
                    .filter(|input| input.confirmed_at_unix_ms.is_none() && input.attempts > 0),
                spawn.brief.as_ref(),
            ) else {
                continue;
            };
            let Some(observation) = self.spawn_observation(spawn) else {
                continue;
            };
            // An agent whose prompt hook reports no digest records the brief
            // in its session file, which the label worker reads on each of
            // its state changes.
            if self
                .row_facts_of(&observation.actor.pane_id)
                .is_some_and(|facts| {
                    facts.prompt_digests.iter().any(|digest| {
                        hide_agent_hooks::delivery::matches_prompt_digest(brief, digest)
                    })
                })
            {
                work.push(InputWork {
                    key: InputKey::Spawn(spawn.id.clone()),
                    observation: observation.clone(),
                    notice: None,
                    retype: None,
                    recorded: true,
                });
                continue;
            }
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
                    recorded: false,
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
                recorded: false,
            });
        }
        work
    }

    /// The label worker's facts on the row of `pane_id`, on this machine or
    /// a device, laid only while the pane's session proves them.
    fn row_facts_of(&self, pane_id: &str) -> Option<&crate::request_view::RowFacts> {
        self.snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.agents.iter()),
            )
            .find(|agent| agent.pane_id == pane_id)
            .and_then(|agent| agent.row_facts.as_ref())
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::coordination::{AgentRecord, Command};
    use crate::delivery::Actor;
    use crate::delivery::ledger::Ledger;
    use crate::delivery::typed::{PARENT_GRACE_MS, RECORD_WAIT_MS};
    use crate::node::TEST_NODE;

    const BRIEF: &str = "Node api: add the call.\nReport by letter.";

    /// A parent in pane `sender` that spawned a Codex into pane `child` with a
    /// brief, typed once at `typed_at`; the Codex has no session yet, as
    /// before its first turn. Returns the runtime and the spawn's id.
    fn spawned(handoff: bool, typed_at: u64) -> (tempfile::TempDir, Runtime, String) {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = Arc::try_unwrap(runtime).ok().unwrap().into_inner().unwrap();
        let parent_session = crate::engine::tokens::session_digest("parent-native");
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
        "panes": [
            {"pane_id": "sender", "terminal_id": "term-sender"},
            {"pane_id": "child", "terminal_id": "term-child"},
        ],
        "agents": [
            {"id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
             "state_change_seq": 1, "lineage_session": parent_session},
            {"id": "child", "pane_id": "child", "agent": "codex", "agent_status": "idle",
             "state_change_seq": 2},
        ]}))
        .unwrap();
        runtime.observe_delivery(TEST_NODE, &payload, None, None);
        let child = runtime.delivery_observations.get_mut("child").unwrap();
        child.status_changed_at_unix_ms = 0;
        child.last_input_at_unix_ms = 0;
        let parent = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: parent_session,
        };
        let mut ledger = Ledger::default();
        let apply = |ledger: &mut Ledger, mutation: Mutation, now: u64| {
            crate::coordination::apply(ledger, &parent, &mutation, now).unwrap()
        };
        let parent_id = apply(
            &mut ledger,
            Mutation::Register {
                record: AgentRecord {
                    id: String::new(),
                    name: Some("lead".into()),
                    machine: TEST_NODE.into(),
                    host_scope: "fixture-scope".into(),
                    native_machine: "fixture-machine".into(),
                    session: "parent-native".into(),
                    instance: "term-sender".into(),
                    pane: "sender".into(),
                    parent: None,
                    origin: None,
                    project: None,
                    actor: parent.clone(),
                    ended: false,
                    terminal_id: None,
                    forked: false,
                },
                check: false,
            },
            1,
        )["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let spawn = apply(
            &mut ledger,
            Mutation::Reserve {
                parent: parent_id.clone(),
                command: Command::Spawn {
                    parent: (!handoff).then(|| parent_id.clone()),
                    machine: None,
                    name: "worker".into(),
                    intent: "task-1".into(),
                    kind: "codex".into(),
                    repo: "/fixture".into(),
                    branch: "topic".into(),
                    path: None,
                    args: Vec::new(),
                    launch: Default::default(),
                    task: None,
                    brief: Some(BRIEF.into()),
                    beside: false,
                    mission: None,
                    plan_node: None,
                    plan: None,
                },
            },
            1,
        )["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for mutation in [
            Mutation::Advance {
                seed_tab: None,
                id: spawn.clone(),
                path: Some("/fixture/topic".into()),
                pane: Some("child".into()),
                terminal: Some("term-child".into()),
                shell: None,
                child: None,
            },
            Mutation::Typed { id: spawn.clone() },
        ] {
            apply(&mut ledger, mutation, 1);
        }
        apply(
            &mut ledger,
            Mutation::BriefTyping { id: spawn.clone() },
            typed_at,
        );
        apply(
            &mut ledger,
            Mutation::Briefed { id: spawn.clone() },
            typed_at,
        );
        runtime.publish_delivery(Arc::new(ledger), false);
        (root, runtime, spawn)
    }

    fn input_of(runtime: &Runtime, spawn: &str) -> TypedInput {
        let ledger = runtime.delivery_ledger.as_ref().unwrap();
        ledger
            .spawns
            .iter()
            .find(|record| record.id == spawn)
            .unwrap()
            .brief_input
            .clone()
            .unwrap()
    }

    /// A brief the child never recorded owes its parent one notice naming
    /// the child's pane once the child rested past the record wait; it is
    /// typed again only after input reached the pane and went quiet, and the
    /// operator's row shows it only after the parent's grace.
    #[test]
    fn an_unrecorded_brief_tells_the_parent_then_waits_for_its_screen_to_be_answered() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        assert!(
            runtime
                .typed_input_work(1_000 + RECORD_WAIT_MS - 1)
                .is_empty()
        );
        let at = 1_000 + RECORD_WAIT_MS;
        let work = runtime.typed_input_work(at);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].key, InputKey::Spawn(spawn.clone()));
        let notice = work[0].notice.as_ref().expect("the parent is told");
        assert_eq!(notice.target.actor.pane_id, "sender");
        assert_eq!(notice.intent, format!("input-unconfirmed:{spawn}:1"));
        assert!(notice.body.starts_with(
            "worker did not take its brief: read its screen with `hide pane read child`."
        ));
        assert_eq!(work[0].retype, None, "nobody answered the screen");
        assert!(
            runtime.input_asks(at).is_empty(),
            "the parent clears it first"
        );

        // Someone answered the screen through Hide.
        runtime.note_delivery_key_at("child", at + 1_000);
        let quiet = at + 1_000 + crate::delivery::INPUT_QUIET_MS;
        assert_eq!(runtime.typed_input_work(quiet - 1)[0].retype, None);
        assert_eq!(
            runtime.typed_input_work(quiet)[0].retype.as_deref(),
            Some(BRIEF)
        );

        // Told, the parent has its grace before the operator sees the row.
        let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
        let input = Actor::hide(typed::INPUT_ROLE, TEST_NODE);
        crate::coordination::apply(
            &mut ledger,
            &input,
            &Mutation::BriefNoticed {
                id: spawn.clone(),
                attempt: 1,
            },
            at,
        )
        .unwrap();
        runtime.publish_delivery(Arc::new(ledger), false);
        assert!(
            runtime.typed_input_work(at).is_empty(),
            "one notice per typing"
        );
        assert!(runtime.typed_input_work(quiet)[0].notice.is_none());
        assert!(runtime.input_asks(at + PARENT_GRACE_MS - 1).is_empty());
        let shown = runtime.input_asks(at + PARENT_GRACE_MS);
        assert_eq!(
            shown,
            vec![(
                "child".to_owned(),
                1_000,
                AskSubject::InputUnconfirmed {
                    input: InputKind::Brief,
                    spent: false
                },
            )]
        );
        assert_eq!(input_of(&runtime, &spawn).attempts, 1);
    }

    /// A handed-off child has no parent to clear its screen: its row shows
    /// the unrecorded brief as soon as it is unconfirmed.
    #[test]
    fn a_handed_off_childs_unrecorded_brief_reaches_the_operator_at_once() {
        let (_root, runtime, _) = spawned(true, 1_000);
        let at = 1_000 + RECORD_WAIT_MS;
        assert_eq!(runtime.input_asks(at).len(), 1);
    }

    /// An agent whose session file records the brief among its user
    /// messages confirms it: the work says so and owes no notice.
    #[test]
    fn a_brief_in_the_agents_session_file_is_its_record() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        let mut row = crate::sidebar::project_agents(
            serde_json::from_value(json!({"agents": [
                {"pane_id": "child", "agent": "codex", "agent_status": "idle", "state_change_seq": 2}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        row.row_facts = Some(crate::request_view::RowFacts {
            prompt_digests: vec![hide_agent_hooks::delivery::prompt_digest(BRIEF)],
            ..Default::default()
        });
        runtime.snapshot.navigator.agents.push(row);
        let work = runtime.typed_input_work(1_000 + RECORD_WAIT_MS);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].key, InputKey::Spawn(spawn));
        assert!(work[0].recorded);
        assert!(work[0].notice.is_none());
    }
}
