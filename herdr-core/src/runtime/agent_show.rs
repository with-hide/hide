//! The `state` `hide agent show` answers: what an agent is doing and what it
//! waits on, read from the row the core already projects for the sidebar,
//! `hide agent list` and `hide ps` (docs/status-model.md) and from the input
//! ledger typed input is judged by (`delivery::typed`). It adds no status:
//! every cause is one the core already names. Owned facts are taken under the
//! runtime's lock (`Runtime::agent_state_facts`) and joined after it.

use crate::agent_state::escalation::Cause;
use crate::coordination::SpawnRecord;
use crate::delivery::Actor;
use crate::delivery::answer::{
    Activity, AgentState, InputPhase, PendingInput, Runtime as Life, WaitCause, Waiting,
};
use crate::delivery::ledger::Ledger;
use crate::delivery::typed::{self, InputKind, Standing};
use crate::model::{AskSubject, SidebarAgentSnapshot};

use super::Runtime;

/// The most characters a wait's `text` carries: a row's line, not a
/// question's whole content.
const TEXT_CHARS: usize = 160;

/// Who answers what a waiting agent asks.
pub(crate) const OPERATOR: &str = "operator";

/// What the core holds about one agent's pane, copied under its lock.
pub(crate) struct StateFacts {
    /// The row that draws the pane's agent, when it runs the session asked
    /// about and the core's read of it is current.
    row: Option<Box<SidebarAgentSnapshot>>,
    /// The core cannot read the pane now: a stale read or a device that
    /// does not answer.
    unknown: bool,
    observed_at_unix_ms: Option<u64>,
    pending: Vec<(PendingInput, Option<String>)>,
}

impl Runtime {
    /// The facts of the agent `actor` names, whose spawn or letters typed
    /// input into it as `spawns` and the letters to it say.
    pub(crate) fn agent_state_facts(&self, actor: &Actor, spawns: &[&SpawnRecord]) -> StateFacts {
        let now = super::unix_milliseconds();
        let observation = self
            .delivery_observations
            .get(&actor.pane_id)
            .filter(|observation| {
                actor.session.is_none() || observation.actor.same_identity(actor)
            });
        let disconnected = actor.device_id != self.node.as_str()
            && self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == actor.device_id)
                .is_none_or(|remote| remote.state != "connected");
        let agents = if actor.device_id == self.node.as_str() {
            Some(&self.snapshot.navigator.agents)
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == actor.device_id)
                .and_then(|remote| remote.session.as_ref())
                .map(|session| &session.agents)
        };
        let row = agents
            .and_then(|agents| {
                agents.iter().find(|row| {
                    row.pane_id == actor.pane_id
                        && (actor.session.is_none() || row.lineage_session == actor.session)
                })
            })
            .cloned()
            .map(Box::new);
        let unknown = disconnected
            || observation.is_none_or(|observation| observation.telemetry_stale)
            || row.as_ref().is_none_or(|row| row.telemetry_stale);
        let mut pending: Vec<_> = spawns
            .iter()
            .filter_map(|spawn| self.spawn_pending(spawn, now))
            .collect();
        if let Ok(ledger) = self.delivery_ledger.as_ref() {
            for letter in ledger
                .letters
                .iter()
                .filter(|letter| letter.recipient.same_identity(actor))
            {
                let Some(input) = letter.typed_input() else {
                    continue;
                };
                if let Some(phase) = phase(
                    &input,
                    self.input_standing(&letter.id, &input, observation, now),
                ) {
                    let sender = crate::coordination::registered_id(ledger, &letter.sender);
                    pending.push((
                        pending_input(InputKind::Letter, &input, phase, None, Some(&letter.id)),
                        sender,
                    ));
                }
            }
        }
        StateFacts {
            row,
            unknown,
            observed_at_unix_ms: observation
                .map(|observation| observation.observed_at_unix_ms)
                .filter(|at| *at > 0),
            pending,
        }
    }
}

impl Runtime {
    /// The facts of a spawn whose agent has not registered: its pane's,
    /// while the pane still runs the terminal the spawn recorded. A pane
    /// that runs another terminal now, or none Hide observes, says nothing
    /// of this spawn's agent, which is then unknown; whatever occupies the
    /// pane is never read as it.
    pub(crate) fn spawn_state_facts(&self, spawn: &SpawnRecord) -> StateFacts {
        match self.spawn_observation(spawn) {
            Some(observation) => {
                let actor = observation.actor.clone();
                self.agent_state_facts(&actor, &[spawn])
            }
            None => StateFacts {
                row: None,
                unknown: true,
                observed_at_unix_ms: None,
                pending: self
                    .spawn_pending(spawn, super::unix_milliseconds())
                    .into_iter()
                    .collect(),
            },
        }
    }

