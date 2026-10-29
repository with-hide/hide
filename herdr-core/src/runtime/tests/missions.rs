//! A Conductor's mission (PRD software-factory-conductor): the record its
//! `hide mission` commands keep, a plan that opens nothing, the spawns its plan
//! admits, the session that takes it over, the wakes its pull requests send
//! and the record going with an archived session.

use super::*;
use crate::delivery::{Actor, worker::Worker};
use crate::handle::ChangeNotifier;
use crate::missions::{AnswerFailure, MissionCommand, MissionStatus, NodeStatus, Strength};
use crate::model::{
    DetailItem, GithubProjectSnapshot, GithubSnapshot, PullRequestBadge, PullRequestChecks,
    PullRequestSnapshot, ReviewDecision,
};
use crate::runtime::delivery::tests::{authority, fixture};
use crate::sidebar::SessionSnapshotPayload;
use serde_json::{Value, json};

const PROJECT: &str = "/checkouts/fixture";

struct Fixture {
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    conductor: Actor,
    _worker: Worker,
}

/// The fixture's `sender` pane conducts in the project `/checkouts/fixture`.
fn conducting() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let (runtime, conductor, _, path) = fixture(root.path());
    let (worker, client) =
        Worker::spawn(Arc::downgrade(&runtime), ChangeNotifier::noop(), path).unwrap();
    {
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        guard
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: PROJECT.into(),
                ..Default::default()
            });
    }
    Fixture {
        _root: root,
        runtime,
        conductor,
        _worker: worker,
    }
}

fn command(runtime: &mut Runtime, pane: &str, command: Value) -> Result<(Value, bool), String> {
    let actor = Actor {
        pane_id: pane.into(),
        name: pane.into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: None,
    };
    let command: MissionCommand = serde_json::from_value(command).unwrap();
    runtime.mission_command(
        crate::node::TEST_NODE,
        pane,
        &authority(&actor).context,
        command,
    )
}

fn plan() -> Value {
    json!({"op": "plan", "plan": {"nodes": [
        {"key": "cause", "title": "Find the cause", "kind": "research", "criteria": ["it works"]},
        {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr", "depends_on": ["cause"]},
    ]}})
}

/// The mission as one made before node criteria (D-19): for a test of
/// another rule, whose nodes and close the criteria gate does not hold.
fn without_gate(runtime: &mut Runtime) {
    for mission in &mut runtime.missions.book.missions {
        mission.exempt = true;
    }
}

fn mission_slot_items(runtime: &Runtime) -> usize {
    runtime
        .snapshot
        .ui_state
        .detail
        .items
        .iter()
        .filter(|item| matches!(item, DetailItem::Mission { .. }))
        .count()
}

fn spawn(machine: Option<&str>, intent: &str, brief: Option<&str>) -> crate::delivery::Command {
    crate::delivery::Command::Agents {
        command: crate::coordination::Command::Spawn {
            parent: Some("here".into()),
            machine: machine.map(str::to_owned),
            name: "worker".into(),
            intent: intent.into(),
            kind: "codex".into(),
            repo: PROJECT.into(),
            branch: "cause".into(),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: brief.map(str::to_owned),
            mission: None,
            beside: false,
        },
    }
}

/// The node `cause`'s worker, started on its brief.
fn prepare_spawn(
    runtime: &mut Runtime,
    conductor: &Actor,
    machine: Option<&str>,
) -> Result<(), String> {
    prepare_spawn_of(
        runtime,
        conductor,
        machine,
        "mission-1:cause",
        Some("Find the cause."),
    )
}

fn prepare_spawn_of(
    runtime: &mut Runtime,
    conductor: &Actor,
    machine: Option<&str>,
    intent: &str,
    brief: Option<&str>,
) -> Result<(), String> {
    runtime
        .prepare_delivery(
            crate::node::TEST_NODE,
            &conductor.pane_id,
            &authority(conductor).context,
            None,
            spawn(machine, intent, brief),
        )
        .map(drop)
}

/// A mission node's worker starts on its brief, or it would stand at an
/// empty prompt until a person typed, because a letter cannot wake a
/// session before its first turn: a spawn under a node's intent without a
/// brief is refused, while a worker under another intent (a reviewer) and a
/// spawn outside any mission are taken as before.
#[test]
fn a_mission_nodes_worker_is_refused_without_its_brief() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let conductor = fixture.conductor.clone();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug", "cap": 3}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    assert_eq!(
        prepare_spawn_of(&mut runtime, &conductor, None, "mission-1:cause", None).unwrap_err(),
        "mission_brief_missing"
    );
    assert_eq!(
        prepare_spawn_of(&mut runtime, &conductor, None, "mission-1:fix", None).unwrap_err(),
        "mission_brief_missing"
    );
    prepare_spawn_of(
        &mut runtime,
        &conductor,
        None,
        "mission-1:cause",
        Some("Find it."),
    )
    .unwrap();
    // Not a node of the plan: a reviewer the Conductor names its own way.
    prepare_spawn_of(&mut runtime, &conductor, None, "mission-1:review-fix", None).unwrap();
    prepare_spawn_of(&mut runtime, &conductor, None, "plain", None).unwrap();
}

#[test]
fn the_first_plan_and_a_replan_open_nothing_in_the_lane() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let (created, changed) = command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(created["mission"]["status"], "planning");
    assert_eq!(
        created["mission"]["cap"], 4,
        "the settings' concurrency is the default cap"
    );
    assert_eq!(
        mission_slot_items(&runtime),
        0,
        "an empty mission opens nothing"
    );
    let detail = runtime.snapshot.ui_state.detail.clone();

    // The first plan shows on the Conductor pane's chip, which the shell
    // draws; the lane's 보기 slot stays as it was (mission-pane-chip B11).
    let (planned, _) = command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(planned["mission"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(mission_slot_items(&runtime), 0);
    assert_eq!(runtime.snapshot.ui_state.detail, detail);

    command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(mission_slot_items(&runtime), 0);
    assert_eq!(runtime.snapshot.ui_state.detail, detail);

    runtime.refresh_missions();
    let view = &runtime.snapshot.missions.items[0];
    assert!(view.conductor.present);
    assert_eq!(view.nodes[1].depends_on, vec!["cause".to_owned()]);
    assert_eq!(
        view.nodes[0].stage, None,
        "a research node has no four-step bar"
    );
}

#[test]
fn a_conductors_spawn_is_refused_before_the_plan_is_approved_and_on_another_machine() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let conductor = fixture.conductor.clone();
    // A pane that conducts nothing spawns as it always did.
    assert!(
        runtime
            .mission_spawn_admission(
                &conductor,
                None,
                "codex",
                &Default::default(),
                "plain",
                false
            )
            .unwrap()
            .is_none()
    );

    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug", "cap": 2}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(
        prepare_spawn(&mut runtime, &conductor, None).unwrap_err(),
        "mission_not_started"
    );

    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    assert_eq!(
        prepare_spawn(&mut runtime, &conductor, Some("mini")).unwrap_err(),
        "mission_machine_refused",
        "a worker runs only on the machine the project is on (D-41)"
    );
    let admission = runtime
        .mission_spawn_admission(
            &conductor,
            None,
            "codex",
            &Default::default(),
            "plain",
            false,
        )
        .unwrap()
        .expect("a running mission's spawn is one of its workers");
    assert_eq!(
        (admission.mission.as_str(), admission.cap),
        ("mission-1", 2)
    );
    assert!(prepare_spawn(&mut runtime, &conductor, None).is_ok());

    // A closed mission no longer holds its Conductor's spawns.
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    assert!(
        runtime
            .mission_spawn_admission(
                &conductor,
                None,
                "codex",
                &Default::default(),
                "plain",
                false
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn only_the_conductor_changes_its_mission() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    assert_eq!(
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Another"})
        )
        .unwrap_err(),
        "mission_already_conducting"
    );
    assert_eq!(
        command(
            &mut runtime,
            "recipient",
            json!({"op": "plan", "mission": "mission-1", "plan": {"nodes": []}})
        )
        .unwrap_err(),
        "mission_not_conductor"
    );
    assert_eq!(
        command(&mut runtime, "recipient", json!({"op": "show"})).unwrap_err(),
        "mission_none"
    );
    let (shown, changed) = command(
        &mut runtime,
        "recipient",
        json!({"op": "show", "mission": "mission-1"}),
    )
    .unwrap();
    assert!(!changed);
    assert_eq!(shown["mission"]["title"], "Fix the bug");
}

#[test]
fn a_mission_is_taken_over_only_once_its_conductor_is_gone() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let refused = command(
        &mut runtime,
        "recipient",
        json!({"op": "take", "mission": "mission-1"}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused),
        ("mission_conducted", Some("sender")),
        "the refusal names who holds it (B45)"
    );

    // The Conductor's pane closed.
    runtime.delivery_observations.remove("sender");
    runtime.refresh_missions();
    assert!(!runtime.snapshot.missions.items[0].conductor.present);
    let (taken, changed) = command(
        &mut runtime,
        "recipient",
        json!({"op": "take", "mission": "mission-1"}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(taken["mission"]["conductor"]["name"], "recipient");
    assert!(taken["mission"]["conductor"]["present"].as_bool().unwrap());
    // Taking it again is the same answer.
    assert!(
        !command(
            &mut runtime,
            "recipient",
            json!({"op": "take", "mission": "mission-1"})
        )
        .unwrap()
        .1
    );
}

#[test]
fn a_cancelled_mission_is_over() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    for refused in [
        json!({"op": "reopen", "mission": "mission-1"}),
        json!({"op": "plan", "mission": "mission-1", "plan": {"nodes": []}}),
        json!({"op": "start", "mission": "mission-1"}),
    ] {
        assert_eq!(
            command(&mut runtime, "sender", refused).unwrap_err(),
            "mission_cancelled"
        );
    }
    // Unnamed, a command no longer finds it as the mission it conducts.
    assert_eq!(
        command(&mut runtime, "sender", json!({"op": "start"})).unwrap_err(),
        "mission_none"
    );
    // A cancelled mission no longer counts as conducted: a new one may start.
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Next"}),
    )
    .unwrap();
}

fn pull(
    number: u32,
    head: &str,
    badge: PullRequestBadge,
    review: Option<ReviewDecision>,
) -> PullRequestSnapshot {
    PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: format!("PR {number}"),
        checks: PullRequestChecks::Passing,
        number,
        head_branch: "fix".into(),
        base_branch: "main".into(),
        url: format!("https://github.com/acme/project/pull/{number}"),
        badge,
        review,
        is_draft: false,
        merged_at_unix_ms: None,
        updated_at_unix_ms: Some(1),
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        head_oid: Some(head.into()),
        cross_repository: false,
        work: Default::default(),
    }
}

fn github(pulls: Vec<PullRequestSnapshot>, stale: bool) -> GithubSnapshot {
    let mut project = GithubProjectSnapshot {
        pull_requests_read: true,
        root_path: PROJECT.into(),
        pull_requests: pulls,
        ..Default::default()
    };
    project.status.stale = stale;
    GithubSnapshot {
        projects: vec![project],
    }
}

