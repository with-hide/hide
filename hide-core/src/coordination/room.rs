//! Room in a full ledger (engineering #15). The ledger keeps at most
//! [`AGENT_LIMIT`] registrations and [`SPAWN_LIMIT`] spawn records, and an
//! ended registration or a finished spawn stays in it as history. At either
//! cap a new registration or spawn drops the oldest record that has finished
//! and that nothing still names, and is refused with `capacity` only when
//! every record is live or named.
//!
//! The file holds at most [`FILE_LIMIT`] bytes, which finished spawn records
//! reached long before their count did while each kept its brief. Any change
//! that would take the file past it, whatever it adds, drops the finished
//! letters that finished first (`mailbox::finished_oldest_first`, the rule
//! of the letter total), then the oldest finished spawn records nothing
//! keeps, then the oldest ended registrations nothing names, until the file
//! fits; it is refused with `capacity` only when what is left still does not
//! ([`for_size`]). A completed spawn whose child ended lets its brief and
//! launch words go ([`shed_finished_briefs`]), so its record stays small.
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
//! A registration taken over by the next agent in its pane (`succeed`) has
//! not ended here while an agent of its line is live: the work it was
//! started for runs on, and whoever counts that work reads it from the
//! spawn that started it, through its line (`at_work`). So it is named, and
//! its spawn has not finished.
//!
//! A spawn record has finished once nothing can move it on: its child's
//! line has ended, or it has no child and its parent has ended, since every
//! step of a spawn is its parent's. It is kept while an identity wait or a
//! plan step still handed names it, or while the last of its child's line
//! is held, so the mission or plan that holds the agent at its work still
//! finds the spawn that made it; a kept spawn names its child's whole line.
//! A finished spawn names nothing of its own:
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
use crate::delivery::ledger::{Ledger, encoded_len};
use crate::delivery::{Actor, FILE_LIMIT};

/// The registrations and spawn records the core's books outside the ledger
/// still need (`Runtime::delivery_held`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) agents: BTreeSet<String>,
    pub(crate) spawns: BTreeSet<String>,
    published: Option<Finished>,
}

/// Only records already finished in the published ledger are eligible.
/// A binding reads that ledger while the store writes its candidate, so
/// ending a record in this batch cannot make it safe to drop in this batch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Finished {
    agents: HashSet<String>,
    spawns: HashSet<String>,
}

impl Held {
    pub(crate) fn keep_published(&mut self, ledger: &Ledger) {
        let ended = ended(ledger);
        self.published = Some(Finished {
            agents: ended
                .iter()
                .filter(|(_, ended)| **ended)
                .map(|(id, _)| (*id).to_owned())
                .collect(),
            spawns: ledger
                .spawns
                .iter()
                .filter(|spawn| finished(spawn, &ended))
                .map(|spawn| spawn.id.clone())
                .collect(),
        });
    }
}

/// Whether each registration has ended here: it and every one after it in
/// its line.
fn ended(ledger: &Ledger) -> HashMap<&str, bool> {
    let by_id = by_id(ledger);
    ledger
        .agents
        .iter()
        .map(|agent| {
            (
                agent.id.as_str(),
                line(&by_id, agent).all(|record| record.ended),
            )
        })
        .collect()
}

fn by_id(ledger: &Ledger) -> HashMap<&str, &AgentRecord> {
    ledger
        .agents
        .iter()
        .map(|agent| (agent.id.as_str(), agent))
        .collect()
}

/// `first`'s line (`coordination::line`), found in `by_id`.
fn line<'a, 'm>(
    by_id: &'m HashMap<&'a str, &'a AgentRecord>,
    first: &'a AgentRecord,
) -> impl Iterator<Item = &'a AgentRecord> + 'm
where
    'a: 'm,
{
    super::line(first, by_id.len(), |id| by_id.get(id).copied())
}

