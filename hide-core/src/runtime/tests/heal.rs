//! The heal after a cut (PRD letters-missions-simplify D-06, D-18, D-19,
//! B16 to B20), as the core decides it: which resumed agents are owed the
//! one line, in which words, and which nodes it keeps from warning while
//! the heal runs. The engine's half, that every cut agent is resumed and
//! answered as healed once, is `engine/own` `a_reboot_resumes_every_cut_agent_*`
//! and `typed_agent_goes_on_after_a_restart` (this machine and a device).

use std::sync::Arc;

use super::*;
use crate::agent_hooks::facts::HookFacts;
use crate::coordination::AgentRecord;
use crate::delivery::Actor;

const DEVICE: &str = "mini";

/// The agent in `pane` on `device`, running `session`.
fn actor(device: &str, pane: &str, session: &str) -> Actor {
    Actor {
        pane_id: pane.into(),
        name: format!("agent {pane}"),
        kind: "claude".into(),
        device_id: device.into(),
        session: Some(session.into()),
    }
}

fn record(id: &str, actor: &Actor, parent: Option<&str>) -> AgentRecord {
    AgentRecord {
        id: id.into(),
        name: Some(actor.name.clone()),
        machine: actor.device_id.clone(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: actor.session.clone().unwrap_or_default(),
        instance: actor.pane_id.clone(),
        pane: actor.pane_id.clone(),
        parent: parent.map(Into::into),
        origin: None,
        project: None,
        actor: actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    }
}

/// The facts the core reads at its start: each of this machine's panes
/// with the session that reported last, at work or not.
fn started_with(runtime: &mut Runtime, root: &std::path::Path, panes: &[(&str, &str, bool)]) {
    let panes: serde_json::Map<String, serde_json::Value> = panes
        .iter()
        .map(|(pane, session, busy)| {
            (
                (*pane).to_owned(),
                serde_json::json!({"seen_agent": true, "sessions": [{
                    "runtime": "claude-code", "version": hide_agent_hooks::HOOK_VERSION,
                    "session": session, "busy": busy,
                }]}),
            )
        })
        .collect();
    let path = root.join("hook-facts.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version": 1, "panes": panes})).unwrap(),
    )
    .unwrap();
    *runtime.hook_facts.lock().unwrap() = HookFacts::restore(path, Vec::new);
}

/// `record` with its pane named by `pane`, the id its own machine gives it.
fn record_in(id: &str, actor: &Actor, pane: &str, parent: Option<&str>) -> AgentRecord {
    AgentRecord {
        pane: pane.into(),
        ..record(id, actor, parent)
    }
}

/// Registers `records` with the core.
fn registered(runtime: &mut Runtime, records: Vec<AgentRecord>) {
    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger.agents = records;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
}

/// Shows `actor` in its pane.
fn observe(runtime: &mut Runtime, actor: &Actor) {
    runtime.delivery_observations.insert(
        actor.pane_id.clone(),
        super::super::agent_sleep::letters::asleep_observation(actor.clone()),
    );
}

/// Who each note goes to and what it says.
fn told(notes: Vec<super::super::hide_notes::Note>) -> Vec<(String, String)> {
    notes
        .into_iter()
        .map(|note| (note.target.actor.pane_id, note.body))
        .collect()
}

/// The ledger admits every note due at `now`, and answers what they were.
fn admit(runtime: &mut Runtime, now: u64) -> Vec<(String, String)> {
    let notes = runtime.heal_notes_due(now);
    let keys: Vec<String> = notes.iter().map(|note| note.letter.clone()).collect();
    runtime.heal_lines_admitted(&keys);
    told(notes)
}

/// `pane` on `device` reports a turn at work, then its device's link ends.
fn cut_on_device(runtime: &Runtime, device: &str, pane: &str, session: &str) {
    let mut facts = runtime.hook_facts.lock().unwrap();
    let mut working = hide_node_link::panes::HookStatusReport {
        runtime: "claude-code".into(),
        version: hide_agent_hooks::HOOK_VERSION,
        session: Some(session.into()),
        turn: Some(hide_node_link::panes::HookTurn::Working),
        counts: None,
        subagents: None,
        session_start: false,
        waiting: None,
        account: None,
        refused: None,
    };
    let scoped = crate::address::scoped_pane_id(device, pane);
    facts.record(&scoped, &working, 1).unwrap();
    facts.engine_ended(|pane| crate::address::is_pane_of(pane, device));
    // The resumed agent's first report has no turn running yet.
    working.session_start = true;
    working.turn = None;
    facts.record(&scoped, &working, 2).unwrap();
}

