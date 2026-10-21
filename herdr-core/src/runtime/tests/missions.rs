//! A Conductor's mission (PRD software-factory-conductor): the record its
//! `hide mission` commands keep, the map opened once, the spawns its plan
//! admits, the session that takes it over, the wakes its pull requests send
//! and the record going with an archived session.

use super::*;
use crate::delivery::{Actor, worker::Worker};
use crate::handle::ChangeNotifier;
use crate::missions::{MissionCommand, MissionStatus, NodeStatus};
use crate::model::{
    DetailItem, GithubProjectSnapshot, GithubSnapshot, PullRequestBadge, PullRequestChecks,
    PullRequestSnapshot, ReviewDecision,
};
use crate::runtime::delivery::tests::{authority, fixture};
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
        {"key": "cause", "title": "Find the cause", "kind": "research"},
        {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr", "depends_on": ["cause"]},
    ]}})
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

fn spawn(machine: Option<&str>) -> crate::delivery::Command {
    crate::delivery::Command::Agents {
        command: crate::coordination::Command::Spawn {
            parent: Some("here".into()),
            machine: machine.map(str::to_owned),
            name: "worker".into(),
            intent: "mission-1:cause".into(),
            kind: "codex".into(),
            repo: PROJECT.into(),
            branch: "cause".into(),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            mission: None,
        },
    }
}

fn prepare_spawn(
    runtime: &mut Runtime,
    conductor: &Actor,
    machine: Option<&str>,
) -> Result<(), String> {
    runtime
        .prepare_delivery(
            crate::node::TEST_NODE,
            &conductor.pane_id,
            &authority(conductor).context,
            None,
            spawn(machine),
        )
        .map(drop)
}

#[test]
fn the_first_plan_opens_the_map_once_and_a_replan_does_not_open_it_again() {
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

    let (planned, _) = command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(planned["mission"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(mission_slot_items(&runtime), 1);
    assert!(runtime.snapshot.ui_state.detail.shown);

    // The operator closes the map; a replan leaves it closed.
    runtime.snapshot.ui_state.detail.items.clear();
    runtime.snapshot.ui_state.detail.shown = false;
    command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(mission_slot_items(&runtime), 0);

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
            .mission_spawn_admission(&conductor, None)
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
        .mission_spawn_admission(&conductor, None)
        .unwrap()
        .expect("a running mission's spawn is one of its workers");
    assert_eq!(
        (admission.mission.as_str(), admission.cap),
        ("mission-1", 2)
    );
    assert!(prepare_spawn(&mut runtime, &conductor, None).is_ok());

    // A closed mission no longer holds its Conductor's spawns.
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    assert!(
        runtime
            .mission_spawn_admission(&conductor, None)
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
        vec!["mission:mission-1:fix:7:new_head:b2"]
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
        vec!["mission:mission-1:fix:7:changes_requested:b2"]
    );

    // A merge of a done mission's pull request reopens it with the wake.
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    let merged = github(vec![pull(7, "b2", PullRequestBadge::Merged, None)], false);
    assert_eq!(
        runtime.note_mission_pulls(node, &merged),
        vec!["mission:mission-1:fix:7:merged:-"]
    );
    assert_eq!(
        runtime.missions.book.get("mission-1").unwrap().status,
        MissionStatus::Running
    );
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
    runtime.missions.book.missions[0].conductor.session =
        Some(crate::links::ledger::SessionIdentity {
            device_id: crate::node::TEST_NODE.into(),
            agent: "codex".into(),
            session_id: "native-1".into(),
        });
    assert_eq!(mission_slot_items(&runtime), 1);

    assert!(!runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), "native-2".into())]));
    assert!(runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), "native-1".into())]));
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