fn finished(spawn: &SpawnRecord, ended: &HashMap<&str, bool>) -> bool {
    // A missing endpoint is never taken for finished.
    ended
        .get(spawn.child.as_deref().unwrap_or(&spawn.parent))
        .copied()
        .unwrap_or(false)
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

/// Keeps `ledger` within [`FILE_LIMIT`] after a change: a ledger past it
/// drops its finished letters, the one that finished first first, then its
/// oldest finished spawn records nothing keeps, then its oldest ended
/// registrations nothing names, until it fits, and answers what it dropped.
/// Letters go first: a finished letter only keeps history and a retried
/// send's answer for minutes, and weighs more than a finished spawn record
/// without its brief, whose loss makes a retried spawn start a new agent.
/// It is refused with `capacity`, unchanged, only when dropping all of them
/// would not be enough. The core's books are read only once the finished
/// letters are not enough, and a ledger within the limit is left as it is.
pub(crate) fn for_size(ledger: &mut Ledger, holds: &dyn Holds) -> Result<Option<Made>, String> {
    let size = ledger.size()?;
    let over = size.saturating_sub(FILE_LIMIT);
    if over == 0 {
        return Ok(None);
    }
    // A finished letter names no one, so letting it go keeps every record.
    let mut letters = Room::of(ledger.letters.len());
    for index in crate::delivery::mailbox::finished_oldest_first(ledger) {
        if letters.freed >= over {
            break;
        }
        let letter = &ledger.letters[index];
        letters.take(&letter.id, letter)?;
    }
    let mut spawns = Room::of(ledger.spawns.len());
    let mut agents = Room::of(ledger.agents.len());
    // The core's books are read only when the letters are not enough.
    if letters.freed < over {
        let named = Named::of(ledger, holds.held()?);
        for spawn in &ledger.spawns {
            if letters.freed + spawns.freed >= over {
                break;
            }
            if !named.spawns.contains(spawn.id.as_str()) {
                spawns.take(&spawn.id, spawn)?;
            }
        }
        // A registration goes only once every spawn record that can has
        // gone: one naming it is either kept, and so names it, or gone above.
        for agent in &ledger.agents {
            if letters.freed + spawns.freed + agents.freed >= over {
                break;
            }
            if agent.ended && !named.agents.contains(agent.id.as_str()) {
                agents.take(&agent.id, agent)?;
            }
        }
    }
    let freed = spawns.freed + letters.freed + agents.freed;
    if freed < over {
        crate::diagnostic!(json!({
            "component": "coordination", "kind": "ledger.full",
            "bytes": size, "agents": ledger.agents.len(),
            "ended": ledger.agents.iter().filter(|agent| agent.ended).count(),
            "spawns": ledger.spawns.len(), "letters": ledger.letters.len(),
            "open": ledger.letters.iter().filter(|letter| letter.open()).count(),
        }));
        return Err("capacity".into());
    }
    ledger
        .spawns
        .retain(|spawn| !spawns.gone.contains(&spawn.id));
    ledger
        .letters
        .retain(|letter| !letters.gone.contains(&letter.id));
    ledger
        .agents
        .retain(|agent| !agents.gone.contains(&agent.id));
    // The count above is exact; this raises if it ever is not.
    ledger.check()?;
    Ok(Some(Made {
        bytes: size,
        freed,
        spawns: spawns.gone.len(),
        letters: letters.gone.len(),
        agents: agents.gone.len(),
    }))
}

/// What [`for_size`] dropped: the size it found and the bytes it took off,
/// and how many records of each list went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Made {
    pub(crate) bytes: usize,
    pub(crate) freed: usize,
    pub(crate) spawns: usize,
    pub(crate) letters: usize,
    pub(crate) agents: usize,
}

/// Lets the brief and launch words of each completed spawn whose child
/// ended go, keeping the brief's digest (`SpawnRecord::shed_brief`): they
/// were most of what a spawn record weighs (with-hide/hide#2715). Nothing
/// reads them once the child ended: the brief is typed, confirmed and
/// carried into its first letter while the child runs, a mission node
/// keeps its own copy from its worker's binding, and a completed spawn
/// asked again answers its child before any start, so it only compares the
/// brief, which the digest does. Bounded by the spawn records, and by the
/// registrations once one carries anything, since a spawn's child is found
/// in one pass over them.
pub(crate) fn shed_finished_briefs(ledger: &mut Ledger) {
    let carrying: HashSet<&str> = ledger
        .spawns
        .iter()
        .filter(|spawn| spawn.completed && spawn.carries_brief())
        .filter_map(|spawn| spawn.child.as_deref())
        .collect();
    if carrying.is_empty() {
        return;
    }
    let ended: HashMap<&str, bool> = ledger
        .agents
        .iter()
        .filter(|agent| carrying.contains(agent.id.as_str()))
        .map(|agent| (agent.id.as_str(), agent.ended))
        .collect();
    let shed: Vec<usize> = ledger
        .spawns
        .iter()
        .enumerate()
        .filter(|(_, spawn)| spawn.completed && spawn.carries_brief() && finished(spawn, &ended))
        .map(|(index, _)| index)
        .collect();
    for index in shed {
        ledger.spawns[index].shed_brief();
    }
}

