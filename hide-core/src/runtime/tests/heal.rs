//! The heal after a cut (PRD letters-missions-simplify D-06, D-18, D-19,
//! B16 to B20), as the core decides it: which resumed agents are owed the
//! one line, in which words, what each parent hears, and which nodes it
//! keeps from warning while the heal runs. The engine's half, that every cut agent is resumed and
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
        successor: None,
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
    runtime.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::Ledger {
        agents: records,
        ..Default::default()
    }));
}

/// The engine on `device` resumes the agents saved in `panes` and says so,
/// and names those panes healing.
fn heal(runtime: &mut Runtime, device: &str, panes: &[String], now: u64) {
    let resumed: Vec<crate::engine::ResumedPane> = panes
        .iter()
        .map(|pane| crate::engine::ResumedPane {
            pane: pane.clone(),
            saved: pane.clone(),
        })
        .collect();
    runtime.heal_resumed(device, &resumed, now);
    runtime.note_healing(device, panes, now);
}

/// Shows `actor` in its pane.
fn observe(runtime: &mut Runtime, actor: &Actor) {
    runtime.delivery_observations.insert(
        actor.pane_id.clone(),
        super::super::agent_sleep::letters::asleep_observation(actor.clone()),
    );
}

/// Who each note due goes to and what it says, with nothing admitted.
fn due(runtime: &mut Runtime) -> Vec<(String, String)> {
    runtime
        .heal_notes_due()
        .into_iter()
        .map(|note| (note.target.actor.pane_id, note.body))
        .collect()
}

/// The ledger admits every note due, and answers what they were.
fn admit(runtime: &mut Runtime) -> Vec<(String, String)> {
    let notes = runtime.heal_notes_due();
    let keys: Vec<String> = notes.iter().map(|note| note.letter.clone()).collect();
    runtime.heal_lines_admitted(&keys);
    notes
        .into_iter()
        .map(|note| (note.target.actor.pane_id, note.body))
        .collect()
}

/// `pane` on `device` reported a turn at work, then its device's link ended.
fn cut_on_device(runtime: &Runtime, device: &str, pane: &str, session: &str) {
    let mut facts = runtime.hook_facts.lock().unwrap();
    let working = hide_node_link::panes::HookStatusReport {
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
        asked: None,
    };
    let scoped = crate::address::scoped_pane_id(device, pane);
    facts.record(&scoped, &working, 1).unwrap();
    facts.engine_ended(|pane| crate::address::is_pane_of(pane, device));
}

/// A mission running with one node, whose worker is `worker`.
fn mission_with_worker(worker: &str) -> crate::missions::Mission {
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
    mission.nodes[0].agent_id = Some(worker.into());
    mission
}