#[test]
fn a_missions_pull_request_wakes_its_conductor_once_per_change() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
    let node = crate::node::TEST_NODE;

    // The first answer about the pull request is where its wakes start.
    let open = github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false);
    assert!(runtime.note_mission_pulls(node, &open).is_empty());
    assert!(
        runtime.note_mission_pulls(node, &open).is_empty(),
        "the same answer wakes nothing"
    );

    let pushed = github(vec![pull(7, "b2", PullRequestBadge::Open, None)], false);
    assert!(
        runtime
            .note_mission_pulls(
                node,
                &github(vec![pull(7, "b2", PullRequestBadge::Open, None)], true)
            )
            .is_empty(),
        "a stale read wakes nothing"
    );
    assert_eq!(
        runtime.note_mission_pulls(node, &pushed),
        vec!["mission:mission-1:fix:7:new_head:b2:1:sender"]
    );
    assert!(runtime.note_mission_pulls(node, &pushed).is_empty());

    let reviewed = github(
        vec![pull(
            7,
            "b2",
            PullRequestBadge::Open,
            Some(ReviewDecision::ChangesRequested),
        )],
        false,
    );
    assert_eq!(
        runtime.note_mission_pulls(node, &reviewed),
        vec!["mission:mission-1:fix:7:changes_requested:b2:2:sender"]
    );

    // A new commit on a done mission's pull request opens it again.
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    let status = |runtime: &Runtime, id: &str| runtime.missions.book.get(id).unwrap().status;
    let pushed_again = github(vec![pull(7, "c3", PullRequestBadge::Open, None)], false);
    assert_eq!(runtime.note_mission_pulls(node, &pushed_again).len(), 1);
    assert_eq!(status(&runtime, "mission-1"), MissionStatus::Running);

    // Its merge is the end it was waiting for: no wake, and it stays done.
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    let merged = github(vec![pull(7, "c3", PullRequestBadge::Merged, None)], false);
    assert!(runtime.note_mission_pulls(node, &merged).is_empty());
    assert_eq!(status(&runtime, "mission-1"), MissionStatus::Done);
}

/// A Conductor conducts one open mission, which its commands and starts
/// mean when they name none: a change to a done one while it conducts
/// another tells it, and neither that change nor `reopen` opens a second.
#[test]
fn a_done_mission_does_not_open_beside_its_conductors_open_one() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "First"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
    let node = crate::node::TEST_NODE;
    runtime.note_mission_pulls(
        node,
        &github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false),
    );
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Second"}),
    )
    .unwrap();

    let pushed = github(vec![pull(7, "b2", PullRequestBadge::Open, None)], false);
    assert_eq!(
        runtime.note_mission_pulls(node, &pushed),
        vec!["mission:mission-1:fix:7:new_head:b2:1:sender"],
        "the Conductor is still told"
    );
    let status = |runtime: &Runtime, id: &str| runtime.missions.book.get(id).unwrap().status;
    assert_eq!(status(&runtime, "mission-1"), MissionStatus::Done);
    assert_eq!(
        command(
            &mut runtime,
            "sender",
            json!({"op": "reopen", "mission": "mission-1"})
        )
        .unwrap_err(),
        "mission_already_conducting"
    );
    assert_eq!(status(&runtime, "mission-1"), MissionStatus::Done);
    // What names no mission still means the open one.
    let (shown, _) = command(&mut runtime, "sender", json!({"op": "show"})).unwrap();
    assert_eq!(shown["mission"]["id"], "mission-2");
}

/// A wake is kept in the record until its letter is in the Conductor's
/// inbox: with no sending here, each change is still waiting, a restart
/// reads them back, and only a delivered one leaves. The same change asked
/// for twice on one head is two wakes, not one swallowed by the first's
/// letter.
#[test]
fn a_missions_wake_waits_in_the_record_until_its_letter_is_delivered() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
    let node = crate::node::TEST_NODE;
    let open = github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false);
    let asked = github(
        vec![pull(
            7,
            "a1",
            PullRequestBadge::Open,
            Some(ReviewDecision::ChangesRequested),
        )],
        false,
    );
    runtime.note_mission_pulls(node, &open);
    let first = runtime.note_mission_pulls(node, &asked);
    runtime.note_mission_pulls(node, &open);
    let second = runtime.note_mission_pulls(node, &asked);
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first, second, "a second ask is its own letter");

    let waiting = |runtime: &Runtime| {
        runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .wakes
            .iter()
            .map(|wake| wake.seq)
            .collect::<Vec<_>>()
    };
    assert_eq!(waiting(&runtime), [1, 2]);
    let stored: crate::missions::MissionBook =
        serde_json::from_value(serde_json::to_value(&runtime.missions.book).unwrap()).unwrap();
    assert_eq!(
        stored.get("mission-1").unwrap().wakes.len(),
        2,
        "a restart reads them back"
    );

    runtime.mission_wakes_sent(&[("mission-1".to_owned(), 1)]);
    assert_eq!(waiting(&runtime), [2]);

    // A Conductor that takes the mission over is the recipient its wakes
    // name, so none is taken for a letter its predecessor was sent.
    runtime.delivery_observations.remove("sender");
    command(
        &mut runtime,
        "recipient",
        json!({"op": "take", "mission": "mission-1"}),
    )
    .unwrap();
    runtime.note_mission_pulls(node, &open);
    let after = runtime.note_mission_pulls(node, &asked);
    assert_eq!(after.len(), 1);
    assert!(after[0].ends_with(":recipient"), "{after:?}");
}

#[test]
fn a_cancelled_missions_pull_request_wakes_no_one() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7, "status": "working"}}),
    )
    .unwrap();
    let node = crate::node::TEST_NODE;
    runtime.note_mission_pulls(
        node,
        &github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false),
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    assert!(
        runtime
            .note_mission_pulls(
                node,
                &github(vec![pull(7, "b2", PullRequestBadge::Merged, None)], false)
            )
            .is_empty()
    );
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert_eq!(mission.status, MissionStatus::Cancelled);
    assert_eq!(mission.nodes[1].status, NodeStatus::Working);
}

#[test]
fn a_mission_goes_with_its_conductors_deleted_session() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    // The pane's row has no session id here; the Conductor's own native
    // identity names the session the mission goes with.
    let native = runtime.delivery_observations["sender"]
        .actor
        .session
        .clone()
        .unwrap();
    assert_eq!(
        runtime.missions.book.missions[0]
            .conductor
            .session
            .as_ref()
            .map(|session| session.session_id.as_str()),
        Some(native.as_str())
    );
    // The operator opened the map.
    let id = runtime.missions.book.missions[0].id.clone();
    runtime.open_detail(DetailItem::Mission { mission: id });
    assert_eq!(mission_slot_items(&runtime), 1);

    assert!(!runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), "another".into())]));
    assert!(runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), native)]));
    assert!(runtime.missions.book.missions.is_empty());
    assert_eq!(
        mission_slot_items(&runtime),
        0,
        "the map of a deleted mission leaves the slot"
    );
}

#[test]
fn conductor_settings_out_of_range_are_not_kept() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let mut settings = runtime.snapshot.ui_state.conductor_settings.clone();
    settings.concurrency = 0;
    assert!(runtime.set_conductor_settings(settings.clone()));
    assert_eq!(runtime.snapshot.ui_state.conductor_settings.concurrency, 4);
    settings.concurrency = 6;
    assert!(runtime.set_conductor_settings(settings.clone()));
    assert_eq!(runtime.snapshot.ui_state.conductor_settings.concurrency, 6);
    assert!(
        !runtime.set_conductor_settings(settings),
        "the same settings change nothing"
    );
}

/// Each edit in Settings › Conductor sends only what it changes, so a second
/// edit sent before the first one's echo keeps the first.
#[test]
fn conductor_settings_change_only_what_an_edit_sends() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let event = |payload: Value| {
        serde_json::to_vec(
            &json!({"schema_version": 2, "kind": "conductor_settings_set", "payload": payload}),
        )
        .unwrap()
    };
    assert!(runtime.dispatch_json(&event(json!({"concurrency": 6}))));
    assert!(runtime.dispatch_json(&event(json!({"default_scope": "merge"}))));
    assert!(runtime.dispatch_json(&event(json!({"default_strength": "thorough"}))));
    assert!(runtime.dispatch_json(&event(json!({"other_vendor": false}))));
    let settings = &runtime.snapshot.ui_state.conductor_settings;
    assert_eq!(settings.concurrency, 6);
    assert_eq!(settings.default_scope, crate::missions::MissionScope::Merge);
    assert_eq!(settings.default_strength, Strength::Thorough);
    assert!(!settings.other_vendor);
}

/// A mission takes Settings › Conductor's verification strength and choice
/// of another company when it is created; its plan changes the strength
/// before or during the mission, each change one event with its reason, and
/// a plan that names none, or a later settings edit, keeps it.
#[test]
fn a_mission_keeps_the_strength_it_was_created_with_until_its_plan_changes_it() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let settings = &mut runtime.snapshot.ui_state.conductor_settings;
    settings.default_strength = Strength::Fast;
    settings.other_vendor = false;
    let (created, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    assert_eq!(created["mission"]["strength"], "fast");
    assert_eq!(created["mission"]["other_vendor"], false);
    let settings = &mut runtime.snapshot.ui_state.conductor_settings;
    settings.default_strength = Strength::Normal;
    let mut proposed = plan();
    proposed["plan"]["strength"] = json!("thorough");
    proposed["plan"]["strength_why"] = json!("the stored format changes");
    command(&mut runtime, "sender", proposed).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    let mut lighter = plan();
    lighter["plan"]["strength"] = json!("normal");
    command(&mut runtime, "sender", lighter.clone()).unwrap();
    let (shown, _) = command(&mut runtime, "sender", lighter).unwrap();
    assert_eq!(shown["mission"]["strength"], "normal");
    assert_eq!(shown["mission"]["other_vendor"], false);
    assert_eq!(kinds(&runtime), ["planned", "strength", "strength"]);
    let log = &runtime.missions.events["mission-1"];
    assert_eq!(
        (log.events[1].strength, log.events[1].line.as_deref()),
        (Some(Strength::Thorough), Some("the stored format changes"))
    );
    assert_eq!(
        (log.events[2].strength, log.events[2].line.as_deref()),
        (Some(Strength::Normal), None)
    );
}

#[test]
fn a_mission_asked_for_before_the_projects_are_read_says_so_rather_than_unknown() {
    let fixture = conducting();
    let mut guard = fixture.runtime.lock().unwrap();
    guard.worktree_catalog.projects.clear();
    guard.snapshot.git_worktrees_loading = true;
    let create = json!({"op": "create", "title": "Early"});
    assert_eq!(
        command(&mut guard, &fixture.conductor.pane_id, create.clone()).unwrap_err(),
        "mission_projects_loading"
    );
    guard.snapshot.git_worktrees_loading = false;
    assert_eq!(
        command(&mut guard, &fixture.conductor.pane_id, create).unwrap_err(),
        "mission_project_unknown"
    );
}

#[test]
fn a_mission_book_that_cannot_be_read_is_never_written_over() {
    let fixture = conducting();
    let path = fixture._root.path().join("missions.json");
    std::fs::write(&path, b"not a mission book").unwrap();
    {
        let mut guard = fixture.runtime.lock().unwrap();
        let store = crate::missions::MissionStore::new(path.clone());
        let restored = store.restore();
        guard.install_mission_store(store, restored);
        assert_eq!(
            command(
                &mut guard,
                &fixture.conductor.pane_id,
                json!({"op": "create", "title": "Lost"})
            )
            .unwrap_err(),
            "mission_unavailable"
        );
        // An archive delete and a pull request read both save the book when it changes.
        guard.drop_missions_of(&[(crate::node::TEST_NODE.to_owned(), "session".to_owned())]);
        guard.save_missions();
        // Dropping the store waits for any save it queued.
        drop(guard.missions.store.take());
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"not a mission book");
}

/// A device's catalog never answers an empty model list as if it read one:
/// each agent that takes a model says its models are read on the core's
/// machine only, and an unknown or disconnected machine is refused
/// (PRD software-factory-conductor B10).
#[test]
fn a_devices_catalog_says_why_it_has_no_models_rather_than_listing_none() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let node = crate::node::TEST_NODE;
    assert_eq!(
        runtime
            .agent_catalog(node, Some("mini"), false)
            .unwrap_err(),
        "machine_unknown"
    );
    let remote = |state: &str| crate::model::RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: state.into(),
        message: None,
        herdr_version: None,
        session: None,
        files: crate::model::RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    };
    runtime.snapshot.status.remote.push(remote("disconnected"));
    assert_eq!(
        runtime
            .agent_catalog(node, Some("mini"), false)
            .unwrap_err(),
        "machine_unavailable"
    );
    runtime.snapshot.status.remote[0] = remote("connected");
    let catalog = runtime.agent_catalog(node, Some("mini"), false).unwrap();
    assert_eq!(catalog["machine"], "mini");
    let items = catalog["items"].as_array().unwrap();
    assert!(!items.is_empty());
    for item in items {
        if item["takes_model"] == true {
            assert!(item["models"].is_null(), "{item}");
            assert_eq!(
                item["models_unavailable_reason"], "models_read_on_this_machine_only",
                "{item}"
            );
        }
    }
    assert!(items.iter().any(|item| item["takes_model"] == true));
}

