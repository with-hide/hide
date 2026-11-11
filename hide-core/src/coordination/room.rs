//! Room in a full ledger (engineering #15). The ledger keeps at most
//! [`AGENT_LIMIT`] registrations and [`SPAWN_LIMIT`] spawn records, and an
//! ended registration or a finished spawn stays in it as history. At either
//! cap a new registration or spawn drops the oldest record that has finished
//! and that nothing still names, and is refused with `capacity` only when
//! every record is live or named.
//!
//! A registration is named, and kept, by:
//! - another registration, as its `parent` or `origin`;
//! - a spawn record that stays, as its `parent` or `child`;
//! - an identity wait: a fork's origin, and the spawn a child is owed for;
//! - the person's turn it declared (`human_turns`);
//! - an open letter it sent or is to receive, and a watch it keeps or is
//!   kept on, matched by identity as delivery matches them;
//! - the core's books outside the ledger ([`Held`]): an open mission's
//!   Conductor, workers and checkers, and the child of a plan step still
//!   handed.
//!
//! A spawn record has finished once nothing can move it on: its child has
//! ended, or it has no child and its parent has ended, since every step of
//! a spawn is its parent's. It is kept while an identity wait or a plan
//! step still handed names it, or while its child is held, whose brief the
//! mission panel reads from it. A finished spawn names nothing of its own:
//! it goes with the registration it made or, childless, with its parent.
//!
//! Anything else that names a registration (Desk's job cards, sleep and
//! archive records, memory promotions, suggestion attempts) reads a missing
//! one as ended, and a sleeping or archived session's registration has not
//! ended, so the rule never reaches it.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;

use super::identity_wait::Owed;
use super::{AGENT_LIMIT, AgentRecord, SPAWN_LIMIT, SpawnRecord};
use crate::delivery::Actor;
use crate::delivery::ledger::Ledger;

/// The registrations and spawn records the core's books outside the ledger
/// still need (`Runtime::delivery_held`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) agents: BTreeSet<String>,
    pub(crate) spawns: BTreeSet<String>,
}

/// Where [`Held`] comes from: the core's books, read only once a full
/// ledger has to make room, or a value a caller already has.
pub(crate) trait Holds {
    fn held(&self) -> Result<&Held, String>;
}

impl Holds for Held {
    fn held(&self) -> Result<&Held, String> {
        Ok(self)
    }
}

/// Makes room for one more registration in a ledger holding
/// [`AGENT_LIMIT`]: drops the oldest ended registration nothing names, with
/// the finished spawn records that name it. The new registration's parent
/// or origin is already named: a live caller, a spawn's live parent, or a
/// fork's origin its identity wait names.
pub(super) fn for_agent(ledger: &mut Ledger, holds: &dyn Holds) -> Result<(), String> {
    if ledger.agents.len() < AGENT_LIMIT {
        return Ok(());
    }
    let held = holds.held()?;
    let named = Named::of(ledger, held);
    let Some(index) = ledger
        .agents
        .iter()
        .position(|agent| agent.ended && !named.agents.contains(agent.id.as_str()))
    else {
        crate::diagnostic!(json!({
            "component": "coordination", "kind": "registrations.full",
            "agents": ledger.agents.len(),
            "ended": ledger.agents.iter().filter(|agent| agent.ended).count(),
            "spawns": ledger.spawns.len(),
        }));
        return Err("capacity".into());
    };
    let gone = ledger.agents.remove(index);
    ledger
        .spawns
        .retain(|spawn| spawn.parent != gone.id && spawn.child.as_ref() != Some(&gone.id));
    Ok(())
}

/// Makes room for one more spawn record in a ledger holding
/// [`SPAWN_LIMIT`]: drops the oldest finished spawn record nothing keeps.
pub(super) fn for_spawn(ledger: &mut Ledger, holds: &dyn Holds) -> Result<(), String> {
    if ledger.spawns.len() < SPAWN_LIMIT {
        return Ok(());
    }
    let held = holds.held()?;
    let named = Named::of(ledger, held);
    let Some(index) = ledger
        .spawns
        .iter()
        .position(|spawn| !named.spawns.contains(spawn.id.as_str()))
    else {
        crate::diagnostic!(json!({
            "component": "coordination", "kind": "spawns.full",
            "spawns": ledger.spawns.len(),
            "agents": ledger.agents.len(),
        }));
        return Err("capacity".into());
    };
    ledger.spawns.remove(index);
    Ok(())
}