/// B16, B17, B18, B20: when the engine starts healing after a restart, the
/// registrations that were at work and are a child or a mission's Conductor
/// are each owed the one line, on this machine and on a device whose link
/// ended, before any of them reports: in the operator's language and the
/// same whatever cut them. The operator's own agent and one at rest are
/// typed nothing. A line waits for its agent in its pane and is sent again
/// until the ledger admits it; once the heal settles each parent hears once
/// how many of its children came back.
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
    cut_on_device(&runtime, DEVICE, "w1:p5", "s-5");
    let device_pane = crate::address::scoped_pane_id(DEVICE, "w1:p5");

    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    let own = actor(&node, "w1:p2", "s-2");
    let conductor = actor(&node, "w1:p3", "s-3");
    let resting = actor(&node, "w1:p4", "s-4");
    let kept = actor(&node, "w1:p6", "s-6");
    let device_child = actor(DEVICE, &device_pane, "s-5");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &child, Some("agent-9")),
            record("agent-2", &own, None),
            record("agent-3", &conductor, None),
            record("agent-4", &resting, Some("agent-9")),
            record("agent-6", &kept, Some("agent-9")),
            record_in("agent-5", &device_child, "w1:p5", Some("agent-3")),
        ],
    );
    let mut mission = crate::missions::tests::mission();
    mission.status = crate::missions::MissionStatus::Running;
    mission.conductor.actor = conductor.clone();
    mission.conductor.agent_id = Some("agent-3".into());
    runtime.missions.book.missions.push(mission);

    let healing: Vec<String> = ["w1:p1", "w1:p2", "w1:p3", "w1:p4"]
        .map(str::to_owned)
        .to_vec();
    heal(&mut runtime, &node, &healing, 1_000);
    heal(&mut runtime, DEVICE, &["w1:p5".to_owned()], 1_000);
    // The holder kept its agent: it reports with no heal behind it.
    runtime.hook_after_heal(&node, "w1:p6", false);
    assert_eq!(
        runtime.heal_letters_owed(),
        [
            "go-on:agent-1:1000",
            "go-on:agent-3:1000",
            "go-on:agent-5:1000"
        ],
        "owed at the heal's start, before any agent reported"
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
    assert!(
        due(&mut runtime).is_empty(),
        "no agent is back in its pane yet"
    );

    for actor in [&child, &own, &parent] {
        observe(&mut runtime, actor);
    }
    let line = "끊겼던 일을 이어서 하세요.".to_owned();
    assert_eq!(due(&mut runtime), [("w1:p1".to_owned(), line.clone())]);
    assert_eq!(
        due(&mut runtime),
        [("w1:p1".to_owned(), line.clone())],
        "a line the ledger did not admit is due again"
    );
    assert_eq!(admit(&mut runtime), [("w1:p1".to_owned(), line.clone())]);

    for actor in [&conductor, &device_child] {
        observe(&mut runtime, actor);
    }
    // The Conductor's one child reported after its resume, so its count
    // goes too; the child at rest is still healing, so its parent's waits.
    runtime.hook_after_heal(DEVICE, "w1:p5", true);
    assert_eq!(
        admit(&mut runtime),
        [
            (
                "w1:p3".to_owned(),
                format!("되살림: 1명 · agent {device_pane}")
            ),
            ("w1:p3".to_owned(), line.clone()),
            (device_pane.clone(), line),
        ]
    );
    assert!(runtime.heal_letters_owed().is_empty());
    runtime.note_healing(&node, &[], 2_000);
    assert_eq!(
        admit(&mut runtime),
        [(
            "w1:p9".to_owned(),
            "되살림: 2명 · agent w1:p1, agent w1:p4".to_owned()
        )]
    );
    assert!(admit(&mut runtime).is_empty(), "once per heal");
}

/// B17: a second cut of the same conversation is a new line, not the first
/// one again.
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
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 1_000);
    assert_eq!(runtime.heal_letters_owed(), ["go-on:agent-1:1000"]);
    admit(&mut runtime);
    runtime.note_healing(&node, &[], 2_000);

    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 50_000);
    assert_eq!(runtime.heal_letters_owed(), ["go-on:agent-1:50000"]);
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
        let healing: Vec<String> = (0..panes).map(|at| format!("w1:p{at}")).collect();
        heal(&mut runtime, device, &healing, 1_000);
    }
    assert_eq!(runtime.heal_letters_owed().len(), 2 * panes);
}

/// B17, B18: a line is owed to a registration, not to a pane. When the
/// child ends before its line could go and the operator's own agent takes
/// its pane, the new agent is typed nothing and the parent hears the child
/// could not be told.
#[test]
fn a_line_never_goes_to_another_agent_that_took_the_pane() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    let taker = actor(&node, "w1:p1", "s-7");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &child, Some("agent-9")),
        ],
    );
    observe(&mut runtime, &parent);
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 1_000);

    // The child's registration ends; the operator's own agent takes the pane.
    let mut ended = record("agent-1", &child, Some("agent-9"));
    ended.ended = true;
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            ended,
            record("agent-7", &taker, None),
        ],
    );
    observe(&mut runtime, &taker);
    runtime.note_healing(&node, &[], 2_000);
    assert_eq!(
        admit(&mut runtime),
        [(
            "w1:p9".to_owned(),
            "Could not tell agent w1:p1 to carry on".to_owned()
        )]
    );
    assert!(runtime.heal_letters_owed().is_empty());
}