/// The catalog on the core's own machine asks for one read of the model
/// lists when a list is unread, as the start panel does, and says a read is
/// pending; once the read lands it names the models and asks no more, and
/// a caller whose wait is over gets `models_read_timeout` beside the CLI's
/// default model, so a plan made from it is never empty (mission fixes (b)).
#[test]
fn the_catalog_asks_for_one_model_read_and_names_the_default_when_it_times_out() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let node = crate::node::TEST_NODE;
    assert!(!runtime.ai_request().observing, "nobody is looking yet");

    let catalog = runtime.agent_catalog(node, None, false).unwrap();
    assert_eq!(catalog["machine"], node);
    assert_eq!(catalog["models_read_pending"], true);
    assert!(
        runtime.ai_request().observing,
        "the catalog's ask observes the providers until a read lands"
    );
    let item = |catalog: &Value, id: &str| {
        catalog["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id} in {catalog}"))
    };
    let claude = item(&catalog, "claude-code");
    assert_eq!(claude["takes_model"], true);
    assert!(claude["models"].is_null());
    assert_eq!(claude["models_unavailable_reason"], "models_unread");
    assert_eq!(
        claude["cli_default_model"],
        hide_ai::ProviderId::CLAUDE.default_model(),
        "the measured CLI default stands in while the list is unread"
    );
    let opencode = item(&catalog, "opencode");
    assert_eq!(opencode["takes_model"], false);
    assert!(opencode["cli_default_model"].is_null());

    // The caller's wait is over: an unread list says so, beside the default.
    let late = runtime.agent_catalog(node, None, true).unwrap();
    assert_eq!(late["models_read_pending"], false);
    assert_eq!(
        item(&late, "claude-code")["models_unavailable_reason"],
        "models_read_timeout"
    );
    assert_eq!(
        item(&late, "claude-code")["cli_default_model"],
        hide_ai::ProviderId::CLAUDE.default_model()
    );
    assert!(
        runtime.ai_request().observing,
        "the ask stands until a read lands"
    );

    // The read lands: the models are named and nothing observes any more.
    runtime.ingest_background_ai(super::agents_settings_remote::ready_read(&[
        "claude", "codex",
    ]));
    assert!(!runtime.ai_request().observing);
    let read = runtime.agent_catalog(node, None, false).unwrap();
    assert_eq!(
        read["models_read_pending"], true,
        "grok and the others are still unread"
    );
    assert_eq!(item(&read, "claude-code")["models"], json!(["m-1", "m-2"]));
    assert!(item(&read, "claude-code")["models_unavailable_reason"].is_null());
    assert_eq!(item(&read, "codex")["models"], json!(["m-1", "m-2"]));
    // Every list read: no ask is made.
    runtime.ingest_background_ai(super::agents_settings_remote::ready_read(
        &hide_ai::PROVIDERS
            .iter()
            .map(|provider| provider.as_str())
            .collect::<Vec<_>>(),
    ));
    let all = runtime.agent_catalog(node, None, false).unwrap();
    assert_eq!(all["models_read_pending"], false);
    assert!(!runtime.ai_request().observing);
}

/// A plan names the model of every worker that takes one: refused
/// `mission_model_missing` otherwise, while an agent that takes no model
/// keeps `null` (mission fixes (b)).
#[test]
fn a_plan_names_the_model_of_every_worker_that_takes_one() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let plan_with = |worker: Value, kind: &str| {
        let mut node = json!({"key": "one", "title": "One", "kind": kind, "worker": worker, "criteria": ["it works"]});
        if kind == "change" {
            node["delivery"] = json!("pr");
        }
        json!({"op": "plan", "plan": {"nodes": [node]}})
    };
    for kind in ["change", "research", "verify"] {
        assert_eq!(
            command(
                &mut runtime,
                "sender",
                plan_with(json!({"agent": "claude"}), kind)
            )
            .unwrap_err(),
            "mission_model_missing",
            "{kind}"
        );
        assert_eq!(
            command(
                &mut runtime,
                "sender",
                plan_with(json!({"agent": "codex", "effort": "high"}), kind)
            )
            .unwrap_err(),
            "mission_model_missing",
            "{kind}"
        );
    }
    assert!(crate::missions::next_action("mission_model_missing").contains("cli_default_model"));
    let (planned, _) = command(
        &mut runtime,
        "sender",
        plan_with(
            json!({"agent": "claude", "model": "opus", "effort": "high"}),
            "change",
        ),
    )
    .unwrap();
    assert_eq!(planned["mission"]["nodes"][0]["worker"]["model"], "opus");
    // OpenCode takes no model: it keeps null.
    let (planned, _) = command(
        &mut runtime,
        "sender",
        plan_with(json!({"agent": "opencode"}), "research"),
    )
    .unwrap();
    assert!(planned["mission"]["nodes"][0]["worker"]["model"].is_null());
    // A node with no worker yet is still planned.
    command(&mut runtime, "sender", plan()).unwrap();
}

/// A mission left waiting on the operator's merges closes itself once its
/// whole stack is merged, with no Conductor there (B41); a stack still part
/// open keeps it waiting.
#[test]
fn a_merge_wait_mission_closes_when_its_whole_stack_is_merged() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Stack"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "api", "title": "API", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            {"key": "screen", "title": "Screen", "kind": "change", "criteria": ["it works"], "delivery": "pr", "depends_on": ["api"]},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    for (node, pr) in [("api", 7), ("screen", 8)] {
        without_gate(&mut runtime);
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"pr": pr, "status": "done"}}),
        )
        .unwrap();
    }
    let node = crate::node::TEST_NODE;
    let open = |badge_api, badge_screen| {
        github(
            vec![
                pull(7, "a1", badge_api, None),
                pull(8, "b1", badge_screen, None),
            ],
            false,
        )
    };
    runtime.note_mission_pulls(node, &open(PullRequestBadge::Open, PullRequestBadge::Open));
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    // The Conductor is gone.
    let present = runtime.delivery_observations.remove("sender").unwrap();
    let status = |runtime: &Runtime| runtime.missions.book.get("mission-1").unwrap().status;

    runtime.note_mission_pulls(
        node,
        &open(PullRequestBadge::Merged, PullRequestBadge::Open),
    );
    assert_eq!(status(&runtime), MissionStatus::MergeWait);
    runtime.note_mission_pulls(
        node,
        &open(PullRequestBadge::Merged, PullRequestBadge::Merged),
    );
    assert_eq!(status(&runtime), MissionStatus::Done);

    // A stack already merged when the Conductor says it waits has nothing
    // to wait on: the close is done at once.
    runtime.missions.book.get_mut("mission-1").unwrap().status = MissionStatus::Running;
    runtime
        .delivery_observations
        .insert("sender".into(), present);
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "done");
    assert!(
        runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .closed_at_unix_ms
            .is_some()
    );
}

/// A waiting mission stands only behind the pull requests still in its
/// stack: a node that failed is out of it, a pull request closed without
/// its merge leaves nothing to wait on, and a mission with none closes at
/// once.
#[test]
fn a_merge_wait_mission_waits_only_on_the_pull_requests_left_in_its_stack() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Stack"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "api", "title": "API", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            {"key": "screen", "title": "Screen", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            {"key": "docs", "title": "Docs", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    for (node, pr, status) in [
        ("api", 7, "done"),
        ("screen", 8, "failed"),
        ("docs", 9, "done"),
    ] {
        without_gate(&mut runtime);
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"pr": pr, "status": status}}),
        )
        .unwrap();
    }
    let node = crate::node::TEST_NODE;
    let answer = |api, docs| {
        github(
            vec![
                pull(7, "a1", api, None),
                pull(8, "b1", PullRequestBadge::Open, None),
                pull(9, "c1", docs, None),
            ],
            false,
        )
    };
    runtime.note_mission_pulls(
        node,
        &answer(PullRequestBadge::Open, PullRequestBadge::Open),
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    let status = |runtime: &Runtime, id: &str| runtime.missions.book.get(id).unwrap().status;
    runtime.note_mission_pulls(
        node,
        &answer(PullRequestBadge::Merged, PullRequestBadge::Open),
    );
    assert_eq!(status(&runtime, "mission-1"), MissionStatus::MergeWait);
    runtime.note_mission_pulls(
        node,
        &answer(PullRequestBadge::Merged, PullRequestBadge::Closed),
    );
    assert_eq!(
        status(&runtime, "mission-1"),
        MissionStatus::Done,
        "the failed node's open pull request is not waited on"
    );

    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Research"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "cause", "title": "Find the cause", "kind": "research", "criteria": ["it works"]},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    without_gate(&mut runtime);
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "done");
}

/// With worker candidates set, a Conductor's worker outside them is refused
/// by name (B8, D-34): another agent, another model than the one a candidate
/// fixes or a model where it keeps the CLI's default, another effort than a
/// fixed one. An automatic effort is the Conductor's to choose.
#[test]
fn a_conductors_worker_outside_the_candidates_is_refused() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let conductor = fixture.conductor.clone();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    let launch = |model: Option<&str>, effort: Option<&str>| hide_agent_adapter::LaunchChoice {
        model: model.map(str::to_owned),
        effort: effort.map(str::to_owned),
    };
    let admit = |runtime: &Runtime, kind: &str, choice: hide_agent_adapter::LaunchChoice| {
        runtime
            .mission_spawn_admission(&conductor, None, kind, &choice, "plain", false)
            .map(|admission| admission.is_some())
    };
    // No candidates: any agent the machine has.
    assert_eq!(admit(&runtime, "codex", launch(None, None)), Ok(true));

    let candidates = vec![
        crate::missions::Candidate {
            agent: "claude".into(),
            model: None,
            effort: None,
            purpose: "changes".into(),
        },
        crate::missions::Candidate {
            agent: "codex".into(),
            model: Some("gpt-x".into()),
            effort: Some("high".into()),
            purpose: "review".into(),
        },
    ];
    // A Settings edit applies from the next mission (B48): this one was
    // created with none and keeps admitting any agent.
    runtime.snapshot.ui_state.conductor_settings.candidates = candidates.clone();
    assert_eq!(admit(&runtime, "opencode", launch(None, None)), Ok(true));
    // A mission created with candidates is held to them.
    runtime.missions.book.missions.clear();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    assert_eq!(runtime.missions.book.missions[0].candidates, candidates);
    let refused = Err("mission_candidate_refused".to_owned());
    assert_eq!(admit(&runtime, "claude", launch(None, None)), Ok(true));
    assert_eq!(
        admit(&runtime, "claude", launch(None, Some("max"))),
        Ok(true),
        "an automatic effort is free"
    );
    assert_eq!(
        admit(&runtime, "claude", launch(Some("other"), None)),
        refused
    );
    assert_eq!(
        admit(&runtime, "codex", launch(Some("gpt-x"), Some("high"))),
        Ok(true)
    );
    assert_eq!(
        admit(&runtime, "codex", launch(Some("gpt-x"), None)),
        refused
    );
    assert_eq!(
        admit(&runtime, "codex", launch(None, Some("high"))),
        refused
    );
    assert_eq!(admit(&runtime, "opencode", launch(None, None)), refused);
    // The spawn command itself is refused, before the ledger reserves it.
    runtime.missions.book.missions[0].candidates.truncate(1);
    assert_eq!(
        prepare_spawn(&mut runtime, &conductor, None).unwrap_err(),
        "mission_candidate_refused"
    );
}