/// What dropping records from one of the ledger's lists takes off the file.
struct Room {
    /// How many records the list holds.
    len: usize,
    gone: HashSet<String>,
    freed: usize,
}

impl Room {
    fn of(len: usize) -> Self {
        Self {
            len,
            gone: HashSet::new(),
            freed: 0,
        }
    }

    /// Drops `record`: its encoding and the comma beside it, which the
    /// list's last record has none of.
    fn take(&mut self, id: &str, record: &impl serde::Serialize) -> Result<(), String> {
        self.gone.insert(id.to_owned());
        self.freed += encoded_len(record)? + usize::from(self.gone.len() < self.len);
        Ok(())
    }
}

/// The records that stay because something names them, read in one pass.
struct Named<'a> {
    agents: HashSet<&'a str>,
    /// The spawn records kept: unfinished, or named.
    spawns: HashSet<&'a str>,
}

impl<'a> Named<'a> {
    fn of(ledger: &'a Ledger, held: &'a Held) -> Self {
        let ended = ended(ledger);
        let by_id = by_id(ledger);
        let mut agents: HashSet<&str> = held.agents.iter().map(String::as_str).collect();
        for agent in &ledger.agents {
            if agent.ended && !ended[agent.id.as_str()] {
                agents.insert(&agent.id);
            }
            if held
                .published
                .as_ref()
                .is_some_and(|published| !published.agents.contains(&agent.id))
            {
                agents.insert(&agent.id);
            }
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
            let child_line = || {
                spawn
                    .child
                    .as_deref()
                    .and_then(|child| by_id.get(child))
                    .into_iter()
                    .flat_map(|child| line(&by_id, child))
            };
            let kept = !finished(spawn, &ended)
                || held
                    .published
                    .as_ref()
                    .is_some_and(|published| !published.spawns.contains(&spawn.id))
                || waited.contains(spawn.id.as_str())
                || spawn
                    .child
                    .as_ref()
                    .is_some_and(|child| held.agents.contains(child))
                || child_line()
                    .last()
                    .is_some_and(|last| held.agents.contains(&last.id));
            if kept {
                spawns.insert(spawn.id.as_str());
                agents.insert(&spawn.parent);
                agents.extend(spawn.child.as_deref());
                agents.extend(child_line().map(|record| record.id.as_str()));
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
pub(crate) mod tests {
    use super::*;
    use crate::coordination::human_turn::{HumanKind, HumanTurn};
    use crate::coordination::identity_wait::IdentityWait;
    use crate::coordination::tests::command;
    use crate::coordination::{Mutation, apply_with};
    use crate::delivery::BODY_LIMIT;
    use crate::delivery::ledger::{State, from_bytes};
    use serde_json::Value;

    const NOW: u64 = 1_000;

    pub(crate) fn actor(n: u64) -> Actor {
        Actor {
            pane_id: format!("w1:p{n}"),
            name: format!("a{n}"),
            kind: "codex".into(),
            device_id: "local".into(),
            session: crate::engine::tokens::session_digest(&format!("native-{n}")),
        }
    }

    pub(crate) fn agent(n: u64, ended: bool) -> AgentRecord {
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
            successor: None,
        }
    }

    /// A spawn record `spawn-{n}` of `parent`, with `child` bound when given.
    pub(crate) fn spawn(n: u64, parent: u64, child: Option<u64>) -> SpawnRecord {
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
            agents: (1..=AGENT_LIMIT as u64).map(|n| agent(n, n != 1)).collect(),
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
            assert_eq!(reloaded(ledger), *ledger);
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
            assert_eq!(reloaded(ledger), *ledger);
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

    /// A start whose brief is as long as a brief may be.
    pub(crate) fn briefed(intent: &str) -> crate::coordination::Command {
        let mut command = command(intent);
        if let crate::coordination::Command::Spawn { brief, .. } = &mut command {
            *brief = Some("n".repeat(BODY_LIMIT));
        }
        command
    }

    /// Adds spawn records of `parent`, `spawn-3000` on, each keeping a
    /// brief, until the file is `room` bytes short of its limit.
    pub(crate) fn fill_with_briefs(ledger: &mut Ledger, parent: &str, room: usize) {
        let mut size = ledger.size().unwrap();
        let target = FILE_LIMIT - room;
        for n in 3000.. {
            let mut record: SpawnRecord = serde_json::from_value(json!({
                "id": format!("spawn-{n}"), "parent": parent,
                "intent": format!("intent-{n}"), "name": "worker", "kind": "codex",
                "repo": "/repo", "branch": "topic", "requested_path": null, "args": [],
                "path": null, "pane": null, "child": null,
            }))
            .unwrap();
            record.brief = Some("b".repeat(BODY_LIMIT / 2));
            let grown =
                size + encoded_len(&record).unwrap() + usize::from(!ledger.spawns.is_empty());
            if grown > target {
                break;
            }
            size = grown;
            ledger.spawns.push(record);
        }
        // A brief's `b` is one byte in the file too.
        let mut need = target - size;
        for spawn in ledger.spawns.iter_mut().rev() {
            let brief = spawn.brief.as_mut().unwrap();
            let add = need.min(BODY_LIMIT - brief.len());
            brief.push_str(&"b".repeat(add));
            need -= add;
        }
        assert_eq!(need, 0);
        assert_eq!(ledger.size().unwrap(), target);
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
    fn both_full_collections_take_a_spawn_and_registration_without_dangling_references() {
        let mut ledger = full_of_agents();
        ledger.agents[1].parent = Some("agent-1".into());
        ledger.spawns = (3000..3000 + SPAWN_LIMIT as u64)
            .map(|n| spawn(n, 1, Some(2)))
            .collect();
        reserve(&mut ledger, &Held::default()).unwrap();
        assert_eq!(ledger.spawns.len(), SPAWN_LIMIT);
        assert!(!holds_spawn(&ledger, 3000));
        assert!(holds_agent(&ledger, 2));
        register(&mut ledger, &Held::default()).unwrap();
        assert_eq!(ledger.agents.len(), AGENT_LIMIT);
        assert!(!holds_agent(&ledger, 2));
        assert_eq!(ledger.spawns.len(), 1);
        assert_eq!(ledger.spawns[0].id, "spawn-100000");
    }

    #[test]
    fn records_not_yet_published_as_finished_wait_for_a_later_batch() {
        let mut ledger = full_of_agents();
        let mut held = Held::default();
        held.keep_published(&ledger);
        // A preceding request in this batch replaced the oldest record
        // and then ended the new one; it was never published as ended.
        ledger.agents[1] = agent(90_001, true);
        register(&mut ledger, &held).unwrap();
        assert!(holds_agent(&ledger, 90_001));
        assert!(!holds_agent(&ledger, 3));

        let mut ledger = full_of_spawns();
        held.keep_published(&ledger);
        ledger.spawns[0] = spawn(90_001, 2, None);
        reserve(&mut ledger, &held).unwrap();
        assert!(holds_spawn(&ledger, 90_001));
        assert!(!holds_spawn(&ledger, 3001));
    }

    #[test]
    fn a_full_ledger_of_live_registrations_refuses_a_new_one_with_capacity() {
        let mut ledger = full_of_agents();
        for agent in &mut ledger.agents {
            agent.ended = false;
        }
        let before = ledger.clone();
        assert_eq!(
            register(&mut ledger, &Held::default()).unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn an_ended_registration_another_names_as_parent_or_origin_stays() {
        let mut ledger = full_of_agents();
        // agent-10 was delegated by agent-2; agent-11 was forked from agent-3.
        ledger.agents[9].ended = false;
        ledger.agents[10].ended = false;
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
            published: None,
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
            published: None,
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
    fn a_ledger_past_its_file_limit_drops_its_oldest_finished_spawn_records_first() {
        // agent-2 made every spawn record and ended; agent-3 ended too.
        let mut ledger = Ledger {
            agents: vec![agent(1, false), agent(2, true), agent(3, true)],
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 100);
        let spawns = ledger.spawns.len();
        let answer = apply_with(
            &mut ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: briefed("new-start"),
            },
            NOW,
            &Held::default(),
        )
        .unwrap();
        assert!(ledger.size().unwrap() > FILE_LIMIT);
        for_size(&mut ledger, &Held::default()).unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        assert!(!holds_spawn(&ledger, 3000) && !holds_spawn(&ledger, 3001));
        assert!(holds_spawn(&ledger, 3002));
        assert_eq!(ledger.spawns.len(), spawns + 1 - 2);
        assert!(ledger.spawns.iter().any(|spawn| spawn.id == answer["id"]));
        // A registration goes only once no spawn record can.
        assert!((1..=3).all(|n| holds_agent(&ledger, n)));
        assert_eq!(reloaded(&ledger), ledger);
    }

    #[test]
    fn a_ledger_past_its_file_limit_of_kept_spawn_records_drops_ended_registrations_or_refuses() {
        // agent-2 runs, so none of its spawn records has finished.
        let mut ledger = Ledger {
            agents: (1..=8).map(|n| agent(n, n > 2)).collect(),
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 0);
        let spawns = ledger.spawns.len();
        // A new root registers itself in a file already at its limit.
        let mut record = agent(90_000, false);
        record.id = String::new();
        let mutation = Mutation::Register {
            record,
            check: false,
        };
        let answer = apply_with(
            &mut ledger,
            &actor(90_000),
            &mutation,
            NOW,
            &Held::default(),
        );
        for_size(&mut ledger, &Held::default()).unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        assert!(holds_agent(&ledger, 100_000) && answer.unwrap()["id"] == "agent-100000");
        assert!(!holds_agent(&ledger, 3) && holds_agent(&ledger, 8));
        assert_eq!(ledger.spawns.len(), spawns);

        let mut ledger = Ledger {
            agents: vec![agent(1, false), agent(2, false)],
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 100);
        apply_with(
            &mut ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: briefed("new-start"),
            },
            NOW,
            &Held::default(),
        )
        .unwrap();
        let before = ledger.clone();
        assert_eq!(
            for_size(&mut ledger, &Held::default()).unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger, before);
    }

    /// The live lead's letter to `agent-3`, as long as a body may be.
    fn long_letter(ledger: &mut Ledger) {
        crate::delivery::mailbox::send(
            ledger,
            &actor(1),
            &actor(3),
            "new-letter",
            &"l".repeat(BODY_LIMIT),
            "progress",
            None,
            NOW,
        )
        .unwrap();
    }

    /// Letter `n` from `agent-2` to the lead, `body` bytes long, read and
    /// finished at `NOW + n`.
    fn finished_letter(ledger: &mut Ledger, n: u64, body: usize) -> String {
        let sent = crate::delivery::mailbox::send(
            ledger,
            &actor(2),
            &actor(1),
            &format!("read-{n}"),
            &"r".repeat(body),
            "progress",
            None,
            NOW,
        )
        .unwrap();
        let letter = ledger
            .letters
            .iter_mut()
            .find(|letter| letter.id == sent.id)
            .unwrap();
        letter.state = State::Delivered;
        letter.hook_confirmed = Some(true);
        letter.finished_at_unix_ms = Some(NOW + n);
        sent.id
    }

    #[test]
    fn a_ledger_at_its_file_limit_of_finished_spawn_records_takes_a_new_letter() {
        let mut ledger = Ledger {
            agents: vec![agent(1, false), agent(2, true)],
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 100);
        let last = 3000 + ledger.spawns.len() as u64 - 1;
        long_letter(&mut ledger);
        let made = for_size(&mut ledger, &Held::default()).unwrap().unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        assert!(
            ledger
                .letters
                .iter()
                .any(|letter| letter.intent == "new-letter")
        );
        // The oldest go, only as many as the letter needs.
        assert!(!holds_spawn(&ledger, 3000) && holds_spawn(&ledger, last));
        assert!(made.spawns > 0 && made.spawns < 5);
        assert_eq!((made.letters, made.agents), (0, 0));
        assert_eq!(reloaded(&ledger), ledger);
    }

    #[test]
    fn a_ledger_past_its_file_limit_drops_finished_letters_before_spawn_records_and_registrations()
    {
        // agent-2 made every spawn record and ended, so they have finished;
        // agents 3 to 8 ended and nothing names them.
        let mut ledger = Ledger {
            agents: (1..=8).map(|n| agent(n, n > 1)).collect(),
            next_id: 100_000,
            ..Ledger::default()
        };
        let open = crate::delivery::mailbox::send(
            &mut ledger,
            &actor(1),
            &actor(2),
            "open",
            "still waiting to be read",
            "progress",
            None,
            NOW,
        )
        .unwrap()
        .id;
        // Sent in this order, finished the other way round: the one that
        // finished first goes first.
        let late = finished_letter(&mut ledger, 9, 9_000);
        let early = finished_letter(&mut ledger, 1, 9_000);
        let middle = finished_letter(&mut ledger, 5, 9_000);
        fill_with_briefs(&mut ledger, "agent-2", 100);
        let spawns = ledger.spawns.len();
        long_letter(&mut ledger);
        let made = for_size(&mut ledger, &Held::default()).unwrap().unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        let kept = |id: &str| ledger.letters.iter().any(|letter| letter.id == id);
        assert!(!kept(&early) && !kept(&middle));
        assert!(kept(&late) && kept(&open));
        assert_eq!(ledger.spawns.len(), spawns);
        assert!((1..=8).all(|n| holds_agent(&ledger, n)));
        assert_eq!((made.spawns, made.letters, made.agents), (0, 2, 0));
    }

    #[test]
    fn a_ledger_at_its_file_limit_whose_every_record_is_live_refuses_a_letter_unchanged() {
        // agent-2 runs, so its spawn records stay, and the one letter is
        // still to be read.
        let mut ledger = Ledger {
            agents: vec![agent(1, false), agent(2, false)],
            next_id: 100_000,
            ..Ledger::default()
        };
        crate::delivery::mailbox::send(
            &mut ledger,
            &actor(1),
            &actor(2),
            "open",
            "still waiting to be read",
            "progress",
            None,
            NOW,
        )
        .unwrap();
        fill_with_briefs(&mut ledger, "agent-2", 100);
        long_letter(&mut ledger);
        let before = ledger.clone();
        assert_eq!(
            for_size(&mut ledger, &Held::default()).unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn a_named_spawn_record_is_never_dropped_to_make_room_by_size() {
        let mut ledger = Ledger {
            agents: vec![
                agent(1, false),
                agent(2, true),
                AgentRecord {
                    parent: Some("agent-2".into()),
                    ..agent(4, true)
                },
            ],
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 100);
        // spawn-3000 is owed a registration, spawn-3001 is a plan step still
        // handed, and spawn-3002's child, which ended, is an open mission's
        // worker.
        ledger.spawns[2] = SpawnRecord {
            brief: ledger.spawns[2].brief.take(),
            ..spawn(3002, 2, Some(4))
        };
        ledger.identity_waits = vec![IdentityWait {
            id: "wait-9000".into(),
            machine: "local".into(),
            pane: "w1:p9000".into(),
            name: "waited".into(),
            kind: "codex".into(),
            owed: Owed::Spawn {
                spawn: "spawn-3000".into(),
            },
        }];
        let held = Held {
            published: None,
            agents: ["agent-4"].map(str::to_owned).into(),
            spawns: ["spawn-3001"].map(str::to_owned).into(),
        };
        apply_with(
            &mut ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: briefed("new-start"),
            },
            NOW,
            &held,
        )
        .unwrap();
        for_size(&mut ledger, &held).unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        assert!((3000..=3002).all(|n| holds_spawn(&ledger, n)));
        assert!(!holds_spawn(&ledger, 3003));
    }

    /// A mission's worker the next agent in its pane took over (`succeed`)
    /// keeps its registration and the spawn that started it while that agent
    /// works, so the mission still counts the work against its cap; once
    /// that agent ends too, they stay while the open mission holds it, and
    /// go with it after. The size pass keeps them alike.
    #[test]
    fn a_taken_over_workers_spawn_stays_while_its_line_works_or_is_held() {
        let mut ledger = full_of_agents();
        // spawn-3000 started agent-2 for mission-1; agent-3 took its pane
        // over and works.
        ledger.agents[1].parent = Some("agent-1".into());
        ledger.agents[1].successor = Some("agent-3".into());
        ledger.agents[2].ended = false;
        ledger.spawns = vec![SpawnRecord {
            mission: Some("mission-1".into()),
            ..spawn(3000, 1, Some(2))
        }];
        register(&mut ledger, &Held::default()).unwrap();
        assert!(holds_agent(&ledger, 2) && holds_spawn(&ledger, 3000));
        assert!(
            !holds_agent(&ledger, 4),
            "the oldest ended one nothing names went"
        );
        assert_eq!(
            crate::coordination::mission_workers(&ledger, "mission-1", &[], NOW),
            1
        );

        // agent-3 ended as well; the open mission holds it, since its roles
        // follow the line.
        let at = |ledger: &Ledger| ledger.agents.iter().position(|agent| agent.id == "agent-3");
        let index = at(&ledger).unwrap();
        ledger.agents[index].ended = true;
        let held = Held {
            agents: ["agent-3"].map(str::to_owned).into(),
            ..Held::default()
        };
        register_agent(&mut ledger, 90_001, &held).unwrap();
        assert!(holds_agent(&ledger, 2) && holds_agent(&ledger, 3) && holds_spawn(&ledger, 3000));
        register_agent(&mut ledger, 90_002, &Held::default()).unwrap();
        assert!(!holds_agent(&ledger, 2) && !holds_spawn(&ledger, 3000));

        let mut ledger = Ledger {
            agents: vec![
                agent(1, false),
                agent(2, true),
                AgentRecord {
                    parent: Some("agent-1".into()),
                    successor: Some("agent-6".into()),
                    ..agent(5, true)
                },
                agent(6, false),
            ],
            spawns: vec![SpawnRecord {
                mission: Some("mission-1".into()),
                brief: Some(String::new()),
                ..spawn(2999, 1, Some(5))
            }],
            next_id: 100_000,
            ..Ledger::default()
        };
        fill_with_briefs(&mut ledger, "agent-2", 100);
        apply_with(
            &mut ledger,
            &actor(1),
            &Mutation::Reserve {
                parent: "agent-1".into(),
                command: briefed("new-start"),
            },
            NOW,
            &Held::default(),
        )
        .unwrap();
        for_size(&mut ledger, &Held::default()).unwrap();
        assert!(ledger.size().unwrap() <= FILE_LIMIT);
        assert!(
            !holds_spawn(&ledger, 3000),
            "a finished spawn made the room"
        );
        assert!(holds_spawn(&ledger, 2999) && holds_agent(&ledger, 5));
        assert_eq!(
            crate::coordination::mission_workers(&ledger, "mission-1", &[], NOW),
            1
        );
    }

    #[test]
    fn a_completed_spawn_whose_child_ended_lets_its_brief_go_and_still_answers_its_intent() {
        let brief = "Fix the flaky test, then report.";
        let mut ledger = Ledger {
            agents: vec![
                agent(1, false),
                AgentRecord {
                    parent: Some("agent-1".into()),
                    ..agent(2, true)
                },
                AgentRecord {
                    parent: Some("agent-1".into()),
                    ..agent(3, false)
                },
            ],
            next_id: 100_000,
            ..Ledger::default()
        };
        for (n, child) in [(3000, 2), (3001, 3)] {
            ledger.spawns.push(SpawnRecord {
                brief: Some(brief.into()),
                launch_words: vec!["--prompt".into(), brief.into()],
                ..spawn(n, 1, Some(child))
            });
        }
        let before = ledger.size().unwrap();
        shed_finished_briefs(&mut ledger);
        // The ended child's spawn keeps neither; the running one's keeps both.
        assert_eq!(ledger.spawns[0].brief, None);
        assert!(ledger.spawns[0].launch_words.is_empty());
        assert_eq!(ledger.spawns[1].brief.as_deref(), Some(brief));
        assert_eq!(ledger.spawns[1].launch_words.len(), 2);
        assert!(ledger.size().unwrap() < before);
        assert_eq!(reloaded(&ledger), ledger);
        // Asked again with the same brief, it is the same completed spawn;
        // with another, a conflict.
        let again = |text: &str| {
            let mut command = command("intent-3000");
            if let crate::coordination::Command::Spawn {
                brief, repo, args, ..
            } = &mut command
            {
                *brief = Some(text.into());
                *repo = Some("/repo".into());
                args.clear();
            }
            Mutation::Reserve {
                parent: "agent-1".into(),
                command,
            }
        };
        let answer =
            apply_with(&mut ledger, &actor(1), &again(brief), NOW, &Held::default()).unwrap();
        assert_eq!(answer["id"], "spawn-3000");
        assert_eq!(answer["completed"], true);
        assert_eq!(answer["child"], "agent-2");
        assert_eq!(
            apply_with(
                &mut ledger,
                &actor(1),
                &again("Another task."),
                NOW,
                &Held::default()
            )
            .unwrap_err(),
            "intent_conflict"
        );
    }

    #[test]
    fn dropped_names_what_a_batch_removed() {
        let before = full_of_agents();
        let mut after = before.clone();
        register(&mut after, &Held::default()).unwrap();
        let (agents, spawns) = dropped(&before, &after);
        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            ["agent-2"]
        );
        assert!(spawns.is_empty());
    }
}
