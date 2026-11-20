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

/// B16, B17, B20: after a restart, the resumed agents that were at work
/// and are a child or a mission's Conductor are each owed the one line, in
/// the operator's language and the same whatever cut them, on this machine
/// and on a device whose link ended; the operator's own agent and one at
/// rest are typed nothing, and an agent whose holder kept it is never cut.
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
    let device_pane = crate::address::scoped_pane_id(DEVICE, "w1:p5");
    {
        let mut facts = runtime.hook_facts.lock().unwrap();
        let mut working = hide_node_link::panes::HookStatusReport {
            runtime: "claude-code".into(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: Some("s-5".into()),
            turn: Some(hide_node_link::panes::HookTurn::Working),
            counts: None,
            subagents: None,
            session_start: false,
            waiting: None,
            account: None,
            refused: None,
        };
        facts.record(&device_pane, &working, 1).unwrap();
        facts.engine_ended(|pane| crate::address::is_pane_of(pane, DEVICE));
        // The resumed agent's first report has no turn running yet.
        working.session_start = true;
        working.turn = None;
        facts.record(&device_pane, &working, 2).unwrap();
    }

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
        [device_pane.as_str(), "w1:p1", "w1:p2", "w1:p3"],
        "only an agent at work that a heal resumed is owed"
    );
    assert!(
        !runtime.hook_facts.lock().unwrap().take_cut("w1:p6"),
        "a report with no heal behind it ends the cut"
    );

    let child = actor(&node, "w1:p1", "s-1");
    let own = actor(&node, "w1:p2", "s-2");
    let conductor = actor(&node, "w1:p3", "s-3");
    let device_child = actor(DEVICE, &device_pane, "s-5");
    let mut ledger = crate::delivery::ledger::Ledger::default();
    for (id, actor, parent) in [
        ("agent-1", &child, Some("agent-9")),
        ("agent-2", &own, None),
        ("agent-3", &conductor, None),
        ("agent-5", &device_child, Some("agent-3")),
    ] {
        ledger.agents.push(record(id, actor, parent));
    }
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let mut mission = crate::missions::tests::mission();
    mission.status = crate::missions::MissionStatus::Running;
    mission.conductor.actor = conductor.clone();
    mission.conductor.agent_id = Some("agent-3".into());
    runtime.missions.book.missions.push(mission);

    // Only the agents seen in their panes are settled.
    for actor in [&child, &own] {
        runtime.delivery_observations.insert(
            actor.pane_id.clone(),
            super::super::agent_sleep::letters::asleep_observation(actor.clone()),
        );
    }
    let told = |notes: Vec<super::super::hide_notes::Note>| -> Vec<(String, String)> {
        notes
            .into_iter()
            .map(|note| (note.target.actor.pane_id, note.body))
            .collect()
    };
    let line = "끊겼던 일을 이어서 하세요.".to_owned();
    assert_eq!(
        told(runtime.heal_notes_due(2_000)),
        [("w1:p1".to_owned(), line.clone())],
        "the operator's own agent is typed nothing"
    );
    assert_eq!(runtime.heal_letters_owed(), [device_pane.as_str(), "w1:p3"]);

    for actor in [&conductor, &device_child] {
        runtime.delivery_observations.insert(
            actor.pane_id.clone(),
            super::super::agent_sleep::letters::asleep_observation(actor.clone()),
        );
    }
    assert_eq!(
        told(runtime.heal_notes_due(3_000)),
        [
            (device_pane.clone(), line.clone()),
            ("w1:p3".to_owned(), line),
        ]
    );
    assert!(runtime.heal_letters_owed().is_empty());
}

/// An agent that never shows in its pane after its heal is given up on
/// rather than kept owed.
#[test]
fn a_healed_agent_that_never_shows_is_given_up_on() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 1_000);
    assert!(runtime.heal_notes_due(2_000).is_empty());
    assert_eq!(runtime.heal_letters_owed(), ["w1:p1"]);
    assert!(
        runtime
            .heal_notes_due(1_000 + 10 * 60 * 1000 + 1)
            .is_empty()
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
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    runtime.note_healing(&node, &[]);
    runtime.hook_after_heal(&node, "w1:p1", "s-1", true, 1_000);
    assert!(runtime.healing(&worker));
    runtime.delivery_observations.insert(
        "w1:p1".into(),
        super::super::agent_sleep::letters::asleep_observation(worker.actor.clone()),
    );
    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger.agents.push(worker.clone());
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(runtime.heal_notes_due(2_000).len(), 1);
    assert!(!runtime.healing(&worker));
}