/// A created, planned and started mission conducted by `sender`.
fn running(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(runtime, "sender", plan()).unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
}

fn kinds(runtime: &Runtime) -> Vec<String> {
    runtime.missions.events["mission-1"]
        .events
        .iter()
        .map(|event| {
            serde_json::to_value(event.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn a_decision_carries_its_reason_choices_recommendation_and_node_and_refuses_what_is_not_one() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    for (asked, code) in [
        (
            json!({"line": "Q", "options": ["a", "b"], "recommend": 3}),
            "mission_recommend_invalid",
        ),
        (
            json!({"line": "Q", "options": ["1", "2", "3", "4", "5", "6"]}),
            "mission_options_too_many",
        ),
        (
            json!({"line": "Q", "node": "nowhere"}),
            "mission_node_unknown",
        ),
        (
            json!({"line": null, "options": ["a"]}),
            "mission_question_without_line",
        ),
        (
            json!({"line": "Q", "options": ["two\nlines"]}),
            "mission_option_invalid",
        ),
    ] {
        let mut request = json!({"op": "decide"});
        request
            .as_object_mut()
            .unwrap()
            .extend(asked.as_object().unwrap().clone());
        assert_eq!(command(&mut runtime, "sender", request).unwrap_err(), code);
    }
    let ask = json!({"op": "decide", "line": "Which base?", "why": "Two PRs touch it",
        "options": ["main", "#1503"], "recommend": 2, "node": "fix"});
    let (answer, changed) = command(&mut runtime, "sender", ask.clone()).unwrap();
    assert!(changed);
    let question = &answer["mission"]["question"];
    assert_eq!(
        (
            question["key"].clone(),
            question["options"].clone(),
            question["recommend"].clone(),
            question["node"].clone()
        ),
        (json!(1), json!(["main", "#1503"]), json!(2), json!("fix"))
    );
    // The same question again changes nothing and keeps its key.
    assert!(!command(&mut runtime, "sender", ask).unwrap().1);
    // `--line` alone still asks, under a new key.
    let (answer, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Ship it?"}),
    )
    .unwrap();
    assert_eq!(answer["mission"]["question"]["key"], 2);
    assert_eq!(answer["mission"]["question"]["options"], json!(null));
}

#[test]
fn a_commit_is_a_direct_changes_and_a_verdict_a_verify_nodes() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "direct"},
            {"key": "pr", "title": "Open it", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            {"key": "check", "title": "Check it", "kind": "verify", "criteria": ["it works"]},
        ]}}),
    )
    .unwrap();
    let node = |key: &str, patch: Value| json!({"op": "node", "node": key, "patch": patch});
    for (key, patch, code) in [
        (
            "pr",
            json!({"commit": "abc1234"}),
            "mission_field_kind_invalid",
        ),
        (
            "fix",
            json!({"verdict": "pass"}),
            "mission_field_kind_invalid",
        ),
        (
            "check",
            json!({"commit": "abc1234"}),
            "mission_field_kind_invalid",
        ),
        (
            "fix",
            json!({"commit": "not-a-sha"}),
            "mission_commit_invalid",
        ),
        (
            "check",
            json!({"checks": {"passed": 7, "total": 6}}),
            "mission_checks_invalid",
        ),
    ] {
        assert_eq!(
            command(&mut runtime, "sender", node(key, patch)).unwrap_err(),
            code,
            "{key}"
        );
    }
    command(
        &mut runtime,
        "sender",
        node("fix", json!({"commit": "abc1234def"})),
    )
    .unwrap();
    let (answer, _) = command(
        &mut runtime,
        "sender",
        node(
            "check",
            json!({"verdict": "fail", "checks": {"passed": 4, "total": 6}}),
        ),
    )
    .unwrap();
    let nodes = &answer["mission"]["nodes"];
    assert_eq!(nodes[0]["commit"], "abc1234def");
    assert_eq!(
        (nodes[2]["verdict"].clone(), nodes[2]["checks"].clone()),
        (json!("fail"), json!({"passed": 4, "total": 6}))
    );
}

#[test]
fn each_change_is_one_event_and_the_same_change_again_is_none() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    let node = |patch: Value| json!({"op": "node", "node": "fix", "patch": patch});
    command(&mut runtime, "sender", node(json!({"status": "working"}))).unwrap();
    command(&mut runtime, "sender", node(json!({"status": "working"}))).unwrap();
    command(&mut runtime, "sender", node(json!({"pr": 1503}))).unwrap();
    command(
        &mut runtime,
        "sender",
        node(json!({"status": "failed", "line": "CI failed"})),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        node(json!({"status": "working", "attempts": 2})),
    )
    .unwrap();
    // A replan that changes one node says so; the same plan again says nothing.
    let mut replan = plan();
    replan["plan"]["nodes"][0]["title"] = json!("Find the real cause");
    replan["plan"]["nodes"][0]["change"] = json!("The first read was wrong");
    command(&mut runtime, "sender", replan.clone()).unwrap();
    command(&mut runtime, "sender", replan).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled", "report": "Stopped.\nNothing merged."}),
    )
    .unwrap();
    assert_eq!(
        kinds(&runtime),
        [
            "planned",
            "node",
            "pr_opened",
            "node",
            "retried",
            "node",
            "replanned",
            "reported",
            "cancelled"
        ]
    );
    let log = &runtime.missions.events["mission-1"];
    assert_eq!(log.events[3].line.as_deref(), Some("CI failed"));
    assert_eq!(
        (log.events[6].count, log.events[6].line.as_deref()),
        (Some(1), Some("The first read was wrong"))
    );
    assert_eq!(log.events[7].line.as_deref(), Some("Stopped."));
}

#[test]
fn a_banner_answer_waits_for_its_question_and_is_kept_once() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?", "options": ["main", "#1503"]}),
    )
    .unwrap();
    // An answer to a question the Conductor no longer asks is refused and shows nothing.
    assert!(!runtime.mission_answer_event("mission-1", 9, Some(1), None, false));
    assert!(runtime.missions.book.missions[0].answer.is_none());
    assert!(runtime.mission_answer_event("mission-1", 1, Some(2), None, false));
    // The same question answered again keeps the first answer.
    assert!(!runtime.mission_answer_event("mission-1", 1, None, Some("main".into()), false));
    runtime.refresh_missions();
    let view = runtime.snapshot.missions.items[0].answer.clone().unwrap();
    assert_eq!(
        (view.text.as_str(), view.state),
        ("#1503", crate::runtime::missions::AnswerState::Sending)
    );
    let due = runtime.mission_answers_due();
    assert_eq!(due.len(), 1);
    assert!(due[0].line.contains("Which base?") && due[0].line.ends_with("#1503"));
    runtime.mission_answer_held("mission-1");
    runtime.refresh_missions();
    assert_eq!(
        runtime.snapshot.missions.items[0]
            .answer
            .as_ref()
            .unwrap()
            .state,
        crate::runtime::missions::AnswerState::Waiting
    );
    runtime.mission_answer_typing("mission-1", 1);
    runtime.mission_answer_sent("mission-1", 1, Ok(()));
    assert!(
        runtime.mission_answers_due().is_empty(),
        "a typed answer is typed once"
    );
    // Clearing the question after the banner's answer adds no second answer.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    assert_eq!(kinds(&runtime)[1..], ["asked", "answered"]);
    assert_eq!(
        runtime.missions.events["mission-1"].events[2].source,
        Some(crate::mission_events::AnswerSource::Banner)
    );
}

#[test]
fn an_answer_whose_conductor_left_or_whose_question_changed_is_not_delivered() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert!(runtime.mission_answer_event("mission-1", 1, None, Some("main, please".into()), false));
    // The Conductor changed its question before the answer was typed.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Merge now?"}),
    )
    .unwrap();
    assert!(runtime.missions.book.missions[0].answer.is_none());
    assert!(runtime.mission_answer_event("mission-1", 2, None, Some("yes".into()), false));
    // Its pane closed before the answer reached it (D-30): a short absence
    // only waits, as after a restart before the panes are read again.
    let observed = runtime.delivery_observations.remove("sender").unwrap();
    assert!(runtime.mission_answers_due().is_empty());
    assert!(runtime.missions.book.missions[0].answer.is_some());
    runtime
        .delivery_observations
        .insert("sender".into(), observed);
    assert_eq!(runtime.mission_answers_due().len(), 1);
    assert!(runtime.missions.answer_absent_since.is_empty());
    runtime.delivery_observations.remove("sender");
    assert!(runtime.mission_answers_due().is_empty());
    *runtime
        .missions
        .answer_absent_since
        .get_mut("mission-1")
        .unwrap() -= 30_000;
    assert!(runtime.mission_answers_due().is_empty());
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (mission.answer.is_none(), mission.answer_failed),
        (true, Some(AnswerFailure::ConductorGone))
    );
    assert_eq!(
        kinds(&runtime)[1..],
        ["asked", "answer_undelivered", "asked", "answer_undelivered"]
    );
    // With no Conductor there, the banner's answer is refused with its reason.
    assert!(runtime.mission_answer_event("mission-1", 2, None, Some("yes".into()), false));
    assert_eq!(
        runtime.missions.book.missions[0].answer_failed,
        Some(AnswerFailure::ConductorGone)
    );
    runtime.refresh_missions();
    assert_eq!(
        runtime.snapshot.missions.items[0].answer_failed,
        Some(AnswerFailure::ConductorGone)
    );
    // A question cleared with no banner answer was answered in the conversation.
    command(
        &mut runtime,
        "recipient",
        json!({"op": "take", "mission": "mission-1"}),
    )
    .unwrap();
    // The reason holds only while no Conductor is there (B39).
    runtime.refresh_missions();
    assert_eq!(runtime.snapshot.missions.items[0].answer_failed, None);
    command(
        &mut runtime,
        "recipient",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    assert_eq!(
        runtime.missions.events["mission-1"]
            .events
            .last()
            .unwrap()
            .source,
        Some(crate::mission_events::AnswerSource::Conversation)
    );
}

#[test]
fn closing_a_mission_ends_its_decision_and_records_an_answer_never_typed() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert!(runtime.mission_answer_event("mission-1", 1, None, Some("main".into()), false));
    runtime.mission_answer_held("mission-1");
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (
            mission.decision.is_none(),
            mission.question.is_none(),
            mission.answer.is_none(),
            mission.answer_failed
        ),
        (true, true, true, None)
    );
    assert!(runtime.missions.answers_held.is_empty());
    assert!(runtime.mission_answers_due().is_empty());
    assert_eq!(
        kinds(&runtime)
            .iter()
            .filter(|kind| *kind == "answer_undelivered")
            .count(),
        1
    );
}