    /// The spawn's brief while its agent has not recorded it, with the
    /// parent it waits on for a delegated spawn.
    fn spawn_pending(
        &self,
        spawn: &SpawnRecord,
        now: u64,
    ) -> Option<(PendingInput, Option<String>)> {
        let input = spawn.brief_input.as_ref()?;
        let phase = phase(
            input,
            self.input_standing(&spawn.id, input, self.spawn_observation(spawn), now),
        )?;
        let parent = (spawn.mode == crate::coordination::SpawnMode::Delegation)
            .then(|| spawn.parent.clone());
        Some((
            pending_input(InputKind::Brief, input, phase, Some(&spawn.id), None),
            parent,
        ))
    }
}

/// Where typed `input` stands by its `standing` (`Runtime::input_standing`,
/// the verdict the row reads), or `None` once the agent recorded it.
fn phase(input: &typed::TypedInput, standing: Standing) -> Option<InputPhase> {
    if input.attempts == 0 {
        return None;
    }
    match standing {
        Standing::Confirmed => None,
        // No pass judged this typing yet: no record is owed on what is known.
        Standing::Waiting | Standing::Unsettled => Some(InputPhase::AwaitingRecord),
        Standing::Unconfirmed { .. } if input.attempts >= typed::INPUT_ATTEMPTS => {
            Some(InputPhase::Exhausted)
        }
        Standing::Unconfirmed { .. } => Some(InputPhase::Unconfirmed),
    }
}

fn pending_input(
    kind: InputKind,
    input: &typed::TypedInput,
    phase: InputPhase,
    spawn: Option<&str>,
    letter: Option<&str>,
) -> PendingInput {
    PendingInput {
        kind,
        spawn_id: spawn.map(str::to_owned),
        request_id: letter.map(str::to_owned),
        attempts: input.attempts,
        max_attempts: typed::INPUT_ATTEMPTS,
        phase,
        typed_at_unix_ms: input.typed_at_unix_ms,
        noticed_at_unix_ms: (input.noticed_at_unix_ms > 0).then_some(input.noticed_at_unix_ms),
    }
}

impl StateFacts {
    /// The state, for an agent in `life` whose parent registration is
    /// `parent` (a delegated child's, which answers its own questions).
    pub(crate) fn state(self, life: Option<Life>, parent: Option<&str>) -> AgentState {
        let Self {
            row,
            unknown,
            observed_at_unix_ms,
            pending,
        } = self;
        let resting = matches!(life, Some(Life::Asleep | Life::Ended));
        let activity = match &row {
            _ if resting => Activity::Stopped,
            Some(row) if !unknown => match row.activity.as_str() {
                "working" => Activity::Working,
                "stopped" => Activity::Stopped,
                _ => Activity::Unknown,
            },
            _ => Activity::Unknown,
        };
        let waiting = if resting {
            None
        } else {
            match row.as_ref().filter(|_| !unknown) {
                None => Some(Waiting::on(WaitCause::Unknown, None, None)),
                Some(row) => waiting(row, parent, &pending),
            }
        };
        AgentState {
            activity,
            lifecycle: life,
            observed_at_unix_ms,
            waiting,
            pending_inputs: pending.into_iter().map(|(input, _)| input).collect(),
        }
    }
}

impl Waiting {
    fn on(cause: WaitCause, on: Option<&str>, since: Option<u64>) -> Self {
        Self {
            cause,
            on: on.map(str::to_owned),
            since_unix_ms: since,
            letter_id: None,
            spawn_id: None,
            text: None,
        }
    }
}

