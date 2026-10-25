//! A Conductor's mission (PRD software-factory-conductor): the record its
//! `hide mission` commands keep, a plan that opens nothing, the spawns its plan
//! admits, the session that takes it over, the wakes its pull requests send
//! and the record going with an archived session.

use super::*;
use crate::delivery::{Actor, worker::Worker};
use crate::handle::ChangeNotifier;
use crate::missions::{AnswerFailure, MissionCommand, MissionStatus, NodeStatus};
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
    let settings = &runtime.snapshot.ui_state.conductor_settings;
    assert_eq!(settings.concurrency, 6);
    assert_eq!(settings.default_scope, crate::missions::MissionScope::Merge);
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
        let mut node = json!({"key": "one", "title": "One", "kind": kind, "worker": worker});
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
            {"key": "api", "title": "API", "kind": "change", "delivery": "pr"},
            {"key": "screen", "title": "Screen", "kind": "change", "delivery": "pr", "depends_on": ["api"]},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    for (node, pr) in [("api", 7), ("screen", 8)] {
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
            {"key": "api", "title": "API", "kind": "change", "delivery": "pr"},
            {"key": "screen", "title": "Screen", "kind": "change", "delivery": "pr"},
            {"key": "docs", "title": "Docs", "kind": "change", "delivery": "pr"},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    for (node, pr, status) in [
        ("api", 7, "done"),
        ("screen", 8, "failed"),
        ("docs", 9, "done"),
    ] {
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
            {"key": "cause", "title": "Find the cause", "kind": "research"},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
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
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "direct"},
            {"key": "pr", "title": "Open it", "kind": "change", "delivery": "pr"},
            {"key": "check", "title": "Check it", "kind": "verify"},
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