#[test]
fn an_answer_herdr_may_have_typed_is_typed_again_only_when_the_operator_sends_it_again() {
    use crate::runtime::mission_events::AnswerSendFailure;
    use crate::runtime::missions::AnswerState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert!(runtime.mission_answer_event("mission-1", 1, None, Some("main".into()), false));
    // Sending again is refused while the answer is still to be typed.
    assert!(!runtime.mission_answer_event("mission-1", 1, None, None, true));
    assert_eq!(runtime.mission_answers_due().len(), 1);
    // The prompt is asked: the answer reads as sending, and the book already
    // holds it as unconfirmed, so a stop before Herdr's reply never types it
    // again after a restart.
    runtime.mission_answer_typing("mission-1", 1);
    runtime.refresh_missions();
    assert_eq!(
        runtime.snapshot.missions.items[0]
            .answer
            .as_ref()
            .unwrap()
            .state,
        AnswerState::Sending
    );
    assert!(
        runtime.missions.book.missions[0]
            .answer
            .as_ref()
            .unwrap()
            .unconfirmed
    );
    assert!(runtime.mission_answers_due().is_empty());
    // A press of 다시 보내기 from a stale screen while the prompt is in flight adds nothing.
    assert!(!runtime.mission_answer_event("mission-1", 1, None, None, true));
    assert!(
        runtime.missions.book.missions[0]
            .answer
            .as_ref()
            .unwrap()
            .unconfirmed
    );
    // Herdr's reply to the prompt timed out: the answer may be in the conversation.
    runtime.mission_answer_sent(
        "mission-1",
        1,
        Err(AnswerSendFailure::MaybeTyped("herdr_call_failed")),
    );
    assert!(
        runtime.mission_answers_due().is_empty(),
        "nothing types it again on its own"
    );
    runtime.refresh_missions();
    let view = &runtime.snapshot.missions.items[0];
    assert_eq!(
        (view.answer.as_ref().unwrap().state, view.answer_failed),
        (AnswerState::Unconfirmed, None)
    );
    // Another answer to the same question is still the first one.
    assert!(!runtime.mission_answer_event("mission-1", 1, None, Some("other".into()), false));
    // The operator sends it again: typed once more, and a second press adds nothing.
    assert!(runtime.mission_answer_event("mission-1", 1, None, None, true));
    assert!(!runtime.mission_answer_event("mission-1", 1, None, None, true));
    let due = runtime.mission_answers_due();
    assert_eq!(due.len(), 1);
    assert!(due[0].line.ends_with("main"));
    runtime.mission_answer_typing("mission-1", 1);
    runtime.mission_answer_sent("mission-1", 1, Ok(()));
    // A late second report for the same prompt changes nothing.
    runtime.mission_answer_sent(
        "mission-1",
        1,
        Err(AnswerSendFailure::MaybeTyped("herdr_format")),
    );
    assert!(
        runtime.missions.book.missions[0]
            .answer
            .as_ref()
            .unwrap()
            .delivered
    );
    assert!(runtime.mission_answers_due().is_empty());
    // A question cleared after it adds no answer of its own.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    assert_eq!(
        kinds(&runtime)[1..],
        ["asked", "answer_unconfirmed", "answered"]
    );
    // A failure before the prompt was asked gives the question back to answer again.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Merge now?"}),
    )
    .unwrap();
    assert!(runtime.mission_answer_event("mission-1", 2, None, Some("yes".into()), false));
    runtime.mission_answer_sent(
        "mission-1",
        2,
        Err(AnswerSendFailure::NotTyped("herdr_parameters")),
    );
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (mission.answer.is_none(), mission.answer_failed),
        (true, Some(AnswerFailure::SendFailed))
    );
}

#[test]
fn the_open_missions_published_events_share_the_cores_list() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.snapshot.ui_state.detail.items = vec![DetailItem::Mission {
        mission: "mission-1".into(),
    }];
    runtime.snapshot.ui_state.detail.shown = true;
    runtime.refresh_mission_events();
    let held = &runtime.missions.events["mission-1"].events;
    assert!(std::sync::Arc::ptr_eq(
        held,
        &runtime.snapshot.mission_events.events
    ));
    // A new event leaves the published list as it was until the next refresh.
    let published = std::sync::Arc::clone(&runtime.snapshot.mission_events.events);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert_eq!(
        published.len() + 1,
        runtime.missions.events["mission-1"].events.len()
    );
    // The event itself is shared too, not copied.
    assert!(std::sync::Arc::ptr_eq(
        &published[0],
        &runtime.missions.events["mission-1"].events[0]
    ));
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.events.len(),
        published.len() + 1
    );
}

#[test]
fn a_conductor_whose_agent_the_doorbell_does_not_ring_takes_no_banner_answer() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?", "options": ["main", "#1503"]}),
    )
    .unwrap();
    runtime.refresh_missions();
    assert!(runtime.snapshot.missions.items[0].conductor.takes_answers);
    runtime.missions.book.missions[0].conductor.actor.kind = "opencode".into();
    runtime.missions.book_edit += 1;
    runtime.refresh_missions();
    assert!(!runtime.snapshot.missions.items[0].conductor.takes_answers);
    assert_eq!(
        runtime
            .answer_mission("mission-1", 1, Some(1), None, false)
            .unwrap_err(),
        "mission_answer_unsupported"
    );
    assert!(!runtime.mission_answer_event("mission-1", 1, Some(1), None, false));
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (mission.answer.is_none(), mission.answer_failed),
        (true, None)
    );
}

#[test]
fn only_the_mission_open_in_the_slot_sends_its_events() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
    assert!(runtime.snapshot.mission_events.events.is_empty());
    runtime.open_detail(DetailItem::Mission {
        mission: "mission-1".into(),
    });
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.mission.as_deref(),
        Some("mission-1")
    );
    assert_eq!(runtime.snapshot.mission_events.events.len(), 1);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "cause", "patch": {"status": "working"}}),
    )
    .unwrap();
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.events.len(), 2);
    // The events go with their mission's deleted session (D-22).
    let session = runtime.missions.book.missions[0]
        .conductor
        .session
        .clone()
        .unwrap();
    assert!(runtime.drop_missions_of(&[(session.device_id, session.session_id)]));
    assert!(!runtime.missions.events.contains_key("mission-1"));
    runtime.refresh_mission_events();
    assert!(runtime.snapshot.mission_events.events.is_empty());
}

/// The fixture's `recipient` pane as the node's worker and a third pane,
/// `reviewer`, as another agent, each registered in the ledger.
fn with_worker_and_reviewer(runtime: &mut Runtime) {
    let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
        {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"recipient-session"},
        {"id":"reviewer","pane_id":"reviewer","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"reviewer-session"},
    ]}))
    .unwrap();
    runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
    let panes = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0].panes;
    let mut reviewer = panes[1].clone();
    reviewer.id = "reviewer".into();
    panes.push(reviewer);
    let mut ledger = crate::delivery::ledger::Ledger::default();
    for (id, pane) in [
        ("agent-1", "sender"),
        ("agent-2", "recipient"),
        ("agent-3", "reviewer"),
    ] {
        ledger.agents.push(crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(pane.into()),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "scope".into(),
            native_machine: "fixture-machine".into(),
            session: format!("{pane}-session"),
            instance: pane.into(),
            pane: pane.into(),
            parent: (id != "agent-1").then(|| "agent-1".into()),
            origin: None,
            project: None,
            actor: runtime.delivery_observations[pane].actor.clone(),
            ended: false,
            terminal_id: None,
            forked: false,
        });
    }
    runtime.delivery_ledger = Ok(Arc::new(ledger));
}

fn criteria_plan() -> Value {
    json!({"op": "plan", "plan": {"check": "cargo test", "nodes": [
        {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
         "criteria": ["the parser keeps v1 files"],
         "checks": [{"as": "review", "by": [{"agent": "codex", "model": "gpt-6-astra"}]}]},
        {"key": "docs", "title": "Docs", "kind": "research", "criteria": ["the guide says it"]},
    ]}})
}

const HEAD: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

/// The fixture checkout, where the `recipient` worker's pane is, at `head`.
fn checkout_at(runtime: &mut Runtime, head: &str) {
    let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
    checkout.head_oid = Some(head.into());
    if let Some(worktree) = checkout.worktree.as_mut() {
        worktree.head_sha = Some(head.into());
    }
}

/// One `record` of `line` on `node` as passed, with `extra`'s fields and an
/// evidence line unless `extra` gives its own.
fn record_of(node: &str, line: &str, extra: Value) -> Value {
    let mut command =
        json!({"op": "record", "node": node, "line": line, "pass": true, "evidence": "it ran"});
    for (key, value) in extra.as_object().unwrap() {
        command[key] = value.clone();
    }
    command
}

/// PRD mission-map-git-criteria B1-B10: a node is done only once every
/// line is met, each line is recorded by its owner alone, the node's worker
/// is never its reviewer, and the mission closes only with every node done
/// or dropped.
#[test]
fn a_node_is_done_only_once_its_owners_met_every_line() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let (planned, _) = command(&mut runtime, "sender", criteria_plan()).unwrap();
    let keys: Vec<&str> = planned["mission"]["nodes"][0]["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["c1", "check", "review"]);
    // The check command is a change node's line only.
    assert_eq!(
        planned["mission"]["nodes"][1]["criteria"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(&mut runtime);
    checkout_at(&mut runtime, HEAD);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "pr": 7}}),
    )
    .unwrap();
    let done = json!({"op": "node", "node": "fix", "patch": {"status": "done"}});
    // A pull request GitHub has not read yet is not known to pass its CI.
    let refused = command(&mut runtime, "sender", done.clone()).unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused),
        ("mission_criteria_unmet", Some("fix: c1, check, review, ci"))
    );
    let record = |line: &str, extra: Value| record_of("fix", line, extra);
    // Only the worker records its own lines and the check command.
    assert_eq!(
        command(&mut runtime, "reviewer", record("c1", json!({}))).unwrap_err(),
        "mission_criterion_not_yours"
    );
    assert_eq!(
        command(&mut runtime, "sender", record("c1", json!({}))).unwrap_err(),
        "mission_criterion_not_yours"
    );
    // Every record carries the line it rests on, a worker's and a
    // checker's alike (B5).
    for evidence in [json!({"evidence": "  "}), json!({"evidence": null})] {
        let mut blank = record("c1", json!({}));
        blank["evidence"] = evidence["evidence"].clone();
        if blank["evidence"].is_null() {
            blank.as_object_mut().unwrap().remove("evidence");
        }
        assert_eq!(
            command(&mut runtime, "recipient", blank).unwrap_err(),
            "mission_evidence_missing"
        );
    }
    assert!(
        command(
            &mut runtime,
            "recipient",
            record("c1", json!({"evidence": "test added"}))
        )
        .unwrap()
        .1
    );
    assert_eq!(
        command(&mut runtime, "recipient", record("check", json!({}))).unwrap_err(),
        "mission_commit_required"
    );
    command(
        &mut runtime,
        "recipient",
        record("check", json!({"commit": &HEAD[..7]})),
    )
    .unwrap();
    // The worker never reviews its own node, nor certifies the review.
    assert_eq!(
        command(&mut runtime, "recipient", record("review", json!({}))).unwrap_err(),
        "mission_criterion_not_yours"
    );
    let bind = |id: &str| json!({"op": "node", "node": "fix", "patch": {"reviewer": id}});
    assert_eq!(
        crate::coordination::split_refusal(
            &command(&mut runtime, "sender", bind("agent-2")).unwrap_err()
        )
        .0,
        "mission_checker_is_worker"
    );
    assert_eq!(
        command(&mut runtime, "reviewer", record("review", json!({}))).unwrap_err(),
        "mission_criterion_not_yours"
    );
    command(&mut runtime, "sender", bind("agent-3")).unwrap();
    // A bound reviewer is never made the node's worker.
    assert_eq!(
        crate::coordination::split_refusal(
            &command(
                &mut runtime,
                "sender",
                json!({"op": "node", "node": "fix", "patch": {"agent_id": "agent-3"}})
            )
            .unwrap_err()
        )
        .0,
        "mission_checker_is_worker"
    );
    // The bound checker is shown where it works (B29).
    let view = runtime.mission_view_of("mission-1").unwrap();
    let review = view.nodes[0]
        .criteria
        .iter()
        .find(|line| line.key == "review")
        .unwrap();
    assert_eq!(
        (
            review.checkers[0].pane_id.as_deref(),
            review.checkers[0].machine.as_deref()
        ),
        (Some("reviewer"), Some(crate::node::TEST_NODE))
    );
    assert_eq!(
        command(
            &mut runtime,
            "reviewer",
            record("review", json!({"evidence": " "}))
        )
        .unwrap_err(),
        "mission_evidence_missing"
    );
    command(&mut runtime, "reviewer", record("review", json!({}))).unwrap();
    // GitHub reads the pull request with its CI passing on that head.
    let read = github(vec![pull(7, HEAD, PullRequestBadge::Open, None)], false);
    runtime.github = read.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
    let (shown, _) = command(&mut runtime, "sender", done.clone()).unwrap();
    let node = &shown["mission"]["nodes"][0];
    assert_eq!(node["status"], "done");
    assert_eq!(node["criteria_met"], 4);
    let by: Vec<&str> = node["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["by"]["role"].as_str().unwrap())
        .collect();
    assert_eq!(by, ["worker", "worker", "hide", "checker"]);

    // A commit in the worktree after the check passed opens the line again
    // (B7).
    let next = "f".repeat(40);
    checkout_at(&mut runtime, &next);
    let view = runtime.mission_view_of("mission-1").unwrap();
    let check = view.nodes[0]
        .criteria
        .iter()
        .find(|line| line.key == "check")
        .unwrap();
    assert_eq!(check.state, crate::mission_criteria::CriterionState::Open);

    // The mission closes as done only once every node is done or dropped,
    // and every line of a node already done is still met.
    let close = json!({"op": "close", "end": "done"});
    assert_eq!(
        crate::coordination::split_refusal(
            &command(&mut runtime, "sender", close.clone()).unwrap_err()
        ),
        ("mission_nodes_unfinished", Some("docs"))
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "docs", "patch": {"status": "dropped"}}),
    )
    .unwrap();
    assert_eq!(
        crate::coordination::split_refusal(
            &command(&mut runtime, "sender", close.clone()).unwrap_err()
        ),
        ("mission_criteria_unmet", Some("fix: check"))
    );
    command(
        &mut runtime,
        "recipient",
        record("check", json!({"commit": next})),
    )
    .unwrap();
    command(&mut runtime, "sender", close).unwrap();
}