/// B18: a child its engine could not bring back is said to its parent by
/// name and left out of the count, unless it is a mission's worker, whose
/// Conductor is raised `worker_ended` already; a line to a parent not
/// reachable yet stays owed, whatever time passes.
#[test]
fn a_parent_hears_of_a_child_the_heal_could_not_bring_back() {
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
    runtime
        .missions
        .book
        .missions
        .push(mission_with_worker("agent-3"));
    let healing: Vec<String> = ["w1:p1", "w1:p2", "w1:p3"].map(str::to_owned).to_vec();
    heal(&mut runtime, &node, &healing, 1_000);
    runtime.heal_not_woken(&["agent-2".to_owned(), "agent-3".to_owned()]);
    observe(&mut runtime, &child);

    // The parent is not in its pane: nothing goes, nothing is dropped.
    runtime.note_healing(&node, &[], 2_000);
    assert!(admit(&mut runtime).is_empty());
    runtime.drive_heal_letters(1_000 + 24 * 60 * 60 * 1000);
    // One entry for the parent, with the child that came back and the one
    // that could not; the worker is its Conductor's `worker_ended`.
    let owed = runtime.heal_letters_owed();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert!(owed[0].starts_with("children:agent-9:") && owed[0].ends_with(":2"));
    observe(&mut runtime, &parent);
    assert_eq!(
        admit(&mut runtime),
        [(
            "w1:p9".to_owned(),
            "Resumed: 1 · agent w1:p1\nCould not wake agent w1:p2".to_owned()
        )]
    );
}

/// B2, B17, B18, PRD letters-missions-simplify D-24: the agent that took a
/// mission worker's or checker's pane over answers to the Conductor the one
/// before it answered to (`coordination::succeed` gives it the parent): the
/// worker at work is told to go on and is counted back in the Conductor's
/// one cell, and a checker its engine could not bring back is named there.
#[test]
fn a_mission_participants_successor_is_healed_as_its_conductors_child() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    started_with(
        &mut runtime,
        root.path(),
        &[("w1:p1", "s-18", true), ("w1:p7", "s-7", true)],
    );
    let conductor = actor(&node, "w1:p3", "s-3");
    let worker = actor(&node, "w1:p1", "s-18");
    let checker = actor(&node, "w1:p7", "s-7");
    // A worker and a checker the Conductor started, each followed in its
    // pane by a new conversation the Conductor never started.
    let mut ledger = crate::delivery::ledger::Ledger {
        next_id: 18,
        agents: vec![
            record("agent-3", &conductor, None),
            record("agent-1", &actor(&node, "w1:p1", "s-1"), Some("agent-3")),
            record("agent-5", &actor(&node, "w1:p7", "s-5"), Some("agent-3")),
        ],
        ..Default::default()
    };
    let successions = std::collections::BTreeMap::from([
        (
            "agent-1".to_owned(),
            crate::coordination::Succession {
                successor: record("", &worker, None),
            },
        ),
        (
            "agent-5".to_owned(),
            crate::coordination::Succession {
                successor: record("", &checker, None),
            },
        ),
    ]);
    assert_eq!(
        crate::coordination::succeed(
            &mut ledger,
            &successions,
            1,
            &crate::coordination::Held::default()
        ),
        [
            ("agent-1".to_owned(), "agent-18".to_owned()),
            ("agent-5".to_owned(), "agent-19".to_owned())
        ]
    );
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let mut mission = mission_with_worker("agent-18");
    mission.conductor.actor = conductor.clone();
    mission.conductor.agent_id = Some("agent-3".into());
    mission.nodes[0].criteria[0].checkers.push(
        serde_json::from_value(
            serde_json::json!({"worker": {"agent": "codex"}, "agent_id": "agent-19"}),
        )
        .unwrap(),
    );
    runtime.missions.book.missions.push(mission);

    let healing: Vec<String> = ["w1:p1", "w1:p7"].map(str::to_owned).to_vec();
    heal(&mut runtime, &node, &healing, 1_000);
    assert_eq!(
        runtime.heal_letters_owed(),
        ["go-on:agent-18:1000", "go-on:agent-19:1000"]
    );
    runtime.heal_not_woken(&["agent-19".to_owned()]);
    for actor in [&conductor, &worker] {
        observe(&mut runtime, actor);
    }
    runtime.hook_after_heal(&node, "w1:p1", true);
    assert_eq!(
        admit(&mut runtime),
        [
            (
                "w1:p3".to_owned(),
                "Resumed: 1 · agent w1:p1\nCould not wake agent w1:p7".to_owned()
            ),
            (
                "w1:p1".to_owned(),
                "Carry on with the work that was cut off.".to_owned()
            ),
        ]
    );
    assert!(runtime.heal_letters_owed().is_empty());
}