/// What `row` waits on, in the order every surface ranks it: a cause that
/// raised it to the operator, a question the core laid on it, its own
/// demand, then typed input its parent has yet to clear.
fn waiting(
    row: &SidebarAgentSnapshot,
    parent: Option<&str>,
    pending: &[(PendingInput, Option<String>)],
) -> Option<Waiting> {
    let line = || {
        row.detail
            .as_deref()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.chars().take(TEXT_CHARS).collect::<String>())
    };
    let demand = || match row.demand.as_str() {
        "question" => Some(WaitCause::Question),
        "error" => Some(WaitCause::Blocked),
        _ if row.blocked || row.demand == "approval" => Some(WaitCause::Approval),
        _ => None,
    };
    if let Some(escalation) = &row.escalation {
        let cause = match escalation.cause {
            Cause::ChildBlocked => demand().unwrap_or(WaitCause::Approval),
            Cause::InputUnconfirmed => WaitCause::InputUnconfirmed,
            Cause::Draft => WaitCause::Draft,
            Cause::ParentBlocked => WaitCause::ParentBlocked,
            Cause::BellExhausted => WaitCause::BellExhausted,
            Cause::Undelivered => WaitCause::Undelivered,
            Cause::ObserverUnconfirmed => WaitCause::ObserverUnconfirmed,
        };
        let mut waiting = Waiting::on(cause, Some(OPERATOR), escalation.since_unix_ms);
        waiting.letter_id = escalation.letter_id.clone();
        waiting.text = line().filter(|_| escalation.letter_id.is_none());
        return Some(waiting);
    }
    if let Some(subject) = &row.ask_subject {
        let mut waiting = match subject {
            AskSubject::PlanReview { .. } => Waiting::on(WaitCause::PlanReview, None, None),
            AskSubject::MissionDecision { line } => {
                let mut waiting = Waiting::on(WaitCause::MissionDecision, None, None);
                waiting.text = Some(line.chars().take(TEXT_CHARS).collect());
                waiting
            }
            AskSubject::InputUnconfirmed { .. } => {
                let mut waiting = Waiting::on(WaitCause::InputUnconfirmed, None, None);
                waiting.spawn_id = pending.iter().find_map(|(input, _)| input.spawn_id.clone());
                waiting.letter_id = pending
                    .iter()
                    .find_map(|(input, _)| input.request_id.clone())
                    .filter(|_| waiting.spawn_id.is_none());
                waiting
            }
        };
        waiting.on = Some(OPERATOR.to_owned());
        waiting.since_unix_ms = row.laid_since_unix_ms;
        return Some(waiting);
    }
    if let Some(cause) = demand() {
        // A delegated child's own question is its parent's to answer
        // (docs/status-model.md, The mark); a root's is the operator's.
        let on = parent.filter(|_| row.delegated).unwrap_or(OPERATOR);
        let mut waiting = Waiting::on(cause, Some(on), row.changed_at_unix_ms);
        waiting.text = line();
        return Some(waiting);
    }
    // Typed input not recorded, before the operator is shown it: its parent
    // or sender has its grace to clear the screen.
    pending
        .iter()
        .find(|(input, _)| input.phase != InputPhase::AwaitingRecord)
        .map(|(input, owner)| {
            let mut waiting = Waiting::on(
                WaitCause::InputUnconfirmed,
                Some(owner.as_deref().unwrap_or(OPERATOR)),
                input.noticed_at_unix_ms,
            );
            waiting.spawn_id = input.spawn_id.clone();
            waiting.letter_id = input.request_id.clone();
            waiting
        })
}