/// B11, B12, B21: the Conductor goes past a line only with a reason, past
/// the check command only on an answered decision about the node, and a
/// later node's defect opens the node again with the defect as a line.
#[test]
fn a_skip_needs_its_reason_and_the_check_command_the_operators_answer() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(&mut runtime);
    let skip = |line: &str, reason: &str, decision: Option<u64>| {
        let mut command = json!({"op": "skip", "node": "fix", "line": line, "reason": reason});
        if let Some(decision) = decision {
            command["decision"] = json!(decision);
        }
        command
    };
    assert_eq!(
        command(&mut runtime, "sender", skip("review", " ", None)).unwrap_err(),
        "mission_skip_reason_missing"
    );
    assert_eq!(
        command(
            &mut runtime,
            "recipient",
            skip("review", "small change", None)
        )
        .unwrap_err(),
        "mission_criterion_not_yours"
    );
    assert!(
        command(&mut runtime, "sender", skip("review", "small change", None))
            .unwrap()
            .1
    );
    // The same skip again changes nothing.
    assert!(
        !command(&mut runtime, "sender", skip("review", "small change", None))
            .unwrap()
            .1
    );
    let protected = |runtime: &mut Runtime, decision: Option<u64>| {
        crate::coordination::split_refusal(
            &command(
                runtime,
                "sender",
                skip("check", "fails on main too", decision),
            )
            .unwrap_err(),
        )
        .0
        .to_owned()
    };
    assert_eq!(protected(&mut runtime, None), "mission_criteria_protected");
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Skip the failing check?", "node": "fix"}),
    )
    .unwrap();
    // Not yet answered.
    assert_eq!(
        protected(&mut runtime, Some(1)),
        "mission_criteria_protected"
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    let (shown, _) = command(
        &mut runtime,
        "sender",
        skip("check", "fails on main too", Some(1)),
    )
    .unwrap();
    let check = &shown["mission"]["nodes"][0]["criteria"][1];
    assert_eq!(
        (&check["state"], &check["skip"]["decision"]),
        (&json!("skipped"), &json!(1))
    );

    // A defect the docs node found opens the fix again.
    let defect =
        json!({"op": "defect", "node": "fix", "found_by": "docs", "line": "drops a v1 node"});
    let (shown, changed) = command(&mut runtime, "sender", defect.clone()).unwrap();
    assert!(changed);
    let node = &shown["mission"]["nodes"][0];
    assert_eq!(
        (&node["status"], &node["reopened"]),
        (&json!("working"), &json!(1))
    );
    let last = node["criteria"].as_array().unwrap().last().unwrap();
    assert_eq!(
        (&last["key"], &last["found_by"], &last["state"]),
        (&json!("d1"), &json!("docs"), &json!("open"))
    );
    assert!(!command(&mut runtime, "sender", defect).unwrap().1);
}

/// D-19, B23: a mission made before criteria is planned, finished and
/// closed as before.
#[test]
fn a_mission_made_before_criteria_is_not_gated() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let bare = json!({"op": "plan", "plan": {"nodes": [
        {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr"},
    ]}});
    assert_eq!(
        crate::coordination::split_refusal(
            &command(&mut runtime, "sender", bare.clone()).unwrap_err()
        ),
        ("mission_criteria_missing", Some("fix"))
    );
    without_gate(&mut runtime);
    command(&mut runtime, "sender", bare).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done", "pr": 7}}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    assert!(runtime.mission_view_of("mission-1").unwrap().exempt);
}

fn outside_plan() -> Value {
    json!({"op": "plan", "plan": {"nodes": [
        {"key": "parser", "title": "The parser PR", "kind": "external", "external": {"pr": 1232}},
        {"key": "lib", "title": "The library branch", "kind": "external", "external": {"branch": "feat/lib"}},
        {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
         "criteria": ["it works"], "depends_on": ["parser", "lib"]},
    ]}})
}

fn node_view(runtime: &Runtime, key: &str) -> crate::runtime::missions::NodeView {
    runtime
        .mission_view_of("mission-1")
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.key == key)
        .unwrap()
}

/// PRD mission-map-git-criteria B19, B20: an outside pull request is
/// followed to main from the project's list, an outside branch from its
/// read, and one Hide cannot read waits and says so; the Conductor cannot
/// finish either, and their arrival in main wakes it.
#[test]
fn an_outside_node_is_done_once_its_pull_request_or_branch_is_in_main() {
    use crate::missions::{External, WakeEvent};
    use crate::runtime::mission_outside::{Ask, Found, Watch};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", outside_plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    let node = crate::node::TEST_NODE;

    runtime.note_mission_pulls(
        node,
        &github(vec![pull(1232, "a1", PullRequestBadge::Open, None)], false),
    );
    assert_eq!(node_view(&runtime, "parser").status, NodeStatus::Waiting);
    let refused = command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "parser", "patch": {"status": "done"}}),
    )
    .unwrap_err();
    assert!(refused.contains("mission_external_invalid"), "{refused}");

    runtime.note_mission_pulls(
        node,
        &github(
            vec![pull(1232, "a1", PullRequestBadge::Merged, None)],
            false,
        ),
    );
    assert_eq!(node_view(&runtime, "parser").status, NodeStatus::Done);
    let last = runtime.missions.book.missions[0]
        .wakes
        .last()
        .cloned()
        .unwrap();
    assert_eq!(
        (last.node.as_str(), last.pr, last.event),
        ("parser", Some(1232), WakeEvent::OutsideDone)
    );

    let place = (node.to_owned(), PROJECT.to_owned());
    let lib = || Ask {
        mission: "mission-1".into(),
        node: "lib".into(),
        watch: Watch::Outside(External::Branch("feat/lib".into())),
    };
    runtime.ingest_outside(
        &place,
        vec![(
            lib(),
            Found::Unread("The branch feat/lib is not in this repository".into()),
        )],
    );
    let read = node_view(&runtime, "lib");
    assert_eq!((read.status, read.unread), (NodeStatus::Waiting, true));
    runtime.ingest_outside(&place, vec![(lib(), Found::InMain(Some(HEAD.into())))]);
    let read = node_view(&runtime, "lib");
    assert_eq!((read.status, read.unread), (NodeStatus::Done, false));
    let last = runtime.missions.book.missions[0]
        .wakes
        .last()
        .cloned()
        .unwrap();
    assert_eq!(
        (last.node.as_str(), last.pr, last.event),
        ("lib", None, WakeEvent::OutsideDone)
    );
}

/// PRD mission-map-git-criteria B20: an outside pull request closed
/// without a merge fails, the node waiting on it reads blocked, and the
/// mission cannot close as done until the node is dropped.
#[test]
fn an_outside_pull_request_closed_without_a_merge_blocks_the_nodes_after_it() {
    use crate::missions::WakeEvent;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", outside_plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    runtime.note_mission_pulls(
        crate::node::TEST_NODE,
        &github(
            vec![pull(1232, "a1", PullRequestBadge::Closed, None)],
            false,
        ),
    );
    assert_eq!(node_view(&runtime, "parser").status, NodeStatus::Failed);
    assert_eq!(
        node_view(&runtime, "fix").blocked,
        Some(crate::missions::Blocked::UpstreamFailed)
    );
    let last = runtime.missions.book.missions[0]
        .wakes
        .last()
        .cloned()
        .unwrap();
    assert_eq!(last.event, WakeEvent::OutsideFailed);
    let refused = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap_err();
    assert!(refused.contains("mission_nodes_unfinished"), "{refused}");
}

fn criteria_plan_checked_by(check: &str) -> Value {
    let mut plan = criteria_plan();
    plan["plan"]["check"] = json!(check);
    plan
}

fn running_criteria_mission(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(runtime, "sender", criteria_plan()).unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(runtime);
    checkout_at(runtime, HEAD);
}

/// The fix node of `running_criteria_mission` with its pull request 7 at
/// `HEAD`, every line recorded by its owner and GitHub's CI passing.
fn met_fix(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "pr": 7, "reviewer": "agent-3"}}),
    )
    .unwrap();
    command(runtime, "recipient", record_of("fix", "c1", json!({}))).unwrap();
    command(
        runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    command(runtime, "reviewer", record_of("fix", "review", json!({}))).unwrap();
    let read = github(vec![pull(7, HEAD, PullRequestBadge::Open, None)], false);
    runtime.github = read.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
}

fn line_state(runtime: &Runtime, key: &str) -> crate::mission_criteria::CriterionState {
    node_view(runtime, "fix")
        .criteria
        .into_iter()
        .find(|line| line.key == key)
        .unwrap()
        .state
}

/// Review findings 1 and 2: one change that brings a failing pull request
/// along with done is gated as the node it would leave, and a mission
/// waiting on its merges closes itself only while every done node still
/// meets its lines.
#[test]
fn a_waiting_mission_closes_itself_only_while_its_done_nodes_meet_every_line() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    let mut failing = pull(8, HEAD, PullRequestBadge::Open, None);
    failing.checks = PullRequestChecks::Failed;
    let read = github(
        vec![pull(7, HEAD, PullRequestBadge::Open, None), failing],
        false,
    );
    runtime.github = read.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
    let refused = command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done", "pr": 8}}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused),
        ("mission_criteria_unmet", Some("fix: ci"))
    );
    assert_eq!(node_view(&runtime, "fix").pr, Some(7));
    for (node, status) in [("fix", "done"), ("docs", "dropped")] {
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"status": status}}),
        )
        .unwrap();
    }
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    let status = |runtime: &Runtime| runtime.missions.book.get("mission-1").unwrap().status;
    assert_eq!(status(&runtime), MissionStatus::MergeWait);

    // The pull request is merged with a commit no check ran on.
    let late = "e".repeat(40);
    let merged = github(vec![pull(7, &late, PullRequestBadge::Merged, None)], false);
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);
    assert_eq!(status(&runtime), MissionStatus::MergeWait);

    // The worker runs the check on the merged commit, and the next read
    // closes the mission.
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": late})),
    )
    .unwrap();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    assert_eq!(status(&runtime), MissionStatus::Done);
}