/// B17, B18: the resumed agent's reports can reach the core before its
/// session pass does: whoever comes first takes the engine's resume and
/// settles it in the same step, so the reports, the second of which says no
/// heal is behind it, and the pass after them leave the line and the
/// parent's count owed, once.
#[test]
fn a_resumed_agent_reporting_before_the_session_pass_is_still_told_once() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &child, Some("agent-9")),
        ],
    );
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    let fake = crate::engine::fake::FakeEngine::absent();
    let engine = fake.engine();
    fake.resumed("w1:p1");
    // Its two reports, as the hook route runs them.
    runtime.take_heal(&node, engine.as_ref(), 1_000);
    runtime.hook_after_heal(&node, "w1:p1", true);
    runtime.take_heal(&node, engine.as_ref(), 2_000);
    runtime.hook_after_heal(&node, "w1:p1", false);
    // The session pass, after both, reads nothing healing.
    runtime.take_heal(&node, engine.as_ref(), 3_000);
    runtime.note_healing(&node, &[], 3_000);
    observe(&mut runtime, &child);
    observe(&mut runtime, &parent);
    assert_eq!(
        admit(&mut runtime),
        [
            ("w1:p9".to_owned(), "Resumed: 1 · agent w1:p1".to_owned()),
            (
                "w1:p1".to_owned(),
                "Carry on with the work that was cut off.".to_owned()
            ),
        ]
    );
    assert!(admit(&mut runtime).is_empty(), "owed once");
}

/// B18: a parent's count waits for every child's outcome, not for the
/// child to show in its pane: one child reports after its resume, the
/// other shows in its pane and then its resume fails; the parent gets one
/// letter, the first counted back and the second not woken.
#[test]
fn a_parent_hears_once_every_child_has_an_outcome() {
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let back = actor(&node, "w1:p1", "s-1");
    let failed = actor(&node, "w1:p2", "s-2");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &back, Some("agent-9")),
            record("agent-2", &failed, Some("agent-9")),
        ],
    );
    observe(&mut runtime, &parent);
    // The restore says both resumes, and the pass reads both healing.
    let fake = crate::engine::fake::FakeEngine::absent();
    let engine = fake.engine();
    fake.resumed("w1:p1");
    fake.resumed("w1:p2");
    runtime.take_heal(&node, engine.as_ref(), 1_000);
    let both = ["w1:p1".to_owned(), "w1:p2".to_owned()];
    runtime.note_healing(&node, &both, 1_000);
    // One reports; the other only shows in its pane, its resume not done.
    runtime.hook_after_heal(&node, "w1:p1", true);
    observe(&mut runtime, &back);
    observe(&mut runtime, &failed);
    runtime.note_healing(&node, &["w1:p2".to_owned()], 2_000);
    assert!(due(&mut runtime).is_empty(), "one child has no outcome yet");

    // Its resume fails: the pass ends its registration and the engine
    // names it healing no more.
    runtime.heal_not_woken(&["agent-2".to_owned()]);
    runtime.note_healing(&node, &[], 120_000);
    assert_eq!(
        admit(&mut runtime),
        [(
            "w1:p9".to_owned(),
            "Resumed: 1 · agent w1:p1\nCould not wake agent w1:p2".to_owned()
        )]
    );
    assert!(admit(&mut runtime).is_empty(), "once");
}