/// The records that stay because something names them, read in one pass.
struct Named<'a> {
    agents: HashSet<&'a str>,
    /// The spawn records kept: unfinished, or named.
    spawns: HashSet<&'a str>,
}

impl<'a> Named<'a> {
    fn of(ledger: &'a Ledger, held: &'a Held) -> Self {
        let ended: HashMap<&str, bool> = ledger
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.ended))
            .collect();
        // A registration the ledger does not hold is never read as ended,
        // so a record naming one is kept rather than taken for finished.
        let has_ended = |id: &str| ended.get(id).copied().unwrap_or(false);
        let mut agents: HashSet<&str> = held.agents.iter().map(String::as_str).collect();
        for agent in &ledger.agents {
            agents.extend(agent.parent.as_deref());
            agents.extend(agent.origin.as_deref());
        }
        let mut waited: HashSet<&str> = held.spawns.iter().map(String::as_str).collect();
        for wait in &ledger.identity_waits {
            match &wait.owed {
                Owed::Spawn { spawn } => {
                    waited.insert(spawn);
                }
                Owed::Fork { origin } => {
                    agents.insert(origin);
                }
            }
        }
        agents.extend(ledger.human_turns.keys().map(String::as_str));
        let mut spawns = HashSet::new();
        for spawn in &ledger.spawns {
            let finished = match spawn.child.as_deref() {
                Some(child) => has_ended(child),
                None => has_ended(&spawn.parent),
            };
            let kept = !finished
                || waited.contains(spawn.id.as_str())
                || spawn
                    .child
                    .as_ref()
                    .is_some_and(|child| held.agents.contains(child));
            if kept {
                spawns.insert(spawn.id.as_str());
                agents.insert(&spawn.parent);
                agents.extend(spawn.child.as_deref());
            }
        }
        let parties: HashSet<Identity<'_>> = ledger
            .letters
            .iter()
            .filter(|letter| letter.open())
            .flat_map(|letter| [&letter.sender, &letter.recipient])
            .chain(
                ledger
                    .watches
                    .iter()
                    .flat_map(|watch| [&watch.parent, &watch.target]),
            )
            .map(Identity::of)
            .collect();
        agents.extend(
            ledger
                .agents
                .iter()
                .filter(|agent| parties.contains(&Identity::of(&agent.actor)))
                .map(|agent| agent.id.as_str()),
        );
        Self { agents, spawns }
    }
}

/// What [`Actor::same_identity`] compares, as a key.
#[derive(Hash, PartialEq, Eq)]
struct Identity<'a> {
    pane: &'a str,
    device: &'a str,
    session: Option<&'a str>,
}

impl<'a> Identity<'a> {
    fn of(actor: &'a Actor) -> Self {
        Self {
            pane: &actor.pane_id,
            device: &actor.device_id,
            session: actor.session.as_deref(),
        }
    }
}

