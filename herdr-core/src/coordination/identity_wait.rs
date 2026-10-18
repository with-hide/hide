//! Registrations Hide owes an agent it started whose session Herdr has not
//! shown yet. Codex runs its session hook at its first turn and OpenCode 2
//! makes no session before its first prompt, so a fork, or a spawn given no
//! prompt, can rest without one for as long as the operator leaves it. The
//! registration is kept as a wait in the ledger instead of failing after a
//! fixed time.
//!
//! The lineage writer reads each agent list Herdr answers against the waits
//! on its server ([`plan`], on the session-sync thread, with no I/O) and its
//! worker settles what it found ([`settle`]): the agent showing its session
//! in the wait's pane under the wait's name registers it as owed; the pane
//! closing, the agent ending, or another session showing in that pane ends
//! the wait. The store applies either in one transaction ([`apply`]), so a
//! settle that races the spawn's own retry converges on one registration.

use super::{AgentRecord, Mutation};
use crate::delivery::{Actor, ledger::Ledger};
use crate::session_sync::ProjectedAgent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Mutex, Weak};
use std::time::Duration;

/// How many waits the ledger holds at once. A wait ends with its pane, so
/// this many is that many agents started and left without a turn at once.
pub(crate) const LIMIT: usize = 64;
/// The refusal when every wait is taken.
pub(crate) const FULL: &str = "identity_waits_full";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityWait {
    pub id: String,
    /// The device the pane is on, as a registration's `machine` names it.
    pub machine: String,
    /// Herdr's id of the pane on that device.
    pub pane: String,
    /// The name the start gave the agent in Herdr. Herdr lists the agent
    /// under it and drops it when the agent ends, so a pane id Herdr hands
    /// out again never matches the wait.
    pub name: String,
    pub kind: String,
    pub owed: Owed,
}

/// What the agent is registered as once its session shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "for", rename_all = "snake_case")]
pub enum Owed {
    /// The child of this spawn, bound and watched as the spawn's own answer
    /// binds it.
    Spawn { spawn: String },
    /// A fork of this registration: the operator's root, with it as origin.
    Fork { origin: String },
}

/// Why a wait ended with nothing registered.
pub(crate) const PANE_CLOSED: &str = "pane_closed";
pub(crate) const AGENT_ENDED: &str = "agent_ended";
pub(crate) const PANE_TAKEN: &str = "pane_taken";

/// What the writer found for one wait in an agent list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Step {
    /// Herdr shows the agent with its session: register it.
    Bind(Box<ProjectedAgent>),
    /// Nothing will show for it any more, for this reason.
    End(&'static str),
}

/// What the store is asked to do with a wait.
#[derive(Clone, Debug)]
pub(crate) enum Settle {
    Bind(Box<AgentRecord>),
    End(String),
}

/// The step each wait on `device` takes for `agents`, one of Herdr's agent
/// lists. A wait whose agent is not listed is judged only once `known` holds
/// it, which says it was in the ledger when an earlier list was read: a list
/// asked before the wait existed may predate the agent's start. The second
/// value says some wait was left for a later list for that reason.
pub(crate) fn plan(
    ledger: &Ledger,
    device: &str,
    agents: &[ProjectedAgent],
    pane_exists: impl Fn(&str) -> bool,
    known: &HashSet<String>,
) -> (Vec<(String, Step)>, bool) {
    let mut steps = Vec::new();
    let mut deferred = false;
    for wait in ledger
        .identity_waits
        .iter()
        .filter(|wait| wait.machine == device)
    {
        let same_kind = |kind: &str| {
            hide_agent_adapter::canonical_kind(kind)
                == hide_agent_adapter::canonical_kind(&wait.kind)
        };
        let own = agents
            .iter()
            .find(|agent| agent.pane_id == wait.pane && agent.name.as_deref() == Some(&wait.name));
        let step = match own {
            // Herdr holds the name with no agent while the start is pending.
            Some(agent) => match agent.agent.as_deref() {
                Some(kind) if !same_kind(kind) => Some(Step::End(PANE_TAKEN)),
                Some(_) if agent.lineage_session.is_some() && agent.agent_session.is_some() => {
                    Some(Step::Bind(Box::new(agent.clone())))
                }
                _ => None,
            },
            None if agents.iter().any(|agent| {
                agent.pane_id == wait.pane && agent.agent.is_some() && agent.agent_session.is_some()
            }) =>
            {
                Some(Step::End(PANE_TAKEN))
            }
            None if !known.contains(&wait.id) => {
                deferred = true;
                None
            }
            None if !pane_exists(&wait.pane) => Some(Step::End(PANE_CLOSED)),
            None => Some(Step::End(AGENT_ENDED)),
        };
        if let Some(step) = step {
            steps.push((wait.id.clone(), step));
        }
    }
    (steps, deferred)
}