/// B17: a registration is owed one line to go on at a time. A second heal
/// while the first heal's line is still owed stays pending, its cut kept
/// even through a report with no heal behind it, and is owed, and goes,
/// once the first line is admitted.
#[test]
fn a_second_heal_waits_until_the_first_line_is_admitted() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let child = actor(&node, "w1:p1", "s-1");
    let worker = record("agent-1", &child, Some("agent-9"));
    registered(
        &mut runtime,
        vec![record("agent-9", &parent, None), worker.clone()],
    );
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 1_000);
    assert_eq!(runtime.heal_letters_owed(), ["go-on:agent-1:1000"]);

    // Cut again before its line went.
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 5_000);
    runtime.hook_after_heal(&node, "w1:p1", false);
    assert_eq!(runtime.heal_pending(), ["w1:p1"]);
    assert_eq!(runtime.heal_letters_owed(), ["go-on:agent-1:1000"]);
    assert!(
        runtime.hook_facts.lock().unwrap().cut_pending("w1:p1"),
        "the cut is kept for the pending heal"
    );
    assert!(runtime.healing(&worker));

    observe(&mut runtime, &child);
    runtime.note_healing(&node, &[], 6_000);
    let line = (
        "w1:p1".to_owned(),
        "Carry on with the work that was cut off.".to_owned(),
    );
    assert_eq!(admit(&mut runtime), std::slice::from_ref(&line));
    assert_eq!(admit(&mut runtime), [line]);
    assert!(runtime.heal_pending().is_empty());
    assert!(!runtime.hook_facts.lock().unwrap().cut_pending("w1:p1"));
    // The parent, away, is still owed its count.
    let owed = runtime.heal_letters_owed();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert!(owed[0].starts_with("children:agent-9:") && owed[0].ends_with(":1"));
}

/// B17, B18: a pending heal is about the agent it was cut for. When that
/// agent ends and another takes the pane before the line could be owed,
/// the newcomer gets nothing and the parent hears the cut one could not be
/// told, once.
#[test]
fn a_pending_heal_never_tells_the_agent_that_took_the_pane_next() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let cut = actor(&node, "w1:p1", "s-1");
    let next = actor(&node, "w1:p1", "s-2");
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            record("agent-1", &cut, Some("agent-9")),
        ],
    );
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 1_000);
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 5_000);
    assert!(due(&mut runtime).is_empty());
    assert_eq!(runtime.heal_pending(), ["w1:p1"]);

    // The cut agent ends; another child registers in its pane.
    let mut ended = record("agent-1", &cut, Some("agent-9"));
    ended.ended = true;
    registered(
        &mut runtime,
        vec![
            record("agent-9", &parent, None),
            ended,
            record("agent-2", &next, Some("agent-9")),
        ],
    );
    observe(&mut runtime, &next);
    observe(&mut runtime, &parent);
    runtime.note_healing(&node, &[], 6_000);
    assert_eq!(
        admit(&mut runtime),
        [(
            "w1:p9".to_owned(),
            "Could not tell agent w1:p1 to carry on".to_owned()
        )]
    );
    assert!(runtime.heal_pending().is_empty());
    assert!(runtime.heal_letters_owed().is_empty());
    assert!(!runtime.hook_facts.lock().unwrap().cut_pending("w1:p1"));
}

/// B18: what a parent hears of its lost children is one entry for that
/// parent, however many fail while it cannot be reached, naming the first
/// and counting the rest; it ends with the parent.
#[test]
fn a_parent_away_owes_one_entry_for_all_its_lost_children() {
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let parent = actor(&node, "w1:p9", "s-9");
    let children: Vec<AgentRecord> = (0..50)
        .map(|n| {
            let pane = format!("w2:p{n}");
            record(
                &format!("agent-{}", 100 + n),
                &actor(&node, &pane, &format!("s-{n}")),
                Some("agent-9"),
            )
        })
        .collect();
    let ids: Vec<String> = children.iter().map(|record| record.id.clone()).collect();
    let mut records = vec![record("agent-9", &parent, None)];
    records.extend(children);
    registered(&mut runtime, records.clone());
    runtime.heal_not_woken(&ids);
    assert!(due(&mut runtime).is_empty(), "the parent is away");
    let owed = runtime.heal_letters_owed();
    assert_eq!(owed.len(), 1, "{owed:?}");
    assert!(owed[0].starts_with("children:agent-9:") && owed[0].ends_with(":50"));

    observe(&mut runtime, &parent);
    let said = admit(&mut runtime);
    assert_eq!(said.len(), 1);
    assert!(
        said[0]
            .1
            .starts_with("Could not wake agent w2:p0, agent w2:p1")
    );
    assert!(said[0].1.ends_with("\nand 18 more"), "{:?}", said[0].1);
    assert!(runtime.heal_letters_owed().is_empty());

    // Owed again, then the parent ends: the entry goes with it.
    runtime.heal_not_woken(&ids[..2]);
    records[0].ended = true;
    registered(&mut runtime, records);
    assert!(due(&mut runtime).is_empty());
    assert!(runtime.heal_letters_owed().is_empty());
}

