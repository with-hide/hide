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

/// Makes room for `incoming` in a ledger holding [`AGENT_LIMIT`]
/// registrations: drops the oldest ended registration nothing names, with
/// the finished spawn records that name it. The registration `incoming`
/// names as its parent or origin stays, whatever it is.
pub(super) fn for_agent(
    ledger: &mut Ledger,
    holds: &dyn Holds,
    incoming: &AgentRecord,
) -> Result<(), String> {
    if ledger.agents.len() < AGENT_LIMIT {
        return Ok(());
    }
    let held = holds.held()?;
    let named = Named::of(
        ledger,
        held,
        [incoming.parent.as_deref(), incoming.origin.as_deref()],
    );
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
    let named = Named::of(ledger, held, []);
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
    /// `keep` names registrations that stay whatever they are: the parent
    /// and origin of the registration being made room for.
    fn of<const N: usize>(ledger: &'a Ledger, held: &'a Held, keep: [Option<&'a str>; N]) -> Self {
        let ended: HashMap<&str, bool> = ledger
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.ended))
            .collect();
        // A registration the ledger does not hold is never read as ended,
        // so a record naming one is kept rather than taken for finished.
        let has_ended = |id: &str| ended.get(id).copied().unwrap_or(false);
        let mut agents: HashSet<&str> = held.agents.iter().map(String::as_str).collect();
        agents.extend(keep.into_iter().flatten());
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