/// Review findings 3 and 4: a passed check holds only while Hide reads the
/// folder it ran in at that commit, whoever works there, and a replan
/// that changes the check command asks for it again on the same commit.
#[test]
fn a_check_holds_only_on_its_folders_current_commit_and_its_command() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);

    // The same command again keeps the line; another one opens it.
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
    command(
        &mut runtime,
        "sender",
        criteria_plan_checked_by("cargo test --workspace"),
    )
    .unwrap();
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);

    // The worker ending leaves the folder, and its head, known.
    let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
    ledger
        .agents
        .iter_mut()
        .find(|agent| agent.id == "agent-2")
        .unwrap()
        .ended = true;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);

    // A folder Hide no longer reads has no head, and nothing passes on it.
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    mission.nodes[0].folder = Some("/checkouts/fixture-removed".into());
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);
    let refused = command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done"}}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused),
        ("mission_criteria_unmet", Some("fix: check"))
    );
}

/// Review finding 5: a pull request's CI gets one rerun per head however
/// many runs GitHub reports, and a spent rerun sends a node in review back
/// to its worker.
#[test]
fn a_failed_ci_rerun_sends_the_node_back_to_work_and_reads_no_more_runs_on_that_head() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "review"}}),
    )
    .unwrap();
    let read = |runtime: &mut Runtime, checks| {
        let mut pull = pull(7, HEAD, PullRequestBadge::Open, None);
        pull.checks = checks;
        let read = github(vec![pull], false);
        runtime.github = read.clone();
        runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
    };
    read(&mut runtime, PullRequestChecks::Failed);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Fail);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Review);
    // The rerun runs and passes: the line passes with its rerun counted.
    read(&mut runtime, PullRequestChecks::Pending);
    read(&mut runtime, PullRequestChecks::Passing);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Pass);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Review);
    // Another run on the same head fails: there is no second rerun.
    read(&mut runtime, PullRequestChecks::Pending);
    read(&mut runtime, PullRequestChecks::Failed);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Fail);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    // A later pass on that head is not read; a new head starts over.
    read(&mut runtime, PullRequestChecks::Passing);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Fail);
}

/// Review finding 6: a mission whose change reaches main as its branch,
/// with no GitHub, waits after its close until Hide reads that branch in
/// main, and then closes itself with the commit it found.
#[test]
fn a_branch_mission_waits_for_its_branch_to_reach_main_then_closes() {
    use crate::runtime::mission_outside::Found;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "direct", "criteria": ["it works"]},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done", "branch": "feat/fix"}}),
    )
    .unwrap();
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "merge_wait");
    let status = |runtime: &Runtime| runtime.missions.book.get("mission-1").unwrap().status;
    land(&mut runtime, "fix", Found::Open);
    assert_eq!(status(&runtime), MissionStatus::MergeWait);
    land(&mut runtime, "fix", Found::InMain(Some(HEAD.into())));
    assert_eq!(status(&runtime), MissionStatus::Done);
    assert_eq!(node_view(&runtime, "fix").in_main.as_deref(), Some(HEAD));
}

/// The branch read Hide asks for `node` of the waiting mission now.
fn landing_ask(runtime: &Runtime, node: &str) -> crate::runtime::mission_outside::Ask {
    use crate::runtime::mission_outside::{Ask, Watch};
    let mission = runtime.missions.book.get("mission-1").unwrap();
    let row = mission.node(node).unwrap();
    Ask {
        mission: "mission-1".into(),
        node: node.into(),
        watch: Watch::Landing {
            branch: row.branch.clone(),
            reopens: runtime.missions.outside.reopens_of("mission-1"),
            done: row.finished_at_unix_ms,
        },
    }
}

/// `found` as the answer of a branch read for `node` asked now.
fn land(runtime: &mut Runtime, node: &str, found: crate::runtime::mission_outside::Found) {
    let ask = landing_ask(runtime, node);
    answer(runtime, ask, found);
}

fn answer(
    runtime: &mut Runtime,
    ask: crate::runtime::mission_outside::Ask,
    found: crate::runtime::mission_outside::Found,
) {
    let place = (crate::node::TEST_NODE.to_owned(), PROJECT.to_owned());
    runtime.ingest_outside(&place, vec![(ask, found)]);
}

/// A running mission of two direct changes, `a` on `feat/a` worked by
/// `recipient` and `b` on `feat/b` worked by `reviewer`, each done with its
/// line met and waiting on its branch.
fn waiting_on_branches(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Two branches"}),
    )
    .unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "a", "title": "A", "kind": "change", "delivery": "direct", "criteria": ["a works"]},
            {"key": "b", "title": "B", "kind": "change", "delivery": "direct", "criteria": ["b works"]},
        ]}}),
    )
    .unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(runtime);
    for (node, agent, pane) in [("a", "agent-2", "recipient"), ("b", "agent-3", "reviewer")] {
        command(
            runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"status": "working", "agent_id": agent, "branch": format!("feat/{node}")}}),
        )
        .unwrap();
        command(runtime, pane, record_of(node, "c1", json!({}))).unwrap();
        command(
            runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"status": "done"}}),
        )
        .unwrap();
    }
    let (closed, _) = command(
        runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "merge_wait");
}

fn mission_status(runtime: &Runtime) -> MissionStatus {
    runtime.missions.book.get("mission-1").unwrap().status
}

/// Re-review finding 1: a waiting mission closes by itself, on its pull
/// request merged, only by the rule the Conductor's close is held to: a
/// node a defect sent back to work, or a node failed, keeps it waiting.
#[test]
fn a_mission_waiting_on_its_pull_requests_keeps_waiting_while_a_node_is_back_at_work_or_failed() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    for (node, status) in [("fix", "done"), ("docs", "dropped")] {
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"status": status}}),
        )
        .unwrap();
    }
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "defect", "node": "fix", "found_by": "docs", "line": "drops a v1 node"}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    let merged = github(vec![pull(7, HEAD, PullRequestBadge::Merged, None)], false);
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "failed"}}),
    )
    .unwrap();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
}

/// Re-review finding 1: the same rule holds the close a waiting mission
/// makes on its branches in main.
#[test]
fn a_mission_waiting_on_its_branches_keeps_waiting_while_a_node_is_back_at_work_or_failed() {
    use crate::runtime::mission_outside::Found;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on_branches(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "defect", "node": "b", "found_by": "a", "line": "b drops a field"}),
    )
    .unwrap();
    land(&mut runtime, "a", Found::InMain(Some(HEAD.into())));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "b", "patch": {"status": "failed"}}),
    )
    .unwrap();
    land(&mut runtime, "a", Found::InMain(Some(HEAD.into())));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
}

/// Re-review finding 2: a branch read in main proves that branch, at that
/// node's finish, in that wait: a node moved to another branch, a branch
/// that moved on, a reopened mission and an answer asked before any of
/// them prove nothing.
#[test]
fn a_branch_read_in_main_proves_only_that_branch_while_its_node_and_wait_stand() {
    use crate::runtime::mission_outside::Found;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on_branches(&mut runtime);
    let (a1, b1, c1) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
    land(&mut runtime, "a", Found::InMain(Some(a1.clone())));
    assert_eq!(node_view(&runtime, "a").in_main, Some(a1.clone()));
    let before_move = landing_ask(&runtime, "a");

    // The Conductor moves `a` to another branch: what was read of the old
    // one proves nothing, then or when its late answer comes.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "a", "patch": {"branch": "feat/c"}}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "a").in_main, None);
    answer(&mut runtime, before_move, Found::InMain(Some(a1)));
    land(&mut runtime, "b", Found::InMain(Some(b1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    assert_eq!(node_view(&runtime, "a").in_main, None);

    // A branch that moved on past main is no longer in it.
    land(&mut runtime, "b", Found::Open);
    assert_eq!(node_view(&runtime, "b").in_main, None);
    land(&mut runtime, "b", Found::InMain(Some(b1.clone())));
    land(&mut runtime, "a", Found::InMain(Some(c1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::Done);

    // Reopened, the same branches are read again before it closes, and an
    // answer asked in the earlier wait proves nothing in the next.
    let earlier = landing_ask(&runtime, "a");
    command(&mut runtime, "sender", json!({"op": "reopen"})).unwrap();
    assert_eq!(
        (
            node_view(&runtime, "a").in_main,
            node_view(&runtime, "b").in_main
        ),
        (None, None)
    );
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "merge_wait");
    land(&mut runtime, "b", Found::InMain(Some(b1)));
    answer(&mut runtime, earlier, Found::InMain(Some(c1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    assert_eq!(node_view(&runtime, "a").in_main, None);
    land(&mut runtime, "a", Found::InMain(Some(c1)));
    assert_eq!(mission_status(&runtime), MissionStatus::Done);
}

/// Third re-review finding 1: a branch read that could not answer proves
/// nothing, as the branch may have moved past main unseen, so what an
/// earlier read found in main closes the mission neither by itself nor by
/// the Conductor's close; the node shows unread until a read finds it in
/// main again.
#[test]
fn a_branch_that_could_not_be_read_again_keeps_its_mission_waiting_until_read_in_main() {
    use crate::runtime::mission_outside::Found;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on_branches(&mut runtime);
    let (a1, b1) = ("a".repeat(40), "b".repeat(40));
    land(&mut runtime, "a", Found::InMain(Some(a1.clone())));
    assert_eq!(node_view(&runtime, "a").in_main, Some(a1.clone()));

    // The next read cannot see `a` while `b` reaches main.
    land(
        &mut runtime,
        "a",
        Found::Unread("The branch feat/a is not in this repository or its last fetch".into()),
    );
    land(&mut runtime, "b", Found::InMain(Some(b1)));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    let a = node_view(&runtime, "a");
    assert_eq!((a.in_main, a.unread), (None, true));
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "merge_wait");

    land(&mut runtime, "a", Found::InMain(Some(a1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::Done);
    let a = node_view(&runtime, "a");
    assert_eq!((a.in_main, a.unread), (Some(a1), false));
}

/// This machine's host answering a node's calls, as the core's link to it
/// does.
struct Host(hide_host::serve::Env);

impl crate::node_access::NodeLink for Host {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        _: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        hide_host::serve::handle_in(call, &self.0)
            .map(crate::node_access::LinkAnswer::Parsed)
            .map_err(crate::node_access::LinkError::Refused)
    }
}

fn git_in(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Review finding 14: in a repository with no remote, a git mission's main
/// is the branch the repository's main worktree has out. An outside branch
/// merged there is done, a waiting mission's own branch keeps it waiting
/// until it is merged too, and the commit found is the one in main.
#[test]
fn a_git_mission_in_a_repository_with_no_remote_follows_its_branches_into_main() {
    use crate::missions::WakeEvent;
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
        &["branch", "feat/lib"],
        &["branch", "feat/fix"],
    ] {
        git_in(&repo, args);
    }
    for branch in ["feat/lib", "feat/fix"] {
        git_in(&repo, &["checkout", "-q", branch]);
        git_in(&repo, &["commit", "-q", "--allow-empty", "-m", branch]);
    }
    git_in(&repo, &["checkout", "-q", "trunk"]);
    git_in(&repo, &["merge", "-q", "--no-ff", "-m", "lib", "feat/lib"]);
    let listed = hide_host::worktrees::read(
        &repo,
        &Default::default(),
        None,
        None,
        &hide_host::worktrees::Measured::default(),
        false,
    )
    .unwrap();
    assert_eq!(listed.base_branch.as_deref(), Some("trunk"));
    let root = hide_platform::path::to_wire(&repo).unwrap();

    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "lib", "title": "The library branch", "kind": "external", "external": {"branch": "feat/lib"}},
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "direct",
             "criteria": ["it works"], "depends_on": ["lib"]},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done", "branch": "feat/fix"}}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    // The mission's project is the repository on disk.
    runtime.missions.book.missions[0].project = root.clone();
    runtime
        .worktree_catalog
        .projects
        .push(crate::model::ProjectWorktreesSnapshot {
            root_path: root.clone(),
            base_branch: listed.base_branch.clone(),
            ..Default::default()
        });
    let host = Host(hide_host::serve::Env::standalone(Some(repo.clone())));
    let device = crate::node::TEST_NODE;
    let status = |runtime: &Runtime| runtime.missions.book.get("mission-1").unwrap().status;

    runtime.read_outside_through(device, &root, &host);
    let lib = node_view(&runtime, "lib");
    assert_eq!((lib.status, lib.unread), (NodeStatus::Done, false));
    let last = runtime.missions.book.missions[0]
        .wakes
        .last()
        .cloned()
        .unwrap();
    assert_eq!(
        (last.node.as_str(), last.event),
        ("lib", WakeEvent::OutsideDone)
    );
    assert_eq!(status(&runtime), MissionStatus::MergeWait);
    assert_eq!(runtime.missions.book.missions[0].nodes[1].in_main, None);

    git_in(&repo, &["merge", "-q", "--no-ff", "-m", "fix", "feat/fix"]);
    runtime.read_outside_through(device, &root, &host);
    assert_eq!(status(&runtime), MissionStatus::Done);
    // The commit found in main is what `hide mission show` and the
    // snapshot give the map for `→ main <sha7>`, beside a commit the
    // Conductor never recorded.
    let landed = git_in(&repo, &["rev-parse", "feat/fix"]);
    let (shown, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "show", "mission": "mission-1"}),
    )
    .unwrap();
    let fix = &shown["mission"]["nodes"][1];
    assert_eq!(
        (&fix["in_main"], &fix["commit"]),
        (&json!(landed), &Value::Null)
    );
    runtime.refresh_missions();
    let snapshot = serde_json::to_value(&runtime.snapshot.missions).unwrap();
    let published = snapshot["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mission| mission["id"] == "mission-1")
        .unwrap();
    assert_eq!(published["nodes"][1]["in_main"], json!(landed));
}

/// verify-screen's finding on #1729: a direct change already in main is
/// read there as soon as its node is done, while the mission still runs, so
/// the map shows `→ main` and not `합치기 남음`; the node turning done starts
/// the read through the worker and this machine's node with no other
/// trigger, a running mission does not close on it, and a close as
/// `merge_wait` after it is done at once.
#[test]
fn a_direct_change_already_in_main_is_read_there_when_it_is_done_while_its_mission_runs() {
    use std::sync::mpsc;
    use std::time::Instant;
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
        &["checkout", "-q", "-b", "land-a"],
        &["commit", "-q", "--allow-empty", "-m", "land-a"],
        &["checkout", "-q", "trunk"],
        &["merge", "-q", "--no-ff", "-m", "land-a", "land-a"],
    ] {
        git_in(&repo, args);
    }
    let landed = git_in(&repo, &["rev-parse", "land-a"]);
    let root = hide_platform::path::to_wire(&repo).unwrap();

    let fixture = conducting();
    let (changed, changes) = mpsc::channel();
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        runtime.install_worker_context(
            Arc::downgrade(&fixture.runtime),
            ChangeNotifier::announcing(move || {
                let _ = changed.send(());
            }),
        );
        runtime.own_node = Arc::new(hide_node::Local::new(Some(repo.clone())));
        runtime
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: root.clone(),
                base_branch: Some("trunk".into()),
                ..Default::default()
            });
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Land a"}),
        )
        .unwrap();
        runtime.missions.book.missions[0].project = root.clone();
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "a", "title": "A", "kind": "change", "delivery": "direct", "criteria": ["a works"]},
            ]}}),
        )
        .unwrap();
        command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
        with_worker_and_reviewer(&mut runtime);
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": "a", "patch": {"status": "working", "agent_id": "agent-2", "branch": "land-a"}}),
        )
        .unwrap();
        command(&mut runtime, "recipient", record_of("a", "c1", json!({}))).unwrap();
        assert_eq!(node_view(&runtime, "a").in_main, None);
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": "a", "patch": {"status": "done"}}),
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        {
            let runtime = fixture.runtime.lock().unwrap();
            if node_view(&runtime, "a").in_main.is_some() {
                break;
            }
        }
        changes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the done branch was read before the deadline");
    }
    let mut runtime = fixture.runtime.lock().unwrap();
    let (shown, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "show", "mission": "mission-1"}),
    )
    .unwrap();
    assert_eq!(shown["mission"]["nodes"][0]["in_main"], json!(landed));
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    let (closed, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    assert_eq!(closed["mission"]["status"], "done");
}