/// B19: a node's worker is healing while its engine names its pane so and
/// while its line is owed, and not after; a pane its engine still names
/// after the bound, its answer lost, is healing no more.
#[test]
fn a_worker_heals_while_its_engine_names_it_and_while_its_line_is_owed() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let node = runtime.node().as_str().to_owned();
    let worker = record("agent-1", &actor(&node, "w1:p1", "s-1"), Some("agent-9"));
    let device_worker = record("agent-2", &actor(DEVICE, "w1:p1", "s-1"), Some("agent-9"));
    assert!(!runtime.healing(&worker));

    runtime.note_healing(&node, &["w1:p1".to_owned()], 0);
    assert!(runtime.healing(&worker));
    assert!(!runtime.healing(&device_worker), "another machine's pane");
    runtime.note_healing(DEVICE, &["w1:p1".to_owned()], 0);
    assert!(runtime.healing(&device_worker));
    runtime.note_healing(DEVICE, &["w1:p1".to_owned()], 10 * 60 * 1000);
    assert!(
        runtime.healing(&device_worker),
        "the clock runs from the first read"
    );
    runtime.drive_heal_letters(10 * 60 * 1000 + 1);
    assert!(!runtime.healing(&device_worker));
    runtime.note_healing(DEVICE, &["w1:p1".to_owned()], 10 * 60 * 1000 + 2);
    assert!(
        !runtime.healing(&device_worker),
        "the same naming is not new"
    );

    // Its heal is over, but it is still owed its line.
    runtime.note_healing(&node, &[], 1);
    registered(&mut runtime, vec![worker.clone()]);
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, &node, &["w1:p1".to_owned()], 2);
    runtime.note_healing(&node, &[], 3);
    assert!(runtime.healing(&worker));
    observe(&mut runtime, &worker.actor);
    assert_eq!(admit(&mut runtime).len(), 1);
    assert!(!runtime.healing(&worker));
}