/// Settles wait `id` on `device` as the writer found it, on the writer's
/// worker: builds the registration from what Herdr showed, has the store
/// apply it, and writes the new registration's lineage tokens on its pane.
/// Returns the store's answer.
pub(crate) fn settle(
    runtime: &Weak<Mutex<crate::runtime::Runtime>>,
    device: &str,
    id: &str,
    step: Step,
) -> Result<Value, String> {
    let (client, ledger, context) = {
        let runtime = runtime.upgrade().ok_or("delivery_unavailable")?;
        let guard = runtime.lock().map_err(|_| "delivery_unavailable")?;
        (
            guard.naming_client().ok_or("delivery_unavailable")?,
            guard.delivery_state()?,
            guard.coordination_context(device)?,
        )
    };
    // The spawn whose pane runs an agent that can never be registered.
    let mut refused = None;
    let settle = match step {
        Step::End(reason) => Settle::End(reason.into()),
        Step::Bind(agent) => {
            let Some(wait) = ledger.identity_waits.iter().find(|wait| wait.id == id) else {
                return Ok(json!({"settled": "absent"}));
            };
            // A registration that cannot be made of the agent as it shows,
            // such as a session past what one keeps, ends the wait, as a
            // bind the ledger refuses does: asking again meets the same agent.
            match owed_record(&ledger, wait, &agent, &context) {
                Ok(record) => Settle::Bind(Box::new(record)),
                Err(reason) => {
                    if let Owed::Spawn { spawn } = &wait.owed {
                        refused = ledger
                            .spawns
                            .iter()
                            .find(|record| &record.id == spawn)
                            .map(|record| (record.clone(), wait.pane.clone(), reason.clone()));
                    }
                    Settle::End(reason)
                }
            }
        }
    };
    let answer = client.submit(
        crate::delivery::worker::Effect::IdentityWait {
            id: id.to_owned(),
            settle,
        },
        Duration::from_secs(5),
    )?;
    // The pane the spawn made is closed rather than left running with
    // nothing tracking its agent, as the spawn's own answer does; only once
    // the wait ended here, so a child a retry bound keeps its pane.
    if let Some((spawn, pane, reason)) = refused.as_ref().filter(|_| answer["ended"].is_string()) {
        super::executor::close_refused_pane(context.connector.as_ref(), spawn, pane, reason);
    }
    if let Some(agent) = answer["bound"].as_str() {
        // The lineage writer's own plan writes the same tokens when this
        // first write fails; the registration it describes is made.
        if let Err(error) =
            super::executor::publish_tokens(&client, context.connector.as_ref(), agent)
        {
            crate::diagnostic!(json!({
                "component": "lineage", "kind": "write_failed", "agent_id": agent, "message": error,
            }));
        }
    }
    Ok(answer)
}

/// The registration `wait` owes the agent Herdr shows in its pane, as the
/// spawn's or the fork's own path would have made it.
fn owed_record(
    ledger: &Ledger,
    wait: &IdentityWait,
    agent: &ProjectedAgent,
    context: &crate::runtime::delivery::CoordinationContext,
) -> Result<AgentRecord, String> {
    let host = super::executor::HostIdentity {
        machine: &wait.machine,
        on_node: context.on_node,
        scope: &context.host_scope,
        native_machine: &context.machine,
    };
    match &wait.owed {
        Owed::Spawn { spawn } => {
            let spawn = ledger
                .spawns
                .iter()
                .find(|record| &record.id == spawn)
                .ok_or("spawn_unavailable")?;
            let mut record = super::executor::record(
                agent,
                host,
                wait.pane.clone(),
                Some(wait.name.clone()),
                spawn.mode.responsibility(&spawn.parent),
                spawn.path.clone(),
            )?;
            record.origin = spawn.mode.origin(&spawn.parent);
            Ok(record)
        }
        Owed::Fork { origin } => {
            let mut record = super::executor::record(
                agent,
                host,
                wait.pane.clone(),
                agent.name.clone(),
                None,
                agent.cwd.clone(),
            )?;
            // The fork is the operator's own work: no parent supervises it,
            // and the source it was forked from is its origin.
            record.origin = Some(origin.clone());
            record.forked = true;
            Ok(record)
        }
    }
}