/// B16, B17, B18, B20: after a restart, the resumed agents that were at
/// work and are a child or a mission's Conductor are each owed the one
/// line, in the operator's language and the same whatever cut them, on this
/// machine and on a device whose link ended; the operator's own agent and
/// one at rest are typed nothing, and an agent whose holder kept it is never
/// cut. A line the ledger has not admitted is sent again, and once the heal
/// settles each parent hears once how many of its children came back.
#[test]
fn a_healed_agent_at_work_is_told_to_go_on_only_when_it_is_a_child_or_a_participant() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    runtime.snapshot.ui_state.interface_language = Some(serde_json::json!("ko"));
    started_with(
        &mut runtime,
        root.path(),
        &[
            ("w1:p1", "s-1", true),  // a child at work
            ("w1:p2", "s-2", true),  // the operator's own, at work
            ("w1:p3", "s-3", true),  // a mission's Conductor, at work
            ("w1:p4", "s-4", false), // a child at rest
            ("w1:p6", "s-6", true),  // a child its holder kept
        ],
    );
    // A device's child was at work when the device's link ended.
    cut_on_device(&runtime, DEVICE, "w1:p5", "s-5");
    let device_pane = crate::address::scoped_pane_id(DEVICE, "w1:p5");

    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    let own = actor(&node, "w1:p2", "s-2");
    let conductor = actor(&node, "w1:p3", "s-3");
    let resting = actor(&node, "w1:p4", "s-4");
    let device_child = actor(DEVICE, &device_pane, "s-5");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &child, Some("agent-9")),
            record("agent-2", &own, None),
            record("agent-3", &conductor, None),
            record("agent-4", &resting, Some("agent-9")),
            record_in("agent-5", &device_child, "w1:p5", Some("agent-3")),
        ],
    );
    let mut mission = crate::missions::tests::mission();
    mission.status = crate::missions::MissionStatus::Running;
    mission.conductor.actor = conductor.clone();
    mission.conductor.agent_id = Some("agent-3".into());
    runtime.missions.book.missions.push(mission);

    for (device, pane, session, healed) in [
        (node.as_str(), "w1:p1", "s-1", true),
        (node.as_str(), "w1:p2", "s-2", true),
        (node.as_str(), "w1:p3", "s-3", true),
        (node.as_str(), "w1:p4", "s-4", true),
        (node.as_str(), "w1:p6", "s-6", false),
        (DEVICE, "w1:p5", "s-5", true),
    ] {
        runtime.hook_after_heal(device, pane, session, healed, 1_000);
    }
    assert_eq!(
        runtime.heal_letters_owed(),
        [
            format!("go-on:{device_pane}:s-5:1000"),
            "go-on:w1:p1:s-1:1000".to_owned(),
            "go-on:w1:p3:s-3:1000".to_owned(),
        ],
        "only an agent at work that a heal resumed is owed, the operator's own never"
    );
    {
        let facts = runtime.hook_facts.lock().unwrap();
        assert!(
            !facts.cut_pending("w1:p2"),
            "the operator's own cut is spent"
        );
        assert!(
            !facts.cut_pending("w1:p6"),
            "a report with no heal ends the cut"
        );
    }

    // Only the agents seen in their panes are sent the line, and a line
    // the ledger did not admit is due again.
    for actor in [&child, &own, &parent] {
        observe(&mut runtime, actor);
    }
    let line = "끊겼던 일을 이어서 하세요.".to_owned();
    assert_eq!(
        told(runtime.heal_notes_due(2_000)),
        [("w1:p1".to_owned(), line.clone())]
    );
    assert_eq!(
        told(runtime.heal_notes_due(2_500)),
        [("w1:p1".to_owned(), line.clone())]
    );
    assert_eq!(
        admit(&mut runtime, 3_000),
        [("w1:p1".to_owned(), line.clone())]
    );

    for actor in [&conductor, &device_child] {
        observe(&mut runtime, actor);
    }
    assert_eq!(
        admit(&mut runtime, 4_000),
        [
            (device_pane.clone(), line.clone()),
            ("w1:p3".to_owned(), line)
        ]
    );
    assert!(runtime.heal_letters_owed().is_empty());

    // The heal settles: each parent hears once how many came back, the
    // child at rest included.
    assert_eq!(
        admit(&mut runtime, 6_000),
        [
            (
                "w1:p3".to_owned(),
                format!("되살림: 1명 · agent {device_pane}")
            ),
            (
                "w1:p9".to_owned(),
                "되살림: 2명 · agent w1:p1, agent w1:p4".to_owned()
            ),
        ]
    );
    assert!(admit(&mut runtime, 20_000).is_empty());
}