/// B17, B30: the line is a news cell of the resumed agent's registration.
/// One refused for room stays owed and lands once there is room, under the
/// same subject and revision; once posted it is settled and never posted
/// again.
#[test]
fn a_line_stays_owed_until_its_cell_is_posted() {
    let root = tempfile::tempdir().unwrap();
    let node = crate::node::TEST_NODE;
    let registered_as = |id: &str, pane: &str, session: &str, parent: Option<&str>| {
        let mut actor = actor(node, pane, session);
        actor.name = id.to_owned();
        actor.session = crate::engine::tokens::session_digest(session);
        AgentRecord {
            session: session.into(),
            name: Some(id.into()),
            ..record(id, &actor, parent)
        }
    };
    let ledger = crate::delivery::ledger::Ledger {
        agents: vec![
            registered_as("agent-9", "w1:p9", "s-9", None),
            registered_as("agent-1", "w1:p1", "s-1", Some("agent-9")),
        ],
        next_id: 10,
        ..Default::default()
    };
    let go_on = |runtime: &Runtime| -> Vec<String> {
        let mut owed = runtime.heal_letters_owed();
        owed.retain(|key| key.starts_with("go-on:"));
        owed
    };
    let mut runtime = super::runtime();
    runtime.publish_delivery(Arc::new(ledger), true);
    // The child is back in its pane, as the session sync sees it.
    let observed: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(serde_json::json!({"agents":[
            {"id":"agent-1","pane_id":"w1:p1","agent":"claude","agent_status":"idle",
             "state_change_seq":1,
             "lineage_session":crate::engine::tokens::session_digest("s-1")},
        ]}))
        .unwrap();
    runtime.observe_delivery_laid(node, &observed, None, None);
    let child = runtime.delivery_observations["w1:p1"].actor.clone();
    started_with(&mut runtime, root.path(), &[("w1:p1", "s-1", true)]);
    heal(&mut runtime, node, &["w1:p1".to_owned()], 1_000);
    assert_eq!(go_on(&runtime), ["go-on:agent-1:1000"]);
    // No room for one more cell: the line stays owed.
    let mut filled = 0;
    while runtime
        .post_status(&child, format!("letter:{filled}"), "ended".into(), true, 1)
        .is_ok()
    {
        filled += 1;
        assert!(filled <= 4_096, "the cells are capped");
    }
    runtime.drive_heal_letters(1_500);
    assert_eq!(go_on(&runtime), ["go-on:agent-1:1000"], "refused for room");
    runtime.retract_status("letter:0");
    runtime.drive_heal_letters(2_000);
    assert!(go_on(&runtime).is_empty(), "posted, it is settled");
    let ledger = runtime.delivery_state().ok();
    let heal_cells = |runtime: &mut Runtime| -> Vec<(String, u64)> {
        runtime
            .take_status_of("agent-1", ledger.as_deref(), None)
            .into_iter()
            .filter(|cell| cell.subject.starts_with("heal:"))
            .map(|cell| (cell.subject, cell.revision))
            .collect()
    };
    let mut given = Vec::new();
    for _ in 0..(filled / 64 + 2) {
        given.extend(heal_cells(&mut runtime));
    }
    assert_eq!(given, [("heal:go-on:agent-1".to_owned(), 1_000)]);
    runtime.drive_heal_letters(3_000);
    assert!(heal_cells(&mut runtime).is_empty(), "posted once");
}

/// B18, B30: each batch of a parent's lost children is a news cell of its
/// own for the parent: once the first was posted, the next children lost
/// make another, and the parent is given both.
#[test]
fn a_parent_hears_each_batch_of_lost_children_as_a_cell() {
    let node = crate::node::TEST_NODE;
    let registered_as = |id: &str, pane: &str, session: &str, parent: Option<&str>| {
        let mut actor = actor(node, pane, session);
        actor.name = id.to_owned();
        actor.session = crate::engine::tokens::session_digest(session);
        AgentRecord {
            session: session.into(),
            name: Some(id.into()),
            ..record(id, &actor, parent)
        }
    };
    let ledger = crate::delivery::ledger::Ledger {
        agents: vec![
            registered_as("agent-9", "w1:p9", "s-9", None),
            registered_as("agent-1", "w1:p1", "s-1", Some("agent-9")),
            registered_as("agent-2", "w1:p2", "s-2", Some("agent-9")),
        ],
        next_id: 10,
        ..Default::default()
    };
    let mut runtime = super::runtime();
    runtime.publish_delivery(Arc::new(ledger), true);
    // The parent is in its pane.
    let observed: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(serde_json::json!({"agents":[
            {"id":"agent-9","pane_id":"w1:p9","agent":"claude","agent_status":"idle",
             "state_change_seq":1,
             "lineage_session":crate::engine::tokens::session_digest("s-9")},
        ]}))
        .unwrap();
    runtime.observe_delivery_laid(node, &observed, None, None);
    for (n, child) in ["agent-1", "agent-2"].into_iter().enumerate() {
        runtime.heal_not_woken(&[child.to_owned()]);
        runtime.drive_heal_letters(1_000 * (n as u64 + 1));
        assert!(runtime.heal_letters_owed().is_empty(), "batch {n} posted");
    }
    let ledger = runtime.delivery_state().ok();
    let given = runtime.take_status_of("agent-9", ledger.as_deref(), None);
    assert_eq!(given.len(), 2, "{given:?}");
    assert_ne!(
        given[0].subject, given[1].subject,
        "a new batch is a new cell"
    );
    let mut lines = given
        .iter()
        .map(|cell| cell.line.as_str())
        .collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, ["Could not wake agent-1", "Could not wake agent-2"]);
}