/// Records that Hide owes `wait`'s registration, for `caller`: the spawn's
/// parent, or the fork's source. The same agent asked again is the same
/// wait, and a spawn whose child is already bound needs none.
pub(crate) fn reserve(
    ledger: &mut Ledger,
    caller: &Actor,
    wait: &IdentityWait,
) -> Result<Value, String> {
    match &wait.owed {
        Owed::Spawn { spawn } => {
            let record = ledger
                .spawns
                .iter()
                .find(|record| &record.id == spawn)
                .ok_or("spawn_unavailable")?;
            if !super::resolve_actor(ledger, &record.parent)
                .is_some_and(|parent| parent.same_identity(caller))
            {
                return Err("parent_authority_required".into());
            }
            if record.child.is_some() {
                return Ok(Value::Null);
            }
            if record.typed == 0
                || record.pane.as_ref() != Some(&wait.pane)
                || record.name != wait.name
                || hide_agent_adapter::canonical_kind(&record.kind)
                    != hide_agent_adapter::canonical_kind(&wait.kind)
                || super::spawn_machine(ledger, record) != Some(wait.machine.as_str())
            {
                return Err("spawn_incomplete".into());
            }
        }
        Owed::Fork { origin } => {
            if !ledger.agents.iter().any(|record| {
                &record.id == origin && !record.ended && record.actor.same_identity(caller)
            }) {
                return Err("parent_authority_required".into());
            }
        }
    }
    if let Some(existing) = ledger.identity_waits.iter().find(|existing| {
        existing.machine == wait.machine && existing.pane == wait.pane && existing.name == wait.name
    }) {
        return if existing.owed == wait.owed && existing.kind == wait.kind {
            Ok(json!(existing))
        } else {
            Err("intent_conflict".into())
        };
    }
    if ledger.identity_waits.len() >= LIMIT {
        return Err(FULL.into());
    }
    let mut wait = wait.clone();
    wait.id = super::allocate(ledger, "wait")?;
    ledger.identity_waits.push(wait.clone());
    Ok(json!(wait))
}

/// Applies `settle` to wait `id` in one transaction: a bind registers the
/// agent as owed and ends the wait; a bind the ledger refuses, and every
/// end, ends the wait with that reason, so nothing is retried for an agent
/// that cannot be registered. A wait already gone answers so and changes
/// nothing, which is what a repeated settle of the same wait reads.
pub(crate) fn apply(
    ledger: &mut Ledger,
    id: &str,
    settle: &Settle,
    now: u64,
) -> Result<Value, String> {
    let Some(index) = ledger.identity_waits.iter().position(|wait| wait.id == id) else {
        return Ok(json!({"settled": "absent"}));
    };
    let record = match settle {
        Settle::End(reason) => {
            ledger.identity_waits.remove(index);
            return Ok(json!({"ended": reason}));
        }
        Settle::Bind(record) => record,
    };
    let wait = ledger.identity_waits[index].clone();
    let mut trial = ledger.clone();
    match bind(&mut trial, &wait, record, now) {
        Ok(agent) => {
            trial.identity_waits.retain(|wait| wait.id != id);
            *ledger = trial;
            Ok(json!({"bound": agent}))
        }
        Err(reason) => {
            ledger.identity_waits.remove(index);
            Ok(json!({"ended": reason}))
        }
    }
}