/// The records `before` held that `after` no longer does: what a batch
/// dropped to make room, since nothing else removes one.
pub(crate) fn dropped<'a>(
    before: &'a Ledger,
    after: &Ledger,
) -> (Vec<&'a AgentRecord>, Vec<&'a SpawnRecord>) {
    let agents: HashSet<&str> = after.agents.iter().map(|agent| agent.id.as_str()).collect();
    let spawns: HashSet<&str> = after.spawns.iter().map(|spawn| spawn.id.as_str()).collect();
    (
        before
            .agents
            .iter()
            .filter(|agent| !agents.contains(agent.id.as_str()))
            .collect(),
        before
            .spawns
            .iter()
            .filter(|spawn| !spawns.contains(spawn.id.as_str()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::human_turn::{HumanKind, HumanTurn};
    use crate::coordination::identity_wait::IdentityWait;
    use crate::coordination::tests::command;
    use crate::coordination::{Mutation, apply_with};
    use crate::delivery::ledger::{State, from_bytes};
    use serde_json::Value;

    const NOW: u64 = 1_000;

    fn actor(n: u64) -> Actor {
        Actor {
            pane_id: format!("w1:p{n}"),
            name: format!("a{n}"),
            kind: "codex".into(),
            device_id: "local".into(),
            session: crate::engine::tokens::session_digest(&format!("native-{n}")),
        }
    }

    fn agent(n: u64, ended: bool) -> AgentRecord {
        let actor = actor(n);
        AgentRecord {
            id: format!("agent-{n}"),
            name: Some(actor.name.clone()),
            machine: actor.device_id.clone(),
            host_scope: "fixture-scope".into(),
            native_machine: "fixture-machine".into(),
            session: format!("native-{n}"),
            instance: format!("terminal-{n}"),
            terminal_id: None,
            pane: actor.pane_id.clone(),
            parent: None,
            origin: None,
            project: None,
            actor,
            ended,
            forked: false,
        }
    }

    /// A spawn record `spawn-{n}` of `parent`, with `child` bound when given.
    fn spawn(n: u64, parent: u64, child: Option<u64>) -> SpawnRecord {
        serde_json::from_value(json!({
            "id": format!("spawn-{n}"), "parent": format!("agent-{parent}"),
            "intent": format!("intent-{n}"), "name": "worker", "kind": "codex",
            "repo": "/repo", "branch": "topic", "requested_path": null, "args": [],
            "path": child.map(|_| "/repo"), "pane": child.map(|child| format!("w1:p{child}")),
            "child": child.map(|child| format!("agent-{child}")),
            "completed": child.is_some(),
        }))
        .unwrap()
    }

    /// The ledger at its registration cap: `agent-1` is the live lead and
    /// every other registration, `agent-2` the oldest, has ended.
    fn full_of_agents() -> Ledger {
        Ledger {
            agents: (1..=AGENT_LIMIT as u64)
                .map(|n| agent(n, n != 1))
                .collect(),
            next_id: 100_000,
            ..Ledger::default()
        }
    }

    /// The ledger at its spawn cap: the live lead `agent-1` and the ended
    /// `agent-2`, whose childless spawns `spawn-3000` on have all finished.
    fn full_of_spawns() -> Ledger {
        Ledger {
            agents: vec![agent(1, false), agent(2, true)],
            spawns: (3000..3000 + SPAWN_LIMIT as u64)
                .map(|n| spawn(n, 2, None))
                .collect(),
            next_id: 100_000,
            ..Ledger::default()
        }
    }

    /// A new root agent registers itself.
    fn register(ledger: &mut Ledger, held: &Held) -> Result<Value, String> {
        register_agent(ledger, 90_000, held)
    }

    /// The root agent `n` registers itself.
    fn register_agent(ledger: &mut Ledger, n: u64, held: &Held) -> Result<Value, String> {
        let mut record = agent(n, false);
        record.id = String::new();
        let caller = record.actor.clone();
        let result = apply_with(
            ledger,
            &caller,
            &Mutation::Register {
                record,
                check: false,
            },
            NOW,
            held,
        );
        if result.is_ok() {
            ledger.check().unwrap();
        }
        result
    }

    /// The live lead starts a new spawn.
    fn reserve(ledger: &mut Ledger, held: &Held) -> Result<Value, String> {
        let result = apply_with(
            ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: command("new-start"),
            },
            NOW,
            held,
        );
        if result.is_ok() {
            ledger.check().unwrap();
        }
        result
    }

    fn holds_agent(ledger: &Ledger, n: u64) -> bool {
        let id = format!("agent-{n}");
        ledger.agents.iter().any(|agent| agent.id == id)
    }

    fn holds_spawn(ledger: &Ledger, n: u64) -> bool {
        let id = format!("spawn-{n}");
        ledger.spawns.iter().any(|spawn| spawn.id == id)
    }

    /// The ledger a core would load from what this one saves.
    fn reloaded(ledger: &Ledger) -> Ledger {
        from_bytes(&ledger.bytes().unwrap()).unwrap()
    }

    #[test]
    fn a_full_ledger_takes_a_registration_by_dropping_its_oldest_ended_one() {
        let mut ledger = full_of_agents();
        let answer = register(&mut ledger, &Held::default()).unwrap();
        assert_eq!(answer["id"], "agent-100000");
        assert_eq!(ledger.agents.len(), AGENT_LIMIT);
        assert!(!holds_agent(&ledger, 2));
        assert!(holds_agent(&ledger, 1) && holds_agent(&ledger, 3));
        assert_eq!(reloaded(&ledger).agents, ledger.agents);
    }

    #[test]
    fn a_full_ledger_of_live_registrations_refuses_a_new_one_with_capacity() {
        let mut ledger = full_of_agents();
        for agent in &mut ledger.agents {
            agent.ended = false;
        }
        let before = ledger.clone();
        assert_eq!(register(&mut ledger, &Held::default()).unwrap_err(), "capacity");
        assert_eq!(ledger, before);
    }

    #[test]
    fn an_ended_registration_another_names_as_parent_or_origin_stays() {
        let mut ledger = full_of_agents();
        // agent-10 was delegated by agent-2; agent-11 was forked from agent-3.
        ledger.agents[9].parent = Some("agent-2".into());
        ledger.agents[10].origin = Some("agent-3".into());
        ledger.agents[10].forked = true;
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2) && holds_agent(&ledger, 3));
        assert!(!holds_agent(&ledger, 4));
    }

    #[test]
    fn a_finished_spawn_goes_with_the_registration_it_made_or_its_ended_parent() {
        let mut ledger = full_of_agents();
        // The lead's spawn made agent-2, which ended; agent-3's start never
        // made its agent and agent-3 ended.
        ledger.agents[1].parent = Some("agent-1".into());
        ledger.spawns = vec![spawn(3000, 1, Some(2)), spawn(3001, 3, None)];
        register(&mut ledger, &Held::default()).unwrap();
        assert!(!holds_agent(&ledger, 2) && !holds_spawn(&ledger, 3000));
        register_agent(&mut ledger, 90_001, &Held::default()).unwrap();
        assert!(!holds_agent(&ledger, 3) && !holds_spawn(&ledger, 3001));
        assert_eq!(reloaded(&ledger).spawns, ledger.spawns);
    }

    #[test]
    fn an_ended_registration_an_open_letter_names_stays() {
        let mut ledger = full_of_agents();
        crate::delivery::mailbox::send(
            &mut ledger,
            &actor(2),
            &actor(1),
            "open",
            "still waiting to be read",
            "progress",
            None,
            NOW,
        )
        .unwrap();
        // agent-3's letter was read: it names no one any more.
        let read = crate::delivery::mailbox::send(
            &mut ledger,
            &actor(3),
            &actor(1),
            "read",
            "taken in",
            "progress",
            None,
            NOW,
        )
        .unwrap();
        let letter = ledger
            .letters
            .iter_mut()
            .find(|letter| letter.id == read.id)
            .unwrap();
        letter.state = State::Delivered;
        letter.hook_confirmed = Some(true);
        letter.finished_at_unix_ms = Some(NOW);
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2));
        assert!(!holds_agent(&ledger, 3));
    }

    #[test]
    fn an_ended_registration_a_watch_names_stays() {
        let mut ledger = full_of_agents();
        ledger.watches.push(
            serde_json::from_value(json!({
                "id": "watch-5000", "parent": actor(1), "target": actor(2),
                "last_activity_at_unix_ms": NOW, "first_warning_at_unix_ms": null,
                "warning_count": 0, "activity_failures": 0, "last_status": "working",
                "last_state_change_seq": null, "status_changed_at_unix_ms": NOW,
            }))
            .unwrap(),
        );
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2));
        assert!(!holds_agent(&ledger, 3));
    }

    #[test]
    fn an_ended_registration_with_a_persons_turn_stays() {
        let mut ledger = full_of_agents();
        ledger.human_turns.insert(
            "agent-2".into(),
            HumanTurn {
                id: "agent-2.1".into(),
                kind: HumanKind::Auth,
                place: "github.com/login".into(),
                declared_at_unix_ms: NOW,
            },
        );
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2));
        assert!(!holds_agent(&ledger, 3));
    }

    #[test]
    fn a_record_an_identity_wait_names_stays() {
        let mut ledger = full_of_agents();
        // A fork of agent-2 and the child agent-3's spawn still owes are
        // waited for.
        ledger.spawns.push(spawn(3000, 3, None));
        let wait = |n: u64, owed: Owed| IdentityWait {
            id: format!("wait-{n}"),
            machine: "local".into(),
            pane: format!("w1:p{n}"),
            name: format!("waited-{n}"),
            kind: "codex".into(),
            owed,
        };
        ledger.identity_waits = vec![
            wait(
                9000,
                Owed::Fork {
                    origin: "agent-2".into(),
                },
            ),
            wait(
                9001,
                Owed::Spawn {
                    spawn: "spawn-3000".into(),
                },
            ),
        ];
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2) && holds_agent(&ledger, 3) && holds_spawn(&ledger, 3000));
        assert!(!holds_agent(&ledger, 4));
        let mut spawns = full_of_spawns();
        spawns.identity_waits = vec![wait(
            9001,
            Owed::Spawn {
                spawn: "spawn-3000".into(),
            },
        )];
        reserve(&mut spawns, &Held::default()).unwrap();
        assert!(holds_spawn(&spawns, 3000));
        assert!(!holds_spawn(&spawns, 3001));
    }

    #[test]
    fn an_ended_registration_an_open_mission_or_a_handed_plan_step_holds_stays() {
        let mut ledger = full_of_agents();
        // An open mission's worker agent-2 ended; a plan step still handed
        // follows agent-3.
        let held = Held {
            agents: ["agent-2", "agent-3"].map(str::to_owned).into(),
            spawns: BTreeSet::new(),
        };
        register(&mut ledger, &held).unwrap();
        assert!(holds_agent(&ledger, 2) && holds_agent(&ledger, 3));
        assert!(!holds_agent(&ledger, 4));
    }

    #[test]
    fn a_full_ledger_takes_a_spawn_by_dropping_its_oldest_finished_one() {
        let mut ledger = full_of_spawns();
        let answer = reserve(&mut ledger, &Held::default()).unwrap();
        assert_eq!(answer["id"], "spawn-100000");
        assert_eq!(ledger.spawns.len(), SPAWN_LIMIT);
        assert!(!holds_spawn(&ledger, 3000) && holds_spawn(&ledger, 3001));
        assert_eq!(reloaded(&ledger).spawns, ledger.spawns);
    }

    #[test]
    fn a_spawn_still_moving_or_held_stays_and_none_left_refuses_with_capacity() {
        let mut ledger = full_of_spawns();
        // agent-3 runs, agent-4 ended as an open mission's worker.
        ledger.agents.extend([agent(3, false), agent(4, true)]);
        for child in &mut ledger.agents[2..] {
            child.parent = Some("agent-2".into());
        }
        ledger.spawns[0] = spawn(3000, 1, None); // the lead's start, under way
        ledger.spawns[1] = spawn(3001, 2, Some(3)); // its child runs
        ledger.spawns[2] = spawn(3002, 2, Some(4)); // its child is held
        // spawn-3003 is a plan step's, still handed.
        let held = Held {
            agents: ["agent-4"].map(str::to_owned).into(),
            spawns: ["spawn-3003"].map(str::to_owned).into(),
        };
        reserve(&mut ledger, &held).unwrap();
        assert!((3000..=3003).all(|n| holds_spawn(&ledger, n)));
        assert!(!holds_spawn(&ledger, 3004));
        for spawn in &mut ledger.spawns {
            spawn.parent = "agent-1".into();
            spawn.child = None;
            spawn.completed = false;
            spawn.path = None;
            spawn.pane = None;
        }
        let before = ledger.clone();
        let mut again = command("another-start");
        if let crate::coordination::Command::Spawn { intent, .. } = &mut again {
            *intent = "another-start".into();
        }
        let refused = apply_with(
            &mut ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: again,
            },
            NOW,
            &held,
        );
        assert_eq!(refused.unwrap_err(), "capacity");
        assert_eq!(ledger, before);
    }

    #[test]
    fn dropped_names_what_a_batch_removed() {
        let before = full_of_agents();
        let mut after = before.clone();
        register(&mut after, &Held::default()).unwrap();
        let (agents, spawns) = dropped(&before, &after);
        assert_eq!(
            agents.iter().map(|agent| agent.id.as_str()).collect::<Vec<_>>(),
            ["agent-2"]
        );
        assert!(spawns.is_empty());
    }
}