/// B17: a second cut of the same conversation is a new line, not the first
/// one sent again; a retry of the one heal converges on its line.
#[test]
fn a_second_cut_of_the_same_conversation_is_told_again() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let child = actor(&node, "w1:p1", "s-1");
    registered(
        &mut runtime,
        vec![record("agent-1", &child, Some("agent-9"))],
    );
    observe(&mut runtime, &child);

    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 1_000);
    let first = runtime.heal_notes_due(2_000);
    let first: Vec<String> = first.into_iter().map(|note| note.letter).collect();
    assert_eq!(first, ["go-on:w1:p1:s-1:1000"]);
    runtime.heal_lines_admitted(&first);

    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 50_000);
    let second: Vec<String> = runtime
        .heal_notes_due(51_000)
        .into_iter()
        .map(|note| note.letter)
        .collect();
    assert_eq!(second, ["go-on:w1:p1:s-1:50000"]);
}

/// B17, B20: two devices each bring back more agents at work than one
/// machine's holders, and every one is owed its line: the bound is each
/// machine's, never one for the whole core.
#[test]
fn every_device_owes_its_own_agents_their_line() {
    let mut runtime = runtime();
    let mut records = Vec::new();
    let panes = hide_kit::panes::HOLDER_CAP + 2;
    for device in ["mini", "studio"] {
        for at in 0..panes {
            let pane = format!("w1:p{at}");
            cut_on_device(&runtime, device, &pane, "s");
            let scoped = crate::address::scoped_pane_id(device, &pane);
            let actor = actor(device, &scoped, "s");
            records.push(record_in(
                &format!("{device}-{at}"),
                &actor,
                &pane,
                Some("agent-9"),
            ));
        }
    }
    registered(&mut runtime, records);
    for device in ["mini", "studio"] {
        for at in 0..panes {
            runtime.hook_after_heal(device, &format!("w1:p{at}"), "s", true, 1_000);
        }
    }
    assert_eq!(runtime.heal_letters_owed().len(), 2 * panes);
}

/// B18: a child that came back but never showed to be told is said to its
/// parent once the heal gives its line up; a child its engine could not
/// bring back is said by name, unless it is a mission's worker, whose
/// Conductor is raised `worker_ended` already.
#[test]
fn a_parent_hears_of_a_child_the_heal_could_not_bring_back_or_tell() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    let lost = actor(&node, "w1:p2", "s-2");
    let worker = actor(&node, "w1:p3", "s-3");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &child, Some("agent-9")),
            record("agent-2", &lost, Some("agent-9")),
            record("agent-3", &worker, Some("agent-9")),
        ],
    );
    let mut mission = crate::missions::apply_plan(
        &crate::missions::tests::mission(),
        &crate::missions::tests::plan(vec![crate::missions::tests::node(
            "a",
            crate::missions::NodeKind::Change,
            &[],
        )]),
        2,
    )
    .unwrap();
    mission.nodes[0].agent_id = Some("agent-3".into());
    runtime.missions.book.missions.push(mission);
    observe(&mut runtime, &parent);

    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 1_000);
    runtime.heal_not_woken(&["agent-2".to_owned(), "agent-3".to_owned()], 1_000);
    // The count waits for the heal to settle; the child is not seen.
    assert_eq!(
        admit(&mut runtime, 2_000),
        [("w1:p9".to_owned(), "Could not wake agent w1:p2".to_owned())]
    );
    assert_eq!(
        admit(&mut runtime, 7_000),
        [("w1:p9".to_owned(), "Resumed: 1 · agent w1:p1".to_owned())]
    );
    assert_eq!(runtime.heal_letters_owed(), ["go-on:w1:p1:s-1:1000"]);
    let late = 1_000 + 10 * 60 * 1000 + 1;
    assert_eq!(
        admit(&mut runtime, late),
        [(
            "w1:p9".to_owned(),
            "Could not tell agent w1:p1 to carry on".to_owned()
        )]
    );
    assert!(runtime.heal_letters_owed().is_empty());
}

/// B19: a node's worker is healing while its engine names its pane so and
/// while its line is owed, and not after.
#[test]
fn a_worker_heals_while_its_engine_names_it_and_while_its_line_is_owed() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let worker = record("agent-1", &actor(&node, "w1:p1", "s-1"), Some("agent-9"));
    let device_worker = record("agent-2", &actor(DEVICE, "w1:p1", "s-1"), Some("agent-9"));
    assert!(!runtime.healing(&worker));

    runtime.note_healing(&node, &["w1:p1".to_owned()]);
    assert!(runtime.healing(&worker));
    assert!(!runtime.healing(&device_worker), "another machine's pane");
    runtime.note_healing(DEVICE, &["w1:p1".to_owned()]);
    assert!(runtime.healing(&device_worker));

    // Back in its pane, it is still healing until it is told to go on.
    registered(&mut runtime, vec![worker.clone()]);
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.note_healing(&node, &[]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 1_000);
    assert!(runtime.healing(&worker));
    observe(&mut runtime, &worker.actor);
    assert_eq!(admit(&mut runtime, 2_000).len(), 1);
    assert!(!runtime.healing(&worker));
}