/// Registers `record` as `wait` owes it and returns its registration id.
fn bind(
    ledger: &mut Ledger,
    wait: &IdentityWait,
    record: &AgentRecord,
    now: u64,
) -> Result<String, String> {
    if record.pane != wait.pane
        || record.machine != wait.machine
        || record.name.as_deref() != Some(wait.name.as_str())
        || hide_agent_adapter::canonical_kind(&record.actor.kind)
            != hide_agent_adapter::canonical_kind(&wait.kind)
    {
        return Err("child_identity_changed".into());
    }
    match &wait.owed {
        Owed::Spawn { spawn } => {
            // The parent's consent is the spawn it reserved; its registration
            // must still be live for the child to be bound and watched.
            let parent = ledger
                .spawns
                .iter()
                .find(|record| &record.id == spawn)
                .and_then(|record| super::resolve_actor(ledger, &record.parent))
                .cloned()
                .ok_or("parent_unavailable")?;
            super::apply(
                ledger,
                &parent,
                &Mutation::BindChild {
                    id: spawn.clone(),
                    record: record.clone(),
                },
                now,
            )?;
            super::apply(
                ledger,
                &parent,
                &Mutation::Complete { id: spawn.clone() },
                now,
            )?;
            ledger
                .spawns
                .iter()
                .find(|record| &record.id == spawn)
                .and_then(|record| record.child.clone())
                .ok_or_else(|| "child_unavailable".into())
        }
        Owed::Fork { origin } => {
            // The source consented when it forked; it may have closed since,
            // and the fork keeps it as where it came from.
            if !record.forked
                || record.parent.is_some()
                || record.origin.as_ref() != Some(origin)
                || !ledger.agents.iter().any(|agent| &agent.id == origin)
            {
                return Err("parent_unavailable".into());
            }
            super::insert_record(ledger, record, false)?["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "child_unavailable".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar::SessionAgentSessionPayload;

    const NODE: &str = crate::node::TEST_NODE;

    fn actor(pane: &str, session: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: NODE.into(),
            session: crate::wire::session_digest(session),
        }
    }

    fn registration(id: &str, pane: &str, session: &str) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            name: Some(pane.into()),
            machine: NODE.into(),
            host_scope: "fixture".into(),
            native_machine: "fixture-machine".into(),
            session: session.into(),
            instance: pane.into(),
            pane: pane.into(),
            parent: None,
            origin: None,
            project: None,
            actor: actor(pane, session),
            ended: false,
            forked: false,
        }
    }

    /// A ledger whose source `agent-1` forked into pane `w2:p1`.
    fn forked() -> (Ledger, IdentityWait) {
        let mut ledger = Ledger {
            next_id: 2,
            agents: vec![registration("agent-1", "w1:p1", "source")],
            ..Default::default()
        };
        let wait = fork_wait("w2:p1", "fork-w1-p1-1-1");
        let reserved = reserve(&mut ledger, &actor("w1:p1", "source"), &wait).unwrap();
        (ledger, serde_json::from_value(reserved).unwrap())
    }

    fn fork_wait(pane: &str, name: &str) -> IdentityWait {
        IdentityWait {
            id: String::new(),
            machine: NODE.into(),
            pane: pane.into(),
            name: name.into(),
            kind: "codex".into(),
            owed: Owed::Fork {
                origin: "agent-1".into(),
            },
        }
    }

    /// The fork's agent as Herdr lists it, with its session once it has one.
    fn listed(pane: &str, name: &str, session: Option<&str>) -> ProjectedAgent {
        ProjectedAgent {
            pane_id: pane.into(),
            name: Some(name.into()),
            workspace_id: "w2".into(),
            tab_id: "w2:t1".into(),
            cwd: Some("/checkouts/fixture".into()),
            agent: Some("codex".into()),
            agent_status: Some("idle".into()),
            agent_session: session.map(|value| SessionAgentSessionPayload {
                kind: "id".into(),
                value: value.into(),
            }),
            spawned_from_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: session.and_then(crate::wire::session_digest),
            state_change_seq: 1,
            completion_seq: None,
            launch_pending: false,
            tokens: Default::default(),
        }
    }

    fn known(ledger: &Ledger) -> HashSet<String> {
        ledger
            .identity_waits
            .iter()
            .map(|wait| wait.id.clone())
            .collect()
    }

    /// Codex at rest after `codex fork` has no session in Herdr; its first
    /// turn reports one, and only then is the fork registered.
    #[test]
    fn a_wait_binds_once_herdr_shows_the_agents_session_and_holds_while_it_rests() {
        let (ledger, wait) = forked();
        let resting = [listed("w2:p1", &wait.name, None)];
        assert_eq!(
            plan(&ledger, NODE, &resting, |_| true, &known(&ledger)),
            (Vec::new(), false)
        );
        let reported = [listed("w2:p1", &wait.name, Some("fork"))];
        let (steps, _) = plan(&ledger, NODE, &reported, |_| true, &known(&ledger));
        assert!(
            matches!(&steps[..], [(id, Step::Bind(agent))] if *id == wait.id && agent.pane_id == "w2:p1")
        );
        // Another device's server knows nothing of this wait.
        assert_eq!(
            plan(&ledger, "mini", &reported, |_| true, &known(&ledger)).0,
            Vec::new()
        );
    }

    /// The wait ends with its pane or its agent, but only by a list read
    /// while the wait was already recorded: one asked before may predate
    /// the agent's start.
    #[test]
    fn a_wait_ends_when_its_pane_closes_or_its_agent_ends_judged_by_a_later_list() {
        let (ledger, wait) = forked();
        assert_eq!(
            plan(&ledger, NODE, &[], |_| false, &HashSet::new()),
            (Vec::new(), true),
            "a list that may predate the start judges nothing"
        );
        let ended =
            |pane_exists: bool| plan(&ledger, NODE, &[], |_| pane_exists, &known(&ledger)).0;
        assert_eq!(ended(false), [(wait.id.clone(), Step::End(PANE_CLOSED))]);
        assert_eq!(ended(true), [(wait.id.clone(), Step::End(AGENT_ENDED))]);
    }

    /// Another session in the pane is not the agent the wait is for, under
    /// another name or as another agent under the same one.
    #[test]
    fn a_wait_ends_when_another_session_shows_in_its_pane() {
        let (ledger, wait) = forked();
        let other = [listed("w2:p1", "someone-else", Some("other"))];
        assert_eq!(
            plan(&ledger, NODE, &other, |_| true, &HashSet::new()).0,
            [(wait.id.clone(), Step::End(PANE_TAKEN))]
        );
        let mut claude = listed("w2:p1", &wait.name, Some("other"));
        claude.agent = Some("claude".into());
        assert_eq!(
            plan(&ledger, NODE, &[claude], |_| true, &known(&ledger)).0,
            [(wait.id.clone(), Step::End(PANE_TAKEN))]
        );
    }

    #[test]
    fn the_same_agent_keeps_one_wait_and_the_waits_are_capped() {
        let (mut ledger, wait) = forked();
        let source = actor("w1:p1", "source");
        let again = reserve(&mut ledger, &source, &fork_wait("w2:p1", &wait.name)).unwrap();
        assert_eq!(again["id"], wait.id.as_str());
        assert_eq!(ledger.identity_waits.len(), 1);
        assert_eq!(
            reserve(
                &mut ledger,
                &actor("w9:p9", "stranger"),
                &fork_wait("w3:p1", "fork-x")
            ),
            Err("parent_authority_required".into())
        );
        for index in 1..LIMIT {
            reserve(
                &mut ledger,
                &source,
                &fork_wait(&format!("w3:p{index}"), "fork-x"),
            )
            .unwrap();
        }
        assert_eq!(ledger.identity_waits.len(), LIMIT);
        assert_eq!(
            reserve(&mut ledger, &source, &fork_wait("w4:p1", "fork-x")),
            Err(FULL.into())
        );
        super::super::validate_records(&ledger).unwrap();
    }

    /// The fork registers as the operator's root whose origin is its source,
    /// even when the source closed while the fork rested; settling the same
    /// wait again finds it gone and registers nothing more.
    #[test]
    fn a_fork_wait_registers_the_operators_root_once_even_after_its_source_ended() {
        let (mut ledger, wait) = forked();
        ledger.agents[0].ended = true;
        let mut record = registration("", "w2:p1", "fork");
        record.name = Some(wait.name.clone());
        record.origin = Some("agent-1".into());
        record.forked = true;
        let settle = Settle::Bind(Box::new(record));
        let answer = apply(&mut ledger, &wait.id, &settle, 1).unwrap();
        let id = answer["bound"].as_str().unwrap().to_owned();
        let fork = ledger.agents.iter().find(|agent| agent.id == id).unwrap();
        assert_eq!(
            (fork.parent.as_deref(), fork.origin.as_deref()),
            (None, Some("agent-1"))
        );
        assert!(fork.forked);
        assert!(ledger.identity_waits.is_empty());
        super::super::validate_records(&ledger).unwrap();
        assert_eq!(
            apply(&mut ledger, &wait.id, &settle, 2).unwrap(),
            json!({"settled": "absent"})
        );
        assert_eq!(ledger.agents.len(), 2);
    }

    /// A registration the ledger refuses ends the wait with that reason,
    /// so nothing is asked again for an agent that cannot be registered.
    #[test]
    fn a_bind_the_ledger_refuses_ends_the_wait_and_registers_nothing() {
        let (mut ledger, wait) = forked();
        let mut record = registration("", "w2:p1", "fork");
        record.name = Some("another-name".into());
        let answer = apply(&mut ledger, &wait.id, &Settle::Bind(Box::new(record)), 1).unwrap();
        assert_eq!(answer, json!({"ended": "child_identity_changed"}));
        assert!(ledger.identity_waits.is_empty());
        assert_eq!(ledger.agents.len(), 1);
    }
}