/// Review finding 7 on the live path, as verify-core ran it: a plan of 32
/// branches Hide cannot find and one in main is read when the plan is
/// admitted, through the read worker and this machine's node, and the pass
/// goes on at once past the first 32 read, so the 33rd is done with no
/// GitHub read, fetch or catalog change to start it.
#[test]
fn a_plan_of_more_outside_branches_than_one_read_takes_is_read_whole_when_admitted() {
    use std::sync::mpsc;
    use std::time::Instant;
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
    ] {
        git_in(&repo, args);
    }
    let listed = hide_host::worktrees::read(
        &repo,
        &Default::default(),
        None,
        None,
        &hide_host::worktrees::Measured::default(),
        false,
    )
    .unwrap();
    let root = hide_platform::path::to_wire(&repo).unwrap();

    let fixture = conducting();
    let (changed, changes) = mpsc::channel();
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        runtime.install_worker_context(
            Arc::downgrade(&fixture.runtime),
            ChangeNotifier::announcing(move || {
                let _ = changed.send(());
            }),
        );
        runtime.own_node = Arc::new(hide_node::Local::new(Some(repo.clone())));
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Many outside branches"}),
        )
        .unwrap();
        runtime.missions.book.missions[0].project = root.clone();
        runtime
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: root.clone(),
                base_branch: listed.base_branch.clone(),
                ..Default::default()
            });
        let mut nodes: Vec<Value> = (0..32)
            .map(|index| {
                json!({"key": format!("e{index}"), "title": format!("outside {index}"),
                       "kind": "external", "external": {"branch": format!("absent-{index}")}})
            })
            .collect();
        nodes.push(
            json!({"key": "last", "title": "In main", "kind": "external",
                          "external": {"branch": "trunk"}}),
        );
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": nodes}}),
        )
        .unwrap();
    }
    // Each answer the worker brings back announces itself; the deadline is
    // only how long a broken read is waited for.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        {
            let runtime = fixture.runtime.lock().unwrap();
            if node_view(&runtime, "last").status == NodeStatus::Done {
                for index in 0..32 {
                    let node = node_view(&runtime, &format!("e{index}"));
                    assert_eq!((node.status, node.unread), (NodeStatus::Waiting, true));
                }
                assert!(
                    runtime
                        .missions
                        .outside
                        .idle_for_test(crate::node::TEST_NODE, &root)
                );
                break;
            }
        }
        changes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the outside reads answered before the deadline");
    }
}

/// This machine's node, holding every call while it is shut and telling
/// `entered` when the first one arrives.
struct Held {
    node: hide_node::Local,
    open: std::sync::Mutex<bool>,
    opened: std::sync::Condvar,
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl Held {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }
}

impl crate::node_access::NodeLink for Held {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
        drop(open);
        crate::node_access::NodeLink::call(&self.node, call, timeout)
    }
}

/// Third re-review finding 2: a plan admitted while the project's read is
/// out is read once that read is answered, with no GitHub read, fetch or
/// worktree change after it; the first read's own unread answer starts no
/// read of its own.
#[test]
fn a_node_followed_while_a_read_is_out_is_read_once_that_read_answers() {
    use std::sync::mpsc;
    use std::time::Instant;
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
    ] {
        git_in(&repo, args);
    }
    let root = hide_platform::path::to_wire(&repo).unwrap();
    let (entered, first_call) = mpsc::channel();
    let held = Arc::new(Held {
        node: hide_node::Local::new(Some(repo.clone())),
        open: std::sync::Mutex::new(false),
        opened: std::sync::Condvar::new(),
        entered: std::sync::Mutex::new(Some(entered)),
    });

    let fixture = conducting();
    let (changed, changes) = mpsc::channel();
    let outside = |key: &str, branch: &str| json!({"key": key, "title": key, "kind": "external", "external": {"branch": branch}});
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        runtime.install_worker_context(
            Arc::downgrade(&fixture.runtime),
            ChangeNotifier::announcing(move || {
                let _ = changed.send(());
            }),
        );
        runtime.own_node = held.clone();
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Followed while reading"}),
        )
        .unwrap();
        runtime.missions.book.missions[0].project = root.clone();
        runtime
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: root.clone(),
                base_branch: Some("trunk".into()),
                ..Default::default()
            });
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [outside("first", "absent")]}}),
        )
        .unwrap();
    }
    first_call
        .recv_timeout(Duration::from_secs(60))
        .expect("the plan's read reached the node");
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [outside("first", "absent"), outside("last", "trunk")]}}),
        )
        .unwrap();
        assert_eq!(node_view(&runtime, "last").status, NodeStatus::Planned);
    }
    held.open();

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        {
            let runtime = fixture.runtime.lock().unwrap();
            if node_view(&runtime, "last").status == NodeStatus::Done {
                let first = node_view(&runtime, "first");
                assert_eq!((first.status, first.unread), (NodeStatus::Waiting, true));
                assert!(
                    runtime
                        .missions
                        .outside
                        .idle_for_test(crate::node::TEST_NODE, &root)
                );
                break;
            }
        }
        changes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the node planned during the read was read before the deadline");
    }
}

/// The `sender` pane's row at rest, a Codex session `sender-session`.
fn conductor_row() -> crate::model::SidebarAgentSnapshot {
    let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [
        {"pane_id": "sender", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-sender",
         "revision": 1, "focused": false, "agent_status": "idle", "agent": "codex",
         "name": "sender", "cwd": PROJECT, "state_change_seq": 1},
    ]}))
    .unwrap();
    let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
    row.session_id = Some("sender-session".into());
    row
}

fn conductor_shown(runtime: &mut Runtime) -> crate::model::SidebarAgentSnapshot {
    runtime.snapshot_delta_payload(0);
    runtime.snapshot.navigator.agents[0].clone()
}

#[test]
fn a_missions_open_decision_waits_on_the_operator_on_its_resting_conductors_row() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    let resting = conductor_shown(&mut runtime);
    assert_ne!(resting.group, "needs_you");

    // B1: the decision stands on the row as a question it carries the line of.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Skip the check line?"}),
    )
    .unwrap();
    let asking = conductor_shown(&mut runtime);
    let decision = crate::model::AskSubject::MissionDecision {
        line: "Skip the check line?".into(),
    };
    assert_eq!(
        (
            asking.group.as_str(),
            asking.ask_subject.clone(),
            asking.detail.as_deref()
        ),
        (
            "needs_you",
            Some(decision.clone()),
            Some("Skip the check line?")
        )
    );
    let wire = serde_json::to_value(&asking).unwrap();
    assert_eq!(
        wire["state"]["ask"]["subject"],
        json!({"cause": "mission_decision", "line": "Skip the check line?"})
    );

    // B3: a new decision replaces the line.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert_eq!(
        conductor_shown(&mut runtime).ask_subject,
        Some(crate::model::AskSubject::MissionDecision {
            line: "Which base?".into()
        })
    );

    // B3: cleared, the row shows its own state again at once.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    let cleared = conductor_shown(&mut runtime);
    assert_eq!(
        (cleared.ask_subject, cleared.user_turn, cleared.group),
        (None, None, resting.group.clone())
    );

    // B2: a working Conductor shows its work; the decision waits for it to stop.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Ship it?"}),
    )
    .unwrap();
    let mut working = vec![conductor_row()];
    working[0].activity = "working".into();
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut working);
    assert!(working[0].ask_subject.is_none());

    // B3: a closed mission takes its decision off the row.
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    assert_eq!(conductor_shown(&mut runtime).ask_subject, None);
}