/// The spawns whose brief was typed into the pane `actor` names: the spawn
/// that made the registration's child, or one whose pane it is.
pub(crate) fn spawns_of<'a>(ledger: &'a Ledger, child: Option<&str>) -> Vec<&'a SpawnRecord> {
    ledger
        .spawns
        .iter()
        .filter(|spawn| child.is_some() && spawn.child.as_deref() == child)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::coordination::executor::Shown;
    use crate::delivery::Actor;
    use crate::node::TEST_NODE;
    use crate::runtime::typed_input::tests::spawned;

    /// The row the sidebar draws for the fixture's child pane, stopped.
    fn child_row() -> crate::model::SidebarAgentSnapshot {
        crate::sidebar::project_agents(
            serde_json::from_value(json!({"agents": [
                {"pane_id": "child", "agent": "codex", "agent_status": "idle", "state_change_seq": 2}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0)
    }

    /// The parent the fixture registered, as its own pane asks.
    fn parent() -> Actor {
        Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: TEST_NODE.into(),
            session: crate::engine::tokens::session_digest("parent-native"),
        }
    }

    /// A child that never registered is found by its spawn id or by its
    /// parent's intent, without a registration: the spawn, its pane, and
    /// the brief it has not recorded, which waits on the parent first.
    #[test]
    fn an_unregistered_child_is_found_by_its_spawn_or_intent_with_its_unrecorded_brief() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        runtime.snapshot.navigator.agents.push(child_row());
        let by_id = Shown::read(&runtime, &parent(), &spawn)
            .unwrap()
            .answer(TEST_NODE);
        let by_intent = Shown::read(&runtime, &parent(), "task-1")
            .unwrap()
            .answer(TEST_NODE);
        assert_eq!(by_id, by_intent);
        assert_eq!(
            by_id["spawn"],
            json!({"id": spawn, "intent": "task-1", "name": "worker", "kind": "codex",
                "machine": null, "pane": "child", "child": null})
        );
        let state = &by_id["state"];
        assert_eq!(state["activity"], "stopped");
        assert_eq!(state["lifecycle"], json!(null));
        assert_eq!(state["waiting"]["cause"], "input_unconfirmed");
        assert_eq!(state["waiting"]["on"], "agent-1");
        assert_eq!(state["waiting"]["spawn_id"], spawn.as_str());
        assert_eq!(
            state["pending_inputs"],
            json!([{"kind": "brief", "spawn_id": spawn, "request_id": null, "attempts": 1,
                "max_attempts": 3, "phase": "unconfirmed", "typed_at_unix_ms": 1_000,
                "noticed_at_unix_ms": null}])
        );
        // Another caller's intent of the same words is not this spawn.
        let stranger = Actor {
            pane_id: "child".into(),
            ..parent()
        };
        assert_eq!(
            Shown::read(&runtime, &stranger, "task-1").err(),
            Some("agent_unavailable".into())
        );
    }

    /// A pane the core cannot read now never reads as ready: no row, or a
    /// stale read, is activity and wait unknown, not stopped with nothing.
    #[test]
    fn a_child_the_core_cannot_read_is_unknown_never_ready() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        let unread = Shown::read(&runtime, &parent(), &spawn)
            .unwrap()
            .answer(TEST_NODE);
        assert_eq!(unread["state"]["activity"], "unknown");
        assert_eq!(
            unread["state"]["waiting"],
            json!({"cause": "unknown", "on": null, "since_unix_ms": null, "letter_id": null,
                "spawn_id": null, "text": null})
        );
        let mut row = child_row();
        row.telemetry_stale = true;
        runtime.snapshot.navigator.agents.push(row);
        let stale = Shown::read(&runtime, &parent(), &spawn)
            .unwrap()
            .answer(TEST_NODE);
        assert_eq!(stale["state"]["activity"], "unknown");
        assert_eq!(stale["state"]["waiting"]["cause"], "unknown");
    }

    /// A spawn whose pane runs another terminal now is unknown: the pane's
    /// new occupant, resting with nothing to ask, is never read as the
    /// spawn's agent stopped and waiting for nothing.
    #[test]
    fn a_spawn_whose_pane_runs_another_terminal_is_unknown_never_the_occupants_state() {
        let (_root, mut runtime, spawn) = spawned(false, 1_000);
        runtime.snapshot.navigator.agents.push(child_row());
        let read = |runtime: &crate::runtime::Runtime| {
            Shown::read(runtime, &parent(), &spawn)
                .unwrap()
                .answer(TEST_NODE)["state"]
                .clone()
        };
        assert_eq!(
            read(&runtime)["activity"],
            "stopped",
            "its own terminal is read"
        );
        runtime
            .delivery_observations
            .get_mut("child")
            .unwrap()
            .terminal_id = Some("term-other".into());
        let state = read(&runtime);
        assert_eq!(state["activity"], "unknown");
        assert_eq!(state["observed_at_unix_ms"], json!(null));
        assert_eq!(
            state["waiting"],
            json!({"cause": "unknown", "on": null, "since_unix_ms": null, "letter_id": null,
                "spawn_id": null, "text": null})
        );
    }

    /// A delegated child's own question waits on its parent and a root's on
    /// the operator; a cause that raised it to the operator comes first;
    /// a resting row with nothing asked waits on nothing.
    #[test]
    fn a_wait_names_the_cores_cause_and_who_answers_it() {
        let row = |demand: &str, delegated: bool| {
            let mut row = child_row();
            row.demand = demand.into();
            row.delegated = delegated;
            row.changed_at_unix_ms = Some(5);
            row
        };
        let waiting = |row| super::waiting(&row, Some("agent-1"), &[]);
        let question = waiting(row("question", true)).unwrap();
        assert_eq!(
            (
                question.cause,
                question.on.as_deref(),
                question.since_unix_ms
            ),
            (
                crate::delivery::answer::WaitCause::Question,
                Some("agent-1"),
                Some(5)
            )
        );
        let root = waiting(row("error", false)).unwrap();
        assert_eq!(
            (root.cause, root.on.as_deref()),
            (
                crate::delivery::answer::WaitCause::Blocked,
                Some("operator")
            )
        );
        let mut raised = row("question", true);
        raised.escalation = Some(crate::agent_state::escalation::Escalation {
            cause: crate::agent_state::escalation::Cause::Undelivered,
            letter_id: Some("letter-9".into()),
            since_unix_ms: Some(3),
            human_notice: true,
        });
        let raised = waiting(raised).unwrap();
        assert_eq!(
            (
                raised.cause,
                raised.on.as_deref(),
                raised.letter_id.as_deref()
            ),
            (
                crate::delivery::answer::WaitCause::Undelivered,
                Some("operator"),
                Some("letter-9")
            )
        );
        assert_eq!(waiting(row("none", true)), None);
    }
}
