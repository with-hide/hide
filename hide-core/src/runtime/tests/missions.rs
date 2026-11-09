//! A Conductor's mission (PRD software-factory-conductor): the record its
//! `hide mission` commands keep, a plan that opens nothing, the spawns its plan
//! admits, the session that takes it over, the wakes its pull requests send
//! and the record going with an archived session.

use super::*;
use crate::delivery::{Actor, worker::Worker};
use crate::handle::ChangeNotifier;
use crate::mission_criteria::CriterionState;
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

fn spawn(machine: Option<&str>, intent: &str, brief: Option<&str>) -> crate::delivery::Command {
    crate::delivery::Command::Agents {
        command: crate::coordination::Command::Spawn {
            pick: None,
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
            plan_node: None,
            plan: None,
            beside: false,
            profile: None,
        },
    }
}

/// A spawn of `kind` in the project with `launch`, on `machine`, naming no
/// profile.
fn worker<'a>(
    machine: Option<&'a str>,
    kind: &'a str,
    launch: &'a hide_agent_adapter::LaunchChoice,
) -> crate::runtime::missions::SpawnWorker<'a> {
    crate::runtime::missions::SpawnWorker {
        machine,
        kind,
        profile: None,
        repo: PROJECT,
        launch,
        finished: false,
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
    assert!(
        runtime.snapshot.ui_state.detail.items.is_empty(),
        "an empty mission opens nothing"
    );
    let detail = runtime.snapshot.ui_state.detail.clone();

    // The first plan shows on the Conductor pane's chip, which the shell
    // draws; the lane's 보기 slot stays as it was (mission-pane-chip B11).
    let (planned, _) = command(&mut runtime, "sender", plan()).unwrap();
    assert_eq!(planned["mission"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(runtime.snapshot.ui_state.detail, detail);

    command(&mut runtime, "sender", plan()).unwrap();
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
                &worker(None, "codex", &Default::default()),
                &mut None,
                "plain",
                false,
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
        crate::coordination::split_refusal(
            &prepare_spawn(&mut runtime, &conductor, Some("mini")).unwrap_err()
        ),
        ("mission_machine_node_refused", Some("cause")),
        "a research node's worker runs only on the machine the project is on"
    );
    let admission = runtime
        .mission_spawn_admission(
            &conductor,
            &worker(None, "codex", &Default::default()),
            &mut None,
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
                &worker(None, "codex", &Default::default()),
                &mut None,
                "plain",
                false,
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

/// 이어받기 resumes the gone Conductor's conversation in a new pane (PRD
/// mission-tool D-11, B14): once that pane is seen running the same
/// session, it conducts the mission without a `take`, and a pane running
/// another session is not mistaken for it.
#[test]
fn a_conductors_conversation_resumed_in_another_pane_conducts_its_mission() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    // The Conductor's session started with a profile, as its pane's record
    // keeps it (PRD agent-profiles D-27).
    let work = crate::launch::LaunchProfile {
        id: hide_agent_adapter::ProfileId("p-work".into()),
        name: "work".into(),
        account: hide_agent_adapter::AccountKey::Folder("/accounts/work".into()),
    };
    runtime.record_pane_launch(
        "sender",
        "codex",
        "sender",
        &Default::default(),
        Some(&work),
        None,
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    assert_eq!(
        runtime.missions.book.missions[0].conductor.profile,
        Some(work.clone())
    );

    // The Conductor's pane closed.
    runtime.delivery_observations.remove("sender");
    runtime.snapshot.navigator.agents.clear();
    runtime.refresh_missions();
    assert!(!runtime.snapshot.missions.items[0].conductor.present);

    let seen = |runtime: &mut Runtime, pane: &str, session: &str| {
        let mut row = conductor_row();
        row.pane_id = pane.into();
        row.session_id = Some(session.into());
        runtime.snapshot.navigator.agents = vec![row];
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id": pane, "pane_id": pane, "agent": "codex", "agent_status": "idle",
             "state_change_seq": 1, "lineage_session": format!("{pane}-lineage")},
        ]}))
        .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        runtime.refresh_missions();
    };
    seen(&mut runtime, "stranger", "another-session");
    assert!(!runtime.snapshot.missions.items[0].conductor.present);
    assert!(runtime.missions.book.missions[0].wakes.is_empty());

    seen(&mut runtime, "resumed", "sender-session");
    let conductor = &runtime.snapshot.missions.items[0].conductor;
    assert!(conductor.present);
    assert_eq!(conductor.pane_id, "resumed");
    // The pane it came back in keeps no record of its own yet: the record
    // keeps the profile the conversation started with, through a save and
    // a read back as a restart makes.
    assert_eq!(
        runtime.missions.book.missions[0].conductor.profile,
        Some(work.clone())
    );
    let saved = serde_json::to_value(&runtime.missions.book).unwrap();
    let read: crate::missions::MissionBook = serde_json::from_value(saved).unwrap();
    assert_eq!(read.missions[0].conductor.profile, Some(work.clone()));
    // A resumed conversation gets no prompt of its own: one letter waits
    // to tell it to go on, and seeing the pane again adds none.
    let events = |runtime: &Runtime| -> Vec<crate::missions::WakeEvent> {
        runtime.missions.book.missions[0]
            .wakes
            .iter()
            .map(|wake| wake.event)
            .collect()
    };
    assert_eq!(events(&runtime), [crate::missions::WakeEvent::Resumed]);
    seen(&mut runtime, "resumed", "sender-session");
    assert_eq!(events(&runtime), [crate::missions::WakeEvent::Resumed]);
}

/// 미션 종료 from the map (UI_BEHAVIOR, A mission's map): a Conductor that
/// holds the mission is asked once, by a letter, and keeps it open to ask
/// about its pull requests; once no Conductor holds it, the same press
/// cancels it at once, and a closed mission takes no more.
#[test]
fn the_operators_end_asks_a_present_conductor_once_and_cancels_a_mission_none_holds() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let wakes = |runtime: &Runtime| -> Vec<crate::missions::WakeEvent> {
        runtime.missions.book.missions[0]
            .wakes
            .iter()
            .map(|wake| wake.event)
            .collect()
    };

    assert!(runtime.mission_end_event("mission-1"));
    runtime.refresh_missions();
    let view = &runtime.snapshot.missions.items[0];
    assert!(view.status.open(), "the Conductor closes it, not the press");
    assert!(view.end_asked);
    assert_eq!(wakes(&runtime), [crate::missions::WakeEvent::EndAsked]);
    // A second press asks nothing more.
    assert!(!runtime.mission_end_event("mission-1"));
    assert_eq!(wakes(&runtime), [crate::missions::WakeEvent::EndAsked]);

    // The Conductor's pane closed before it ended the mission.
    runtime.delivery_observations.remove("sender");
    runtime.refresh_missions();
    assert!(runtime.mission_end_event("mission-1"));
    runtime.refresh_missions();
    let view = &runtime.snapshot.missions.items[0];
    assert_eq!(view.status, MissionStatus::Cancelled);
    assert!(!view.end_asked);
    assert!(view.closed_at_unix_ms.is_some());
    assert!(
        wakes(&runtime).is_empty(),
        "a cancelled mission wakes no one"
    );
    assert_eq!(
        runtime.missions.events["mission-1"]
            .events
            .last()
            .map(|event| event.kind),
        Some(crate::mission_events::EventKind::Cancelled)
    );
    // Closed, it takes no more presses.
    assert!(!runtime.mission_end_event("mission-1"));
}

/// A mission counts when its plan starts, with its size, and when it first
/// closes: a mission cancelled before it started, and a reopened one that
/// closes again, send nothing more.
#[test]
fn a_mission_counts_its_start_and_its_first_close() {
    use crate::product_analytics::{Event, MissionOutcome, MissionSize};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());
    let create = |runtime: &mut Runtime, title: &str| {
        command(runtime, "sender", json!({"op": "create", "title": title})).unwrap();
    };
    create(&mut runtime, "Never started");
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    assert_eq!(statistics.take(), []);

    create(&mut runtime, "Fix the bug");
    command(&mut runtime, "sender", plan()).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    assert_eq!(
        statistics.take(),
        [Event::MissionStarted {
            nodes: MissionSize::TwoToThree
        }]
    );
    without_gate(&mut runtime);
    let close = json!({"op": "close", "end": "done"});
    command(&mut runtime, "sender", close.clone()).unwrap();
    assert_eq!(
        statistics.take(),
        [Event::MissionEnded {
            outcome: MissionOutcome::Done
        }]
    );
    command(&mut runtime, "sender", json!({"op": "reopen"})).unwrap();
    command(&mut runtime, "sender", close).unwrap();
    assert_eq!(statistics.take(), []);
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

    // The first answer about the pull request is where its merge and
    // review wakes start; the CI result it already has is told.
    let open = github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false);
    assert_eq!(
        runtime.note_mission_pulls(node, &open),
        vec!["mission:mission-1:ci:fix:7:a1:passed:sender"]
    );
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
        vec!["mission:mission-1:ci:fix:7:b2:passed:sender"],
        "its CI's result on the new commit"
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
        vec!["mission:mission-1:changes_requested:fix:7:b2:1:sender"]
    );

    // A CI result on a done mission's pull request opens it again.
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

    // So does a run ending again on that commit with another result.
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done"}),
    )
    .unwrap();
    assert_eq!(
        runtime.note_mission_pulls(node, &checked(7, "c3", PullRequestChecks::Failed, &[])),
        vec!["mission:mission-1:ci:fix:7:c3:failed:sender"]
    );
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

/// The pull request `number` at `head` with its checks at `checks`, the
/// failed ones named `failed`.
fn checked(number: u32, head: &str, checks: PullRequestChecks, failed: &[&str]) -> GithubSnapshot {
    let mut pull = pull(number, head, PullRequestBadge::Open, None);
    pull.checks = checks;
    pull.work.failed_checks = failed
        .iter()
        .map(|name| crate::model::PrFailedCheck {
            name: Some((*name).to_owned()),
            url: None,
        })
        .collect();
    github(vec![pull], false)
}

/// A node's pull request tells its Conductor when its CI ends on a commit,
/// not when the commit arrives, and one result on one commit is one
/// letter: a rerun that fails again, or a read that cannot say, sends
/// nothing new.
#[test]
fn a_missions_pull_request_tells_its_conductor_each_ci_result_once_per_commit() {
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
    let mut read = |answer: GithubSnapshot| runtime.note_mission_pulls(node, &answer);

    assert!(read(checked(7, "a1", PullRequestChecks::Pending, &[])).is_empty());
    assert!(
        read(checked(7, "b2", PullRequestChecks::Pending, &[])).is_empty(),
        "a new commit is no letter"
    );
    let failed = checked(
        7,
        "b2",
        PullRequestChecks::Failed,
        &["verify / rust", "verify / journey"],
    );
    assert_eq!(
        read(failed.clone()),
        vec!["mission:mission-1:ci:fix:7:b2:failed:sender"]
    );
    assert!(read(failed.clone()).is_empty(), "the same read again");
    assert!(read(checked(7, "b2", PullRequestChecks::Pending, &[])).is_empty());
    assert!(read(checked(7, "b2", PullRequestChecks::Unknown, &[])).is_empty());
    assert!(
        read(failed).is_empty(),
        "a rerun failing again on the same commit"
    );
    assert_eq!(
        read(checked(7, "b2", PullRequestChecks::Passing, &[])),
        vec!["mission:mission-1:ci:fix:7:b2:passed:sender"]
    );
    // A commit whose CI ended between two reads is still told.
    assert_eq!(
        read(checked(7, "c3", PullRequestChecks::Passing, &[])),
        vec!["mission:mission-1:ci:fix:7:c3:passed:sender"]
    );
    // A review asking for changes still tells, with no CI result of its own.
    let mut asked = pull(
        7,
        "c3",
        PullRequestBadge::Open,
        Some(ReviewDecision::ChangesRequested),
    );
    asked.checks = PullRequestChecks::Passing;
    assert_eq!(
        read(github(vec![asked], false)),
        vec!["mission:mission-1:changes_requested:fix:7:c3:1:sender"]
    );
}

/// A CI letter's first line names the commit and the result, and a
/// failure names the checks that failed, counting those past the first
/// eight; a cancellation no run replaced is told apart from a failure.
#[test]
fn a_ci_letter_names_the_commit_the_result_and_the_failed_checks() {
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
    let names: Vec<String> = (0..10).map(|index| format!("job {index}")).collect();
    let mut failed: Vec<&str> = names.iter().map(String::as_str).collect();
    failed.push("job 0");
    runtime.note_mission_pulls(node, &checked(7, "a1", PullRequestChecks::Pending, &[]));
    runtime.note_mission_pulls(
        node,
        &checked(7, "1234567890ab", PullRequestChecks::Failed, &failed),
    );
    let mut cancelled = pull(7, "abcdef123456", PullRequestBadge::Open, None);
    cancelled.checks = PullRequestChecks::Failed;
    cancelled.work.checks_cancelled = true;
    cancelled.work.failed_checks = vec![crate::model::PrFailedCheck {
        name: Some("verify".into()),
        url: None,
    }];
    runtime.note_mission_pulls(node, &github(vec![cancelled], false));
    runtime.note_mission_pulls(node, &checked(7, "fedcba", PullRequestChecks::Passing, &[]));

    let mission = runtime.missions.book.get("mission-1").unwrap();
    let told: Vec<String> = mission
        .wakes
        .iter()
        .map(|wake| {
            crate::runtime::missions::wake_body(mission, wake)
                .lines()
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    let at = r#"(mission mission-1 "Fix the bug", node fix)."#;
    assert_eq!(
        told,
        [
            format!(
                "PR #7: CI failed on commit 1234567: job 0, job 1, job 2, job 3, job 4, job 5, job 6, job 7 and 2 more {at}"
            ),
            format!("PR #7: CI was cancelled on commit abcdef1 and no run replaced it {at}"),
            format!("PR #7: CI passed on commit fedcba {at}"),
        ]
    );
}

/// Mission `mission-1`, started, whose node `fix` holds pull request 7.
fn fixing_seven(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(runtime, "sender", plan()).unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
}

/// The pull request 7 at `head` whose checks failed while a workflow still
/// runs, or not, the failed ones named `failed`.
fn failed_running(head: &str, running: bool, failed: &[&str]) -> GithubSnapshot {
    let mut answer = checked(7, head, PullRequestChecks::Failed, failed);
    answer.projects[0].pull_requests[0].work.checks_running = running;
    answer
}

/// A failure ranks first among a head's checks, so one workflow failing
/// while another still runs is not the CI's end: the letter waits for it
/// and names every check that failed (with-hide/hide#2089 review 1).
#[test]
fn a_ci_letter_waits_for_every_workflow_on_the_head_to_end() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    let node = crate::node::TEST_NODE;
    runtime.note_mission_pulls(node, &checked(7, "b2", PullRequestChecks::Pending, &[]));
    assert!(
        runtime
            .note_mission_pulls(node, &failed_running("b2", true, &["static-design"]))
            .is_empty(),
        "verify still runs"
    );
    assert_eq!(
        runtime.note_mission_pulls(
            node,
            &failed_running("b2", false, &["static-design", "rust workspace"])
        ),
        vec!["mission:mission-1:ci:fix:7:b2:failed:sender"]
    );
    let mission = runtime.missions.book.get("mission-1").unwrap();
    let body = crate::runtime::missions::wake_body(mission, mission.wakes.last().unwrap());
    assert!(
        body.starts_with("PR #7: CI failed on commit b2: static-design, rust workspace "),
        "{body}"
    );
}

/// One rule decides a head's CI end for the finish line and the letter
/// alike: a failed check while another still runs leaves the ci line open,
/// spends no rerun and sends no letter; the end closes both together.
#[test]
fn a_failure_beside_a_running_check_keeps_the_ci_line_open_like_the_letter() {
    use crate::mission_criteria::{CriterionKind, CriterionState};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    let node = crate::node::TEST_NODE;
    let ci = |runtime: &Runtime| {
        let mission = runtime.missions.book.get("mission-1").unwrap();
        let fix = mission.nodes.iter().find(|node| node.key == "fix").unwrap();
        let line = fix
            .criteria
            .iter()
            .find(|line| line.kind == CriterionKind::Ci)
            .unwrap()
            .clone();
        (line.state, line.reruns)
    };
    runtime.note_mission_pulls(node, &checked(7, "b2", PullRequestChecks::Pending, &[]));
    assert!(
        runtime
            .note_mission_pulls(node, &failed_running("b2", true, &["static-design"]))
            .is_empty()
    );
    assert_eq!(ci(&runtime), (CriterionState::Open, 0), "verify still runs");
    assert_eq!(
        runtime
            .note_mission_pulls(node, &failed_running("b2", false, &["static-design"]))
            .len(),
        1
    );
    assert_eq!(ci(&runtime), (CriterionState::Fail, 0));
}

/// A CI letter the Conductor's full queue refuses is not told: the next
/// read after the queue has room tells it, once.
#[test]
fn a_ci_result_refused_by_a_full_queue_is_told_once_there_is_room() {
    use crate::missions::{PENDING_WAKE_LIMIT, PendingWake, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    let node = crate::node::TEST_NODE;
    runtime.note_mission_pulls(node, &checked(7, "b2", PullRequestChecks::Pending, &[]));
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    mission.wakes = (0..PENDING_WAKE_LIMIT)
        .map(|at| PendingWake {
            seq: at as u64 + 1,
            node: "docs".into(),
            pr: None,
            event: WakeEvent::NewHead,
            head: None,
            to: None,
            reason: None,
            why: None,
            action: None,
            warning: None,
            ci: None,
            key: Some(format!("filler:{at}")),
        })
        .collect();
    let failed = checked(7, "b2", PullRequestChecks::Failed, &["verify"]);
    assert!(
        runtime.note_mission_pulls(node, &failed).is_empty(),
        "the queue is full"
    );
    runtime
        .missions
        .book
        .get_mut("mission-1")
        .unwrap()
        .wakes
        .clear();
    assert_eq!(
        runtime.note_mission_pulls(node, &failed),
        vec!["mission:mission-1:ci:fix:7:b2:failed:sender"],
        "told once there is room"
    );
    assert!(
        runtime.note_mission_pulls(node, &failed).is_empty(),
        "told once"
    );
}

/// A pull request whose CI already ended when Hide first reads it, as when
/// it was named before the list held it, is told: only a merge or a review
/// starts from the first answer (with-hide/hide#2089 review 2).
#[test]
fn a_ci_result_hide_reads_first_is_told() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    let node = crate::node::TEST_NODE;
    assert_eq!(
        runtime.note_mission_pulls(
            node,
            &checked(7, "b2", PullRequestChecks::Failed, &["verify"])
        ),
        vec!["mission:mission-1:ci:fix:7:b2:failed:sender"]
    );
    assert!(
        runtime
            .note_mission_pulls(
                node,
                &checked(7, "b2", PullRequestChecks::Failed, &["verify"])
            )
            .is_empty()
    );
}

/// One result on one commit is told once for good: the mission remembers
/// it, so a failure, a pass and the same failure again send no third
/// letter, after the first letters left the record and with no letter to
/// match it against (with-hide/hide#2089 review 3). The same reads keep the
/// node's last push (with-hide/hide-private#552): results on one commit move
/// it not, a new commit does.
#[test]
fn a_result_told_on_a_commit_is_never_told_again() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    let node = crate::node::TEST_NODE;
    let pushed = |runtime: &Runtime| {
        let mission = runtime.missions.book.get("mission-1").unwrap();
        let fix = mission.nodes.iter().find(|node| node.key == "fix").unwrap();
        fix.pushed_at_unix_ms
    };
    let failed = checked(7, "b2", PullRequestChecks::Failed, &["verify"]);
    let passed = checked(7, "b2", PullRequestChecks::Passing, &[]);
    runtime.note_mission_pulls(node, &checked(7, "b2", PullRequestChecks::Pending, &[]));
    let first = pushed(&runtime);
    assert!(first.is_some(), "the first head read starts the push clock");
    assert_eq!(runtime.note_mission_pulls(node, &failed).len(), 1);
    assert_eq!(runtime.note_mission_pulls(node, &passed).len(), 1);
    let sent: Vec<(String, u64)> = runtime
        .missions
        .book
        .get("mission-1")
        .unwrap()
        .wakes
        .iter()
        .map(|wake| ("mission-1".to_owned(), wake.seq))
        .collect();
    runtime.mission_wakes_sent(&sent);
    let stored: crate::missions::MissionBook =
        serde_json::from_value(serde_json::to_value(&runtime.missions.book).unwrap()).unwrap();
    runtime.missions.book = stored;
    assert!(runtime.note_mission_pulls(node, &failed).is_empty());
    assert!(runtime.note_mission_pulls(node, &passed).is_empty());
    assert!(
        runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .wakes
            .is_empty()
    );
    assert_eq!(pushed(&runtime), first, "results on one commit are no push");
    let fix = |runtime: &mut Runtime| {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        mission
            .nodes
            .iter_mut()
            .find(|node| node.key == "fix")
            .unwrap()
            .pushed_at_unix_ms = Some(1);
    };
    fix(&mut runtime);
    // A new commit is a push, and its results are news again.
    assert_eq!(
        runtime.note_mission_pulls(
            node,
            &checked(7, "c3", PullRequestChecks::Failed, &["verify"])
        ),
        vec!["mission:mission-1:ci:fix:7:c3:failed:sender"]
    );
    assert!(
        pushed(&runtime) > Some(1),
        "a new commit moves the push clock"
    );
}

/// A mission whose project is on another device shows that device's pull
/// request, as GitHub answered for that device's project, never this
/// machine's read of the same path, and its map is stale only while the
/// device does not answer.
#[test]
fn a_mission_on_another_device_shows_that_devices_pull_request() {
    const DEVICE: &str = "mbp";
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
    runtime.missions.book.missions[0].device_id = DEVICE.into();
    runtime.github = github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false);
    let mut on_device = pull(7, "b2", PullRequestBadge::Merged, None);
    on_device.url = "https://github.com/acme/device/pull/7".into();
    runtime
        .device_github
        .insert(DEVICE.into(), github(vec![on_device], false));
    let fix = |runtime: &Runtime| {
        let view = runtime.mission_view_of("mission-1").unwrap();
        let node = view
            .nodes
            .into_iter()
            .find(|node| node.key == "fix")
            .unwrap();
        (node.pr_url, view.stale)
    };

    assert_eq!(
        fix(&runtime),
        (
            Some("https://github.com/acme/device/pull/7".to_owned()),
            true
        ),
        "the device's read, stale while the device does not answer"
    );
    runtime
        .snapshot
        .status
        .remote
        .push(crate::model::RemoteStatusSnapshot {
            target_id: DEVICE.into(),
            state: "connected".into(),
            message: None,
            session: None,
            files: crate::model::RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
    assert_eq!(
        fix(&runtime),
        (
            Some("https://github.com/acme/device/pull/7".to_owned()),
            false
        )
    );
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
        vec!["mission:mission-1:ci:fix:7:b2:passed:sender"],
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
    // CI that runs on: these wakes are the review's alone.
    let open = checked(7, "a1", PullRequestChecks::Pending, &[]);
    let mut asked = pull(
        7,
        "a1",
        PullRequestBadge::Open,
        Some(ReviewDecision::ChangesRequested),
    );
    asked.checks = PullRequestChecks::Pending;
    let asked = github(vec![asked], false);
    // A second ask on the same head is news again: GitHub's time of the
    // pull request moved with it.
    let mut asked_again = pull(
        7,
        "a1",
        PullRequestBadge::Open,
        Some(ReviewDecision::ChangesRequested),
    );
    asked_again.updated_at_unix_ms = Some(2);
    asked_again.checks = PullRequestChecks::Pending;
    let asked_again = github(vec![asked_again], false);
    runtime.note_mission_pulls(node, &open);
    let first = runtime.note_mission_pulls(node, &asked);
    runtime.note_mission_pulls(node, &open);
    let second = runtime.note_mission_pulls(node, &asked_again);
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
    // The operator's mission screen shows it.
    let id = runtime.missions.book.missions[0].id.clone();
    assert!(
        runtime
            .mission_shown(super::super::mission_events::MissionShownPayload { mission: Some(id) })
    );

    assert!(!runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), "another".into())]));
    assert!(runtime.drop_missions_of(&[(crate::node::TEST_NODE.into(), native)]));
    assert!(runtime.missions.book.missions.is_empty());
    assert_eq!(
        runtime.missions.shown, None,
        "the screen's detail of a deleted mission sends nothing"
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

/// Each edit in Settings › Mission sends only what it changes, so a second
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
    assert!(runtime.snapshot.ui_state.conductor_settings.clean_merged);
    assert!(runtime.dispatch_json(&event(json!({"clean_merged": false}))));
    assert!(runtime.dispatch_json(&event(json!({"map_flow": "across"}))));
    let settings = &runtime.snapshot.ui_state.conductor_settings;
    assert_eq!(settings.concurrency, 6);
    assert_eq!(settings.default_scope, crate::missions::MissionScope::Merge);
    assert_eq!(settings.default_strength, Strength::Thorough);
    assert!(!settings.clean_merged);
    assert_eq!(settings.map_flow, crate::missions::MapFlow::Across);
}

/// A mission whose node `fix` is done with pull request 7, which GitHub
/// answers with `badge` on a fresh list unless `github` says otherwise.
fn done_with_pull(runtime: &mut Runtime) {
    command(runtime, "sender", json!({"op": "create", "title": "Fix"})).unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
        ]}}),
    )
    .unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
    without_gate(runtime);
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7, "status": "done"}}),
    )
    .unwrap();
}

fn merged_fix() -> crate::runtime::mission_cleanup::Target {
    crate::runtime::mission_cleanup::Target {
        mission: "mission-1".into(),
        node: "fix".into(),
        machine: crate::node::TEST_NODE.into(),
        root: PROJECT.into(),
        branch: "fix".into(),
    }
}

/// Operator decision 2026-11-02: with the switch on, a done node's merged
/// branch is retired, and only on GitHub's fresh answer holding every pull
/// request the node names merged or closed; off, missions retire nothing.
#[test]
fn a_done_nodes_merged_worktree_is_retired_only_on_githubs_answer_and_with_the_switch_on() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    done_with_pull(&mut runtime);
    let targets = |runtime: &mut Runtime, github: GithubSnapshot| {
        runtime.github = github;
        runtime.mission_cleanup_targets()
    };
    let merged = || vec![pull(7, "a1", PullRequestBadge::Merged, None)];

    assert_eq!(
        targets(&mut runtime, github(merged(), false)),
        [merged_fix()]
    );
    // GitHub's answer is missing: never read, stale, or without the PR.
    assert!(targets(&mut runtime, GithubSnapshot::default()).is_empty());
    assert!(targets(&mut runtime, github(merged(), true)).is_empty());
    let mut unread = github(merged(), false);
    unread.projects[0].pull_requests_read = false;
    assert!(targets(&mut runtime, unread).is_empty());
    assert!(targets(&mut runtime, github(Vec::new(), false)).is_empty());
    // Not merged yet: open, or closed without its merge.
    let open = vec![pull(7, "a1", PullRequestBadge::Open, None)];
    assert!(targets(&mut runtime, github(open, false)).is_empty());
    let closed = vec![pull(7, "a1", PullRequestBadge::Closed, None)];
    assert!(targets(&mut runtime, github(closed, false)).is_empty());
    // Another open pull request on the same branch holds it.
    let follow_up = pull(8, "b1", PullRequestBadge::Open, None);
    assert!(
        targets(
            &mut runtime,
            github(vec![merged().remove(0), follow_up], false)
        )
        .is_empty()
    );

    runtime.snapshot.ui_state.conductor_settings.clean_merged = false;
    assert!(targets(&mut runtime, github(merged(), false)).is_empty());
    runtime.snapshot.ui_state.conductor_settings.clean_merged = true;
    // A node back at work is not retired.
    runtime.missions.book.get_mut("mission-1").unwrap().nodes[0].status = NodeStatus::Working;
    assert!(targets(&mut runtime, github(merged(), false)).is_empty());
}

/// A worktree kept for what the operator settles (changes) is not tried
/// again this run; one kept for an agent or a program, or refused, is
/// tried again on the next pass.
#[test]
fn a_kept_worktree_is_tried_again_only_when_what_keeps_it_may_leave() {
    use crate::runtime::worktree_retire::{Kept, Outcome, Retirement};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    done_with_pull(&mut runtime);
    runtime.github = github(vec![pull(7, "a1", PullRequestBadge::Merged, None)], false);
    for (outcome, again) in [
        (Outcome::Kept(Kept::AgentInside), true),
        (Outcome::Kept(Kept::ProgramRunning), true),
        (Outcome::Failed("agent.list is malformed".into()), true),
        (Outcome::Kept(Kept::Uncommitted), false),
    ] {
        let retirement = Retirement {
            path: Some(format!("{PROJECT}-fix")),
            outcome: outcome.clone(),
        };
        runtime.note_mission_cleanup(vec![(merged_fix(), retirement)], false);
        assert_eq!(
            runtime.mission_cleanup_targets() == [merged_fix()],
            again,
            "{outcome:?}"
        );
    }
}

/// A mission takes Settings › Mission's verification strength when it is
/// created; its plan changes the strength before or during the mission, each
/// change one event with its reason, and a plan that names none, or a later
/// settings edit, keeps it.
#[test]
fn a_mission_keeps_the_strength_it_was_created_with_until_its_plan_changes_it() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let settings = &mut runtime.snapshot.ui_state.conductor_settings;
    settings.default_strength = Strength::Fast;
    let (created, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    assert_eq!(created["mission"]["strength"], "fast");
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
/// stack: a dropped node is out of it, a pull request closed without
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
        ("screen", 8, "dropped"),
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
        "the dropped node's open pull request is not waited on"
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
            .mission_spawn_admission(
                &conductor,
                &worker(None, kind, &choice),
                &mut None,
                "plain",
                false,
            )
            .map(|admission| admission.is_some())
            .map_err(|refusal| crate::coordination::split_refusal(&refusal).0.to_owned())
    };
    // No candidates: any agent the machine has.
    assert_eq!(admit(&runtime, "codex", launch(None, None)), Ok(true));

    let candidates = vec![
        crate::missions::Candidate {
            agent: "claude".into(),
            profile: None,
            model: None,
            effort: None,
            purpose: "changes".into(),
        },
        crate::missions::Candidate {
            agent: "codex".into(),
            profile: None,
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
        crate::coordination::refusal("mission_candidate_refused", "Codex")
    );
}

/// The fixture's own machine lists Codex's profile `team` (`p-team`), and
/// the Conductor's pane (`sender`, a Codex session) records that it runs
/// with it.
fn conductor_runs_team(runtime: &mut Runtime) {
    let node = runtime.node.to_string();
    let mut kit = runtime.kit_state(&node);
    kit.profiles = Some(crate::model::ProfilesSnapshot {
        state: crate::model::ProfilesState::Listed,
        version: 1,
        profiles: vec![crate::model::ProfileSnapshot {
            id: "p-team".into(),
            kind: "codex".into(),
            name: "team".into(),
            account: Some("/accounts/team".into()),
            program: None,
            env_keys: Vec::new(),
            version: 1,
            install: None,
            folder: None,
            hook: None,
            skill: None,
            login: None,
            wrapper_folder: false,
            startable: true,
            account_key: Some(hide_agent_adapter::AccountKey::Folder(
                "/accounts/team".into(),
            )),
            account_variables: Vec::new(),
        }],
        defaults: Default::default(),
    });
    runtime.kit_states.insert(node, kit);
    runtime.snapshot.ui_state.pane_launches.insert(
        "sender".into(),
        crate::model::PaneLaunch {
            pending: false,
            start: 0,
            kind: "codex".into(),
            name: "sender".into(),
            choice: Default::default(),
            session: Some("sender-session".into()),
            profile: Some(crate::launch::LaunchProfile {
                id: hide_agent_adapter::ProfileId("p-team".into()),
                name: "team".into(),
                account: hide_agent_adapter::AccountKey::Folder("/accounts/team".into()),
            }),
        },
    );
}

/// A candidate holds the profile a worker actually starts with (PRD
/// agent-profiles-phase2 D-08, B12, B13, B15): a Conductor running
/// `Codex · team` that spawns a Codex worker without `--profile` hands it
/// its own profile, which a candidate of Codex's built-in default does not
/// admit, and the refusal names what would have started; the same spawn
/// naming the built-in default is the candidate. A candidate of
/// `Codex · team` admits a worker that inherits it or names it, and not one
/// on the built-in default. A candidate of a profile the machine no longer
/// lists admits no worker at all, since no start can take that profile.
#[test]
fn a_candidate_admits_the_profile_a_worker_starts_with() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let conductor = fixture.conductor.clone();
    conductor_runs_team(&mut runtime);
    let candidate = |profile: Option<&str>| crate::missions::Candidate {
        agent: "codex".into(),
        profile: profile.map(|id| hide_agent_adapter::ProfileId(id.into())),
        model: None,
        effort: None,
        purpose: String::new(),
    };
    runtime.snapshot.ui_state.conductor_settings.candidates = vec![candidate(None)];
    running(&mut runtime);
    let defaults = hide_agent_adapter::LaunchChoice::default();
    let admit = |runtime: &Runtime, profile: Option<&str>| {
        runtime
            .mission_spawn_admission(
                &conductor,
                &crate::runtime::missions::SpawnWorker {
                    profile,
                    ..worker(None, "codex", &defaults)
                },
                &mut None,
                "plain",
                false,
            )
            .map(|admission| admission.is_some())
    };
    let refused = |item: &str| {
        Err(crate::coordination::refusal(
            "mission_candidate_refused",
            item,
        ))
    };

    assert_eq!(admit(&runtime, None), refused("Codex · team"));
    assert_eq!(admit(&runtime, Some("default")), Ok(true));
    // The spawn command itself is refused, before the ledger reserves it.
    assert_eq!(
        prepare_spawn(&mut runtime, &conductor, None).unwrap_err(),
        crate::coordination::refusal("mission_candidate_refused", "Codex · team")
    );

    runtime.missions.book.missions[0].candidates = vec![candidate(Some("p-team"))];
    assert_eq!(admit(&runtime, None), Ok(true));
    assert_eq!(admit(&runtime, Some("team")), Ok(true));
    assert_eq!(admit(&runtime, Some("default")), refused("Codex"));

    runtime.missions.book.missions[0].candidates = vec![candidate(Some("p-gone"))];
    assert_eq!(admit(&runtime, None), refused("Codex · team"));
    assert_eq!(admit(&runtime, Some("default")), refused("Codex"));
    assert!(
        admit(&runtime, Some("gone"))
            .unwrap_err()
            .starts_with(crate::launch::PROFILE_GONE)
    );
}

/// `hide mission settings` names each candidate as the agent item it
/// starts and the `--profile` value that starts it (PRD
/// agent-profiles-phase2 B11, B14, B15): a row kept with its agent alone is
/// the built-in default, a row of a listed profile is named by that
/// profile's name now, and a row whose profile this machine no longer lists
/// says so and names none.
#[test]
fn mission_settings_name_each_candidates_item() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    conductor_runs_team(&mut runtime);
    runtime.snapshot.ui_state.conductor_settings.candidates = vec![
        serde_json::from_value(json!({"agent": "codex", "purpose": "changes"})).unwrap(),
        serde_json::from_value(json!({"agent": "codex", "profile": "p-team", "model": "gpt-x"}))
            .unwrap(),
        serde_json::from_value(json!({"agent": "codex", "profile": "p-gone"})).unwrap(),
    ];
    let (answer, _) = command(&mut runtime, "sender", json!({"op": "settings"})).unwrap();
    let rows = answer["settings"]["candidates"].as_array().unwrap().clone();
    let named = |row: &Value| {
        (
            row["item"].clone(),
            row["profile"].clone(),
            row.get("profile_gone").cloned(),
        )
    };
    assert_eq!(named(&rows[0]), (json!("Codex"), json!("default"), None));
    assert_eq!(
        named(&rows[1]),
        (json!("Codex · team"), json!("team"), None)
    );
    assert_eq!(
        named(&rows[2]),
        (Value::Null, Value::Null, Some(json!(true)))
    );
    assert_eq!(
        (rows[0]["purpose"].clone(), rows[1]["model"].clone()),
        (json!("changes"), json!("gpt-x"))
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

/// The phone's missions are read from the record the core publishes, so the
/// field names of `MissionView` and of the phone's whitelist cannot drift
/// apart without this failing.
#[test]
fn the_phone_reads_the_mission_the_core_published_with_its_decision_and_steps() {
    use crate::agent_state::phone::{PhoneMissions, project};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    let ask = json!({"op": "decide", "line": "Which base?", "why": "Two PRs touch it",
        "options": ["main", "#1503"], "recommend": 2, "node": "fix"});
    command(&mut runtime, "sender", ask).unwrap();
    runtime.refresh_missions();
    let rest = json!({
        "navigator": {"agents": []},
        "missions": serde_json::to_value(&runtime.snapshot.missions).unwrap(),
    });
    let phone = PhoneMissions::project(
        &rest,
        crate::node::TEST_NODE,
        &project(&rest, crate::node::TEST_NODE),
    );
    assert!(!phone.failed);
    let mission = phone.find("mission-1").expect("the mission is listed");
    assert_eq!(mission.title, "Fix the bug");
    assert_eq!(mission.decision.as_deref(), Some("Which base?"));
    let question = mission.question.as_ref().expect("its question");
    assert_eq!(
        (question.options.clone(), question.recommend),
        (vec!["main".to_owned(), "#1503".to_owned()], Some(2))
    );
    assert_eq!(
        mission
            .nodes
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>(),
        runtime.snapshot.missions.items[0]
            .nodes
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>(),
    );
    assert!(mission.conductor.takes_answers || !mission.conductor.present);
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
        node(json!({"status": "dropped", "line": "CI failed"})),
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

/// An answer held on a Conductor's session Hide cannot read says why on the
/// banner for as long as the read refuses, and nothing once a read lands.
#[test]
fn an_answer_held_on_an_unreadable_conductor_session_says_why() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Which base?", "options": ["main", "#1503"]}),
    )
    .unwrap();
    assert!(runtime.mission_answer_event("mission-1", 1, Some(2), None, false));
    let observe = |runtime: &mut Runtime, refused: bool| {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"idle",
             "state_change_seq":2,"lineage_session":"sender-session",
             "agent_session":{"kind":"id","value":"sender-native"}},
        ]}))
        .unwrap();
        let labels =
            crate::labels::overlay::LabelOverlay::default().with_refusals(refused.then(|| {
                (
                    "sender".to_owned(),
                    hide_session::label_reference_token("codex", "id", "sender-native"),
                    "session_capacity:line_bytes:262144".to_owned(),
                )
            }));
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, Some(&labels));
        runtime.refresh_missions();
        runtime.snapshot.missions.items[0].answer_failed
    };
    runtime.mission_answer_held("mission-1");
    assert_eq!(
        observe(&mut runtime, true),
        Some(AnswerFailure::SessionUnreadable)
    );
    assert_eq!(observe(&mut runtime, false), None, "a read that lands");
    assert!(
        runtime.missions.book.missions[0].answer_failed.is_none(),
        "the reason is shown, never kept"
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
fn an_answer_the_engine_may_have_typed_is_typed_again_only_when_the_operator_sends_it_again() {
    use crate::runtime::mission_events::SendFailure;
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
    // holds it as unconfirmed, so a stop before the engine's reply never types it
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
    // The engine's reply to the prompt timed out: the answer may be in the conversation.
    runtime.mission_answer_sent(
        "mission-1",
        1,
        Err(SendFailure::MaybeTyped("herdr_call_failed")),
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
    runtime.mission_answer_sent("mission-1", 1, Err(SendFailure::MaybeTyped("herdr_format")));
    // Typed, the answer takes the decision away; the Conductor need not clear it.
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (
            mission.decision.as_deref(),
            mission.question.is_none(),
            mission.answer.is_none()
        ),
        (None, true, true)
    );
    assert!(runtime.mission_answers_due().is_empty());
    // The Conductor clearing it as well adds no answer of its own.
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
        Err(SendFailure::NotTyped("herdr_parameters")),
    );
    let mission = &runtime.missions.book.missions[0];
    assert_eq!(
        (mission.answer.is_none(), mission.answer_failed),
        (true, Some(AnswerFailure::SendFailed))
    );
}

/// A book written while a typed answer stayed until the Conductor cleared
/// its decision is settled when it is read: the banner does not wait on
/// that Conductor, and the answer is never typed a second time (B29).
#[test]
fn a_book_holding_a_typed_answer_settles_its_decision_when_read() {
    let fixture = conducting();
    // Its own file: the fixture's store saves its book in the background.
    let path = fixture._root.path().join("older-missions.json");
    let mut guard = fixture.runtime.lock().unwrap();
    running(&mut guard);
    command(
        &mut guard,
        "sender",
        json!({"op": "decide", "line": "Which base?"}),
    )
    .unwrap();
    assert!(guard.mission_answer_event("mission-1", 1, None, Some("main".into()), false));
    let encoded =
        crate::missions::encode(&std::sync::Arc::new(guard.missions.book.clone())).unwrap();
    let mut written: Value = serde_json::from_slice(&encoded).unwrap();
    assert!(
        written["missions"][0]["answer"].get("delivered").is_none(),
        "the book no longer writes it"
    );
    written["missions"][0]["answer"]["delivered"] = json!(true);
    std::fs::write(&path, serde_json::to_vec(&written).unwrap()).unwrap();
    let store = crate::missions::MissionStore::new(path);
    let restored = store.restore();
    assert!(matches!(restored, crate::missions::Restored::Read(_)));
    guard.install_mission_store(store, restored);
    let mission = &guard.missions.book.missions[0];
    assert_eq!(
        (
            mission.decision.as_deref(),
            mission.question.is_none(),
            mission.answer.is_none()
        ),
        (None, true, true)
    );
    assert!(guard.mission_answers_due().is_empty());
    drop(guard.missions.store.take());
}

#[test]
fn the_open_missions_published_events_share_the_cores_list() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.mission_shown(super::super::mission_events::MissionShownPayload {
        mission: Some("mission-1".into()),
    });
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
fn only_the_mission_the_screen_shows_sends_its_events() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
    assert!(runtime.snapshot.mission_events.events.is_empty());
    assert!(
        runtime.mission_shown(super::super::mission_events::MissionShownPayload {
            mission: Some("mission-1".into()),
        })
    );
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
    // The detail closed: no mission's events are sent; the same answer again changes nothing.
    let shown = |mission: Option<&str>| super::super::mission_events::MissionShownPayload {
        mission: mission.map(str::to_owned),
    };
    // The typed event carries it from the page; a mission the book does not hold reads as none.
    let event = |mission: Value| explorer_event("mission_shown", json!({ "mission": mission }));
    assert!(runtime.dispatch_json(&event(json!("mission-404"))));
    assert_eq!(runtime.missions.shown, None);
    assert!(runtime.dispatch_json(&event(json!("mission-1"))));
    assert_eq!(runtime.missions.shown.as_deref(), Some("mission-1"));
    assert!(runtime.dispatch_json(&event(Value::Null)));
    // A payload with a field the event does not know is refused and changes nothing.
    runtime.dispatch_json(&explorer_event(
        "mission_shown",
        json!({ "mission": "mission-1", "pane": "p1" }),
    ));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|e| e.kind.as_str()),
        Some("event.invalid_payload")
    );
    assert_eq!(runtime.missions.shown, None);
    assert!(!runtime.mission_shown(shown(None)));
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
    assert!(runtime.snapshot.mission_events.events.is_empty());
    assert!(runtime.mission_shown(shown(Some("mission-1"))));
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

/// Operator decision 2026-11-09: a mission's card on top of the front pane's
/// 보기 slot sends its events while the slot is drawn; the mission
/// screen's detail, when the page names one, wins; the card goes with its
/// mission.
#[test]
fn the_mission_card_in_the_slot_sends_its_events_and_goes_with_its_mission() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.open_detail(DetailItem::Mission {
        mission: "mission-1".into(),
    });
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.mission.as_deref(),
        Some("mission-1")
    );
    assert_eq!(runtime.snapshot.mission_events.events.len(), 1);
    // The slot folded behind the documents sends nothing; shown again, it does.
    runtime.snapshot.ui_state.detail.shown = false;
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
    runtime.snapshot.ui_state.detail.shown = true;
    // The mission screen's detail wins over the card while the page names it.
    runtime.missions.shown = Some("mission-404".into());
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.mission.as_deref(),
        Some("mission-404")
    );
    runtime.missions.shown = None;
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.mission.as_deref(),
        Some("mission-1")
    );
    // Another card on top sends no mission's events; ← brings the map back.
    runtime.open_detail(DetailItem::Cost {
        workspace_id: "w".into(),
    });
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
    runtime.detail_back();
    runtime.refresh_mission_events();
    assert_eq!(
        runtime.snapshot.mission_events.mission.as_deref(),
        Some("mission-1")
    );
    // The mission deleted with its Conductor's session takes its card along.
    let session = runtime.missions.book.missions[0]
        .conductor
        .session
        .clone()
        .unwrap();
    assert!(runtime.drop_missions_of(&[(session.device_id, session.session_id)]));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default()
    );
    runtime.refresh_mission_events();
    assert_eq!(runtime.snapshot.mission_events.mission, None);
}

/// Operator decision 2026-11-09: a mission's row opens its map in the slot
/// of its Conductor's pane, which comes forward with it, and leaves the focus
/// where it was while no Conductor holds it.
#[test]
fn a_mission_opens_in_its_conductors_slot() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    runtime.refresh_missions();
    let open = |runtime: &mut Runtime| {
        assert!(runtime.dispatch_json(&explorer_event(
            "detail_open",
            json!({"item": {"kind": "mission", "mission": "mission-1"}, "at": "owner"}),
        )));
    };
    let map = DetailItem::Mission {
        mission: "mission-1".into(),
    };
    assert!(runtime.dispatch_json(&operator_focus_event("recipient")));
    open(&mut runtime);
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some("sender"));
    assert_eq!(runtime.snapshot.ui_state.detail.items.last(), Some(&map));

    // Its Conductor's pane is gone: the pane in front stays.
    runtime.missions.book.missions[0].conductor.actor.pane_id = "gone".into();
    runtime.refresh_missions();
    assert!(!runtime.snapshot.missions.items[0].conductor.present);
    let front = runtime.snapshot.focused.pane_id.clone();
    open(&mut runtime);
    assert_eq!(runtime.snapshot.focused.pane_id, front);
    assert_eq!(runtime.snapshot.status.last_error, None);
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
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
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
    // A summary is one short line the map shows whole; a second line or a
    // paragraph is refused with its way on, and a blank one is none.
    for summary in ["첫 줄\n둘째 줄", &"가".repeat(161)] {
        assert_eq!(
            command(
                &mut runtime,
                "reviewer",
                record("review", json!({"summary": summary}))
            )
            .unwrap_err(),
            "mission_summary_invalid"
        );
    }
    // The node has a pull request, so the review names the commit it read.
    assert_eq!(
        command(
            &mut runtime,
            "reviewer",
            record("review", json!({"summary": "문제 없음"}))
        )
        .unwrap_err(),
        "mission_commit_required"
    );
    let (shown, _) = command(
        &mut runtime,
        "reviewer",
        record(
            "review",
            json!({"summary": "  문제 없음  ", "commit": HEAD}),
        ),
    )
    .unwrap();
    let review = shown["mission"]["nodes"][0]["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["key"] == "review")
        .unwrap();
    assert_eq!(review["checkers"][0]["summary"], "문제 없음");
    assert_eq!(review["checkers"][0]["evidence"], "it ran");
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

/// A node's planned steps ride its view from the plan on, and the brief its
/// worker was spawned on is kept by the node when the worker is bound, so it
/// stays after the ledger lets the spawn and its letter go; only its size
/// rides the map, and the text is read when the panel's folded box opens.
#[test]
fn a_nodes_steps_show_from_the_plan_and_its_brief_is_kept_when_the_worker_binds() {
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
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
             "criteria": ["it works"], "steps": ["Find the cause", "Fix it", "Open the PR"]},
            {"key": "docs", "title": "Docs", "kind": "research", "criteria": ["it is said"]},
        ]}}),
    )
    .unwrap();
    assert_eq!(
        node_view(&runtime, "fix").steps,
        ["Find the cause", "Fix it", "Open the PR"]
    );
    assert!(node_view(&runtime, "docs").steps.is_empty());
    // A step too long is refused with its reason and its next action.
    let refused = command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
             "criteria": ["it works"], "steps": ["x".repeat(crate::missions::STEP_CHARS + 1)]},
        ]}}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused),
        ("mission_steps_invalid", Some("fix"))
    );
    assert!(crate::missions::next_action("mission_steps_invalid").contains("80 characters"));

    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(&mut runtime);
    assert_eq!(node_view(&runtime, "fix").brief_bytes, None);
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let mut record: crate::coordination::SpawnRecord = serde_json::from_value(json!({
        "id": "spawn-1", "parent": "agent-1", "intent": "mission-1:fix", "name": "worker",
        "kind": "codex", "repo": "/checkouts/fixture", "branch": "fix", "requested_path": null,
        "args": [], "path": null, "pane": null, "child": "agent-2",
    }))
    .unwrap();
    record.brief = Some("Node fix: find the cause, then fix it.".into());
    record.briefed = true;
    ledger.spawns.push(record);
    runtime.publish_delivery(Arc::new(ledger), true);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "fix").brief_bytes, Some(38));
    // The text is not in the map's view; the folded box asks for it.
    assert!(runtime.snapshot.mission_brief.text.is_none());
    assert!(
        runtime.mission_brief_read(super::super::mission_brief::BriefReadPayload {
            mission: "mission-1".into(),
            node: "fix".into(),
        })
    );
    assert_eq!(
        runtime.snapshot.mission_brief.text.as_deref(),
        Some("Node fix: find the cause, then fix it.")
    );
    // The ledger forgetting the spawn and its letter leaves the node's copy.
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger.spawns.clear();
    runtime.publish_delivery(Arc::new(ledger), true);
    assert_eq!(node_view(&runtime, "fix").brief_bytes, Some(38));
    // A node with no brief reads as none, never an empty box.
    assert!(
        runtime.mission_brief_read(super::super::mission_brief::BriefReadPayload {
            mission: "mission-1".into(),
            node: "docs".into(),
        })
    );
    assert_eq!(runtime.snapshot.mission_brief.text, None);
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

/// The commit the view publishes as the node's proof of being in main.
fn in_main(view: &crate::runtime::missions::NodeView) -> Option<String> {
    match &view.landing {
        Some(crate::runtime::mission_landing::NodeLanding::Landed {
            proof: crate::runtime::mission_landing::LandingProof::InMain { commit },
        }) => Some(commit.clone()),
        _ => None,
    }
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
    assert_eq!(node_view(&runtime, "parser").status, NodeStatus::Working);
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
    let round = runtime
        .missions
        .book
        .get("mission-1")
        .unwrap()
        .node("lib")
        .unwrap()
        .outside_round;
    let lib = || Ask {
        mission: "mission-1".into(),
        node: "lib".into(),
        watch: Watch::Outside {
            external: External::Branch("feat/lib".into()),
            ahead: None,
            reopens: 0,
            round,
        },
    };
    runtime.ingest_outside(
        &place,
        vec![(
            lib(),
            Found::Unread("The branch feat/lib is not in this repository".into()),
        )],
    );
    let read = node_view(&runtime, "lib");
    assert_eq!((read.status, read.unread), (NodeStatus::Working, true));
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

/// PRD mission-node-simple D-15: an outside pull request closed without a
/// merge keeps its node in progress with the Conductor asked to look, and
/// the mission cannot close as done until the node is dropped.
#[test]
fn an_outside_pull_request_closed_without_a_merge_asks_the_conductor() {
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
    let parser = node_view(&runtime, "parser");
    assert_eq!(parser.status, NodeStatus::Working);
    let attention = parser.attention.expect("the closed pull request is raised");
    assert_eq!(
        (attention.reason, attention.pr),
        (crate::missions::AttentionReason::PrClosed, Some(1232))
    );
    assert_eq!(node_view(&runtime, "fix").blocked, None);
    let last = runtime.missions.book.missions[0]
        .wakes
        .last()
        .cloned()
        .unwrap();
    assert_eq!(last.event, WakeEvent::Attention);
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
    command(
        runtime,
        "reviewer",
        record_of("fix", "review", json!({"commit": HEAD})),
    )
    .unwrap();
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

/// with-hide/hide#2080 review: a ci verdict holds only for the head it was
/// read on. A push moves the pull request to a new head whose runs Hide
/// could not read: the line is open, not the old head's pass, and keeps the
/// node from done.
#[test]
fn a_ci_pass_does_not_hold_after_a_push_whose_checks_are_not_known() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    let read = |runtime: &mut Runtime, head: &str, checks| {
        let mut pull = pull(7, head, PullRequestBadge::Open, None);
        pull.checks = checks;
        let read = github(vec![pull], false);
        runtime.github = read.clone();
        runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
    };
    read(&mut runtime, HEAD, PullRequestChecks::Passing);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Pass);
    let pushed = "f".repeat(40);
    read(&mut runtime, &pushed, PullRequestChecks::Unknown);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Open);
    let mission = runtime.missions.book.missions[0].clone();
    let mut done = mission
        .nodes
        .iter()
        .find(|node| node.key == "fix")
        .unwrap()
        .clone();
    done.status = NodeStatus::Done;
    assert!(
        runtime
            .node_unmet(&mission, &done)
            .contains(&"ci".to_owned())
    );
}

/// Review finding 5, D-15: a pull request's CI gets one rerun per head
/// however many runs GitHub reports, and the line still follows the run that
/// finished last on that head; the node stays in progress throughout.
#[test]
fn a_ci_line_follows_the_last_run_on_its_head_after_its_rerun() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
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
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    // The rerun runs and passes: the line passes with its rerun counted.
    read(&mut runtime, PullRequestChecks::Pending);
    read(&mut runtime, PullRequestChecks::Passing);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Pass);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    // Another run on the same head fails: there is no second rerun.
    read(&mut runtime, PullRequestChecks::Pending);
    read(&mut runtime, PullRequestChecks::Failed);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Fail);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    // Someone runs it again on that head and it passes: the line passes.
    read(&mut runtime, PullRequestChecks::Passing);
    assert_eq!(line_state(&runtime, "ci"), CriterionState::Pass);
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
    assert_eq!(in_main(&node_view(&runtime, "fix")).as_deref(), Some(HEAD));
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
}

/// Re-review finding 1: the same rule holds the close a waiting mission
/// makes on its branches in main.
#[test]
fn a_mission_waiting_on_its_branches_keeps_waiting_while_a_node_is_back_at_work() {
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
    assert_eq!(in_main(&node_view(&runtime, "a")), Some(a1.clone()));
    let before_move = landing_ask(&runtime, "a");

    // The Conductor moves `a` to another branch: what was read of the old
    // one proves nothing, then or when its late answer comes.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "a", "patch": {"branch": "feat/c"}}),
    )
    .unwrap();
    assert_eq!(in_main(&node_view(&runtime, "a")), None);
    answer(&mut runtime, before_move, Found::InMain(Some(a1)));
    land(&mut runtime, "b", Found::InMain(Some(b1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    assert_eq!(in_main(&node_view(&runtime, "a")), None);

    // A branch that moved on past main is no longer in it.
    land(&mut runtime, "b", Found::Open);
    assert_eq!(in_main(&node_view(&runtime, "b")), None);
    land(&mut runtime, "b", Found::InMain(Some(b1.clone())));
    land(&mut runtime, "a", Found::InMain(Some(c1.clone())));
    assert_eq!(mission_status(&runtime), MissionStatus::Done);

    // Reopened, the same branches are read again before it closes, and an
    // answer asked in the earlier wait proves nothing in the next.
    let earlier = landing_ask(&runtime, "a");
    command(&mut runtime, "sender", json!({"op": "reopen"})).unwrap();
    assert_eq!(
        (
            in_main(&node_view(&runtime, "a")),
            in_main(&node_view(&runtime, "b"))
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
    assert_eq!(in_main(&node_view(&runtime, "a")), None);
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
    assert_eq!(in_main(&node_view(&runtime, "a")), Some(a1.clone()));

    // The next read cannot see `a` while `b` reaches main.
    land(
        &mut runtime,
        "a",
        Found::Unread("The branch feat/a is not in this repository or its last fetch".into()),
    );
    land(&mut runtime, "b", Found::InMain(Some(b1)));
    assert_eq!(mission_status(&runtime), MissionStatus::MergeWait);
    let a = node_view(&runtime, "a");
    assert_eq!((in_main(&a), a.unread), (None, true));
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
    assert_eq!((in_main(&a), a.unread), (Some(a1), false));
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
/// whose own commit Hide read is done once merged there, a waiting
/// mission's own branch keeps it waiting until it is merged too, and the
/// commit found is the one in main.
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

    // Hide reads the library branch's own commit before main takes it.
    runtime.read_outside_through(device, &root, &host);
    let lib = node_view(&runtime, "lib");
    assert_eq!((lib.status, lib.unread), (NodeStatus::Working, false));
    git_in(&repo, &["merge", "-q", "--no-ff", "-m", "lib", "feat/lib"]);
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
        (&fix["landing"]["proof"]["commit"], &fix["commit"]),
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
    assert_eq!(
        published["nodes"][1]["landing"]["proof"]["commit"],
        json!(landed)
    );
}

/// GitHub's answer for the project at `root`: one pull request it merged
/// into `trunk` from the branch at `head`.
fn merged_into_trunk(root: &str, head: &str) -> GithubSnapshot {
    let mut merged = pull(9, head, PullRequestBadge::Merged, None);
    merged.base_branch = "trunk".into();
    GithubSnapshot {
        projects: vec![GithubProjectSnapshot {
            pull_requests_read: true,
            root_path: root.into(),
            pull_requests: vec![merged],
            ..Default::default()
        }],
    }
}

/// mission-23: an outside branch is in main once work of its own reached
/// main, never because it was cut from main. In a repository with no
/// remote, a branch with no commit of its own waits, and keeps waiting
/// while main moves on; once Hide read a commit of its own on it, that
/// commit reaching main by a fast-forward makes it done, across a restart
/// between the two. A branch GitHub squash-merged is done though main never
/// holds its commits; one merged before the mission named it, with no pull
/// request to show for it, waits and says how to finish it, as does one
/// whose commits Hide saw were replaced by a force push.
#[test]
fn an_outside_branch_is_in_main_only_once_its_own_work_reached_main() {
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
        &["branch", "feat/cut"],
        &["branch", "feat/old"],
        &["branch", "feat/squashed"],
        &["branch", "feat/forced"],
    ] {
        git_in(&repo, args);
    }
    for branch in ["feat/old", "feat/squashed", "feat/forced"] {
        git_in(&repo, &["checkout", "-q", branch]);
        git_in(&repo, &["commit", "-q", "--allow-empty", "-m", branch]);
    }
    git_in(&repo, &["checkout", "-q", "trunk"]);
    // Merged for real before the mission names it, with no pull request.
    git_in(&repo, &["merge", "-q", "--no-ff", "-m", "old", "feat/old"]);
    let root = hide_platform::path::to_wire(&repo).unwrap();

    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let outside = |key: &str, branch: &str| json!({"key": key, "title": key, "kind": "external", "external": {"branch": branch}});
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            outside("cut", "feat/cut"),
            outside("old", "feat/old"),
            outside("squashed", "feat/squashed"),
            outside("forced", "feat/forced"),
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    runtime.missions.book.missions[0].project = root.clone();
    runtime
        .worktree_catalog
        .projects
        .push(crate::model::ProjectWorktreesSnapshot {
            root_path: root.clone(),
            base_branch: Some("trunk".into()),
            ..Default::default()
        });
    let squashed = git_in(&repo, &["rev-parse", "feat/squashed"]);
    runtime.github = merged_into_trunk(&root, &squashed);
    let host = Host(hide_host::serve::Env::standalone(Some(repo.clone())));
    let device = crate::node::TEST_NODE;
    let status = |runtime: &Runtime, key: &str| {
        let read = node_view(runtime, key);
        assert!(!read.unread, "{key} was read");
        read.status
    };
    let line = |runtime: &Runtime, key: &str| node_view(runtime, key).line.unwrap_or_default();

    runtime.read_outside_through(device, &root, &host);
    assert_eq!(status(&runtime, "cut"), NodeStatus::Working);
    // The Conductor reads why it waits, and what finishes it, in the line.
    assert!(line(&runtime, "cut").contains("has not seen a commit of its own"));
    assert_eq!(status(&runtime, "old"), NodeStatus::Working);
    assert!(line(&runtime, "old").contains("name its pull request instead, or drop the node"));
    assert_eq!(status(&runtime, "squashed"), NodeStatus::Done);
    assert_eq!(status(&runtime, "forced"), NodeStatus::Working);
    assert!(line(&runtime, "forced").contains("commits main does not have"));

    // Main moving on leaves the cut branch behind it, still with nothing
    // of its own.
    git_in(&repo, &["commit", "-q", "--allow-empty", "-m", "elsewhere"]);
    runtime.read_outside_through(device, &root, &host);
    assert_eq!(status(&runtime, "cut"), NodeStatus::Working);

    git_in(&repo, &["checkout", "-q", "feat/cut"]);
    git_in(&repo, &["rebase", "-q", "trunk"]);
    git_in(&repo, &["commit", "-q", "--allow-empty", "-m", "the work"]);
    // The forced branch's commit is thrown away for main's head.
    git_in(&repo, &["branch", "-f", "feat/forced", "trunk"]);
    runtime.read_outside_through(device, &root, &host);
    assert_eq!(status(&runtime, "cut"), NodeStatus::Working);
    assert!(line(&runtime, "cut").contains("commits main does not have"));
    assert_eq!(status(&runtime, "forced"), NodeStatus::Working);
    assert!(line(&runtime, "forced").contains("were replaced"));

    // A restart reads the book back and forgets every read.
    let stored: crate::missions::MissionBook =
        serde_json::from_value(serde_json::to_value(&runtime.missions.book).unwrap()).unwrap();
    runtime.missions.book = stored;
    runtime.missions.outside = Default::default();
    git_in(&repo, &["checkout", "-q", "trunk"]);
    git_in(&repo, &["merge", "-q", "--ff-only", "feat/cut"]);
    runtime.read_outside_through(device, &root, &host);
    assert_eq!(status(&runtime, "cut"), NodeStatus::Done);
    assert!(line(&runtime, "cut").contains("In main"));
}

/// Review of #1961: what Hide saw of an outside branch belongs to one
/// round of its node. An answer asked before the node named another branch
/// and then the first again is dropped, so it cannot bring back what the
/// replan dropped; a node put back to work after it was done starts with
/// nothing seen, and so does a reopened mission's.
#[test]
fn an_outside_branch_answer_from_an_earlier_round_proves_nothing() {
    use crate::missions::External;
    use crate::runtime::mission_outside::{Ask, Found, Watch};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let plan = |branch: &str| {
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "lib", "title": "lib", "kind": "external", "external": {"branch": branch}},
        ]}})
    };
    command(&mut runtime, "sender", plan("feat/a")).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    let place = (crate::node::TEST_NODE.to_owned(), PROJECT.to_owned());
    let ask = |runtime: &Runtime| Ask {
        mission: "mission-1".into(),
        node: "lib".into(),
        watch: Watch::Outside {
            external: External::Branch("feat/a".into()),
            ahead: None,
            reopens: runtime.missions.outside.reopens_of("mission-1"),
            round: runtime.missions.book.missions[0].nodes[0].outside_round,
        },
    };
    let seen = |runtime: &Runtime| {
        runtime.missions.book.missions[0].nodes[0]
            .branch_ahead
            .clone()
    };
    let late = ask(&runtime);

    command(&mut runtime, "sender", plan("feat/b")).unwrap();
    command(&mut runtime, "sender", plan("feat/a")).unwrap();
    // A node taken out of the plan and put back gets a round never given.
    let before = runtime.missions.book.missions[0].nodes[0].outside_round;
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "other", "title": "other", "kind": "external", "external": {"branch": "feat/a"}},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", plan("feat/a")).unwrap();
    assert!(runtime.missions.book.missions[0].nodes[0].outside_round > before);
    runtime.ingest_outside(&place, vec![(late, Found::Ahead("a".repeat(40)))]);
    assert_eq!(seen(&runtime), None);
    assert_eq!(node_view(&runtime, "lib").line, None);

    // An answer of this round is taken.
    let current = ask(&runtime);
    runtime.ingest_outside(&place, vec![(current, Found::Ahead("b".repeat(40)))]);
    assert_eq!(seen(&runtime), Some("b".repeat(40)));
    let current = ask(&runtime);
    runtime.ingest_outside(&place, vec![(current, Found::InMain(Some("b".repeat(40))))]);
    assert_eq!(node_view(&runtime, "lib").status, NodeStatus::Done);

    // Back to work after it was done: a new round, nothing seen, and an
    // answer asked before it is dropped.
    let late = ask(&runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "lib", "patch": {"status": "working"}}),
    )
    .unwrap();
    assert_eq!(seen(&runtime), None);
    runtime.ingest_outside(&place, vec![(late, Found::Ahead("c".repeat(40)))]);
    assert_eq!(seen(&runtime), None);

    let current = ask(&runtime);

    runtime.ingest_outside(&place, vec![(current, Found::Ahead("d".repeat(40)))]);
    assert_eq!(seen(&runtime), Some("d".repeat(40)));
    let late = ask(&runtime);
    let current = ask(&runtime);
    runtime.ingest_outside(&place, vec![(current, Found::InMain(Some("d".repeat(40))))]);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "merge_wait"}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "reopen", "mission": "mission-1"}),
    )
    .unwrap();
    assert_eq!(seen(&runtime), None);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "lib", "patch": {"status": "working"}}),
    )
    .unwrap();
    runtime.ingest_outside(&place, vec![(late, Found::Ahead("e".repeat(40)))]);
    assert_eq!(seen(&runtime), None);
}

/// This machine's node, which on its first resolve of `trunk` moves main
/// on and cuts `topic` again at the new tip before it answers, as another
/// session can between two git calls of one read.
struct MovesMain {
    repo: std::path::PathBuf,
    host: Host,
    moved: std::sync::atomic::AtomicBool,
}

impl crate::node_access::NodeLink for MovesMain {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        let resolves_main = matches!(
            &call,
            hide_node_link::protocol::Call::Git {
                command: hide_node_link::git::GitCommand::RefCommit { name },
                ..
            } if name == "trunk"
        );
        let answer = crate::node_access::NodeLink::call(&self.host, call, timeout);
        if resolves_main && !self.moved.swap(true, std::sync::atomic::Ordering::SeqCst) {
            git_in(
                &self.repo,
                &["commit", "-q", "--allow-empty", "-m", "main moves on"],
            );
            git_in(&self.repo, &["branch", "-f", "topic", "trunk"]);
        }
        answer
    }
}

/// Review of #1961, round 2: main moving on, and the outside branch being
/// cut again at its new tip, between the git calls of one read never reads
/// the branch ahead, so a later read cannot find that head in main and call
/// the node done; the branch still has no commit of its own.
#[test]
fn a_branch_cut_from_a_main_that_moved_during_the_read_has_no_work_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(folder.path()).unwrap();
    for args in [
        &["init", "-q", "-b", "trunk"][..],
        &["config", "user.name", "Fixture"],
        &["config", "user.email", "fixture@example.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "seed"],
        &["branch", "topic"],
    ] {
        git_in(&repo, args);
    }
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
            {"key": "topic", "title": "topic", "kind": "external", "external": {"branch": "topic"}},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    runtime.missions.book.missions[0].project = root.clone();
    runtime
        .worktree_catalog
        .projects
        .push(crate::model::ProjectWorktreesSnapshot {
            root_path: root.clone(),
            base_branch: Some("trunk".into()),
            ..Default::default()
        });
    let node = MovesMain {
        repo: repo.clone(),
        host: Host(hide_host::serve::Env::standalone(Some(repo.clone()))),
        moved: Default::default(),
    };
    let device = crate::node::TEST_NODE;

    for _ in 0..2 {
        runtime.read_outside_through(device, &root, &node);
        let read = node_view(&runtime, "topic");
        assert_eq!((read.status, read.unread), (NodeStatus::Working, false));
        assert!(
            read.line
                .unwrap_or_default()
                .contains("has not seen a commit of its own")
        );
    }
    assert!(node.moved.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        runtime.missions.book.missions[0].nodes[0].branch_ahead,
        None
    );
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
        assert_eq!(in_main(&node_view(&runtime, "a")), None);
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
            if in_main(&node_view(&runtime, "a")).is_some() {
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
    assert_eq!(
        shown["mission"]["nodes"][0]["landing"]["proof"]["commit"],
        json!(landed)
    );
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
        // A branch whose own work is in main through a pull request.
        &["checkout", "-q", "-b", "landed"],
        &["commit", "-q", "--allow-empty", "-m", "landed"],
        &["checkout", "-q", "trunk"],
        &["merge", "-q", "--no-ff", "-m", "landed", "landed"],
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
        // GitHub merged the landed branch's pull request: its own work is
        // in main however Hide first reads it.
        runtime.github = merged_into_trunk(&root, &git_in(&repo, &["rev-parse", "landed"]));
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
                          "external": {"branch": "landed"}}),
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
                    assert_eq!((node.status, node.unread), (NodeStatus::Working, true));
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
        // A branch whose own work is in main through a pull request.
        &["checkout", "-q", "-b", "landed"],
        &["commit", "-q", "--allow-empty", "-m", "landed"],
        &["checkout", "-q", "trunk"],
        &["merge", "-q", "--no-ff", "-m", "landed", "landed"],
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
        // GitHub merged the landed branch's pull request: its own work is
        // in main however Hide first reads it.
        runtime.github = merged_into_trunk(&root, &git_in(&repo, &["rev-parse", "landed"]));
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
            json!({"op": "plan", "plan": {"nodes": [outside("first", "absent"), outside("last", "landed")]}}),
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
                assert_eq!((first.status, first.unread), (NodeStatus::Working, true));
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

    // B3: answered from the map, the row leaves 내 차례 at once, while the
    // answer is still on its way to the Conductor.
    let key = runtime.missions.book.missions[0]
        .question
        .as_ref()
        .unwrap()
        .key;
    assert!(
        runtime
            .answer_mission("mission-1", key, None, Some("main".into()), false)
            .unwrap()
    );
    let answered = conductor_shown(&mut runtime);
    assert_eq!(
        (answered.ask_subject, answered.group),
        (None, resting.group.clone())
    );
    // The Conductor asks something new: that question stands.
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Rebase on main first?"}),
    )
    .unwrap();
    assert_eq!(conductor_shown(&mut runtime).group, "needs_you");

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

/// A decision an engine sync laid before any refresh read it, cleared before
/// the next refresh, still leaves the row (needs-you-cleanup B3).
#[test]
fn a_decision_laid_by_a_sync_and_cleared_before_the_next_refresh_leaves_the_row() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    conductor_shown(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Skip the check line?"}),
    )
    .unwrap();
    // The engine sends the row again before the next pull reads the books.
    let mut synced = std::mem::take(&mut runtime.snapshot.navigator.agents);
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut synced);
    assert!(synced[0].ask_subject.is_some());
    runtime.snapshot.navigator.agents = synced;
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    assert_eq!(conductor_shown(&mut runtime).ask_subject, None);
}

/// One landing case: what the record holds, and what the core answers.
struct LandingCase {
    name: &'static str,
    status: MissionStatus,
    /// Sets the nodes `pr` (a pull request), `direct` (a branch), `first`
    /// (a pull request the merge order puts first) and `research` as the
    /// case needs, after each was made done.
    set: fn(&mut crate::missions::Mission),
    /// The branch read answered unread for `direct` (`확인 못 함`).
    direct_unread: bool,
    /// (node, landing as the view publishes it), for every node.
    landing: [(&'static str, Value); 4],
    order: &'static [&'static str],
    /// The stacked pull requests still waiting (`pr` is based on `first`'s).
    stack: u32,
    /// Whether a mission waiting on its merges closed itself as done.
    closes: bool,
    /// A mission from before criteria, which no gate holds (D-19).
    exempt: bool,
    /// What the Conductor's `close --as merge_wait` of the case, run while
    /// the mission runs, leaves.
    explicit: Option<MissionStatus>,
}

const SHA: &str = "4f1c9a27be03aa00bb11cc22dd33ee44ff556677";

/// What GitHub last answered of `node`'s current pull request.
fn see(node: &mut crate::missions::MissionNode, merged: bool, closed: bool) {
    let number = node.current_pr().expect("the node holds a pull request");
    node.pr_seen.retain(|seen| seen.number != number);
    node.pr_seen.push(crate::missions::PrSeen {
        number,
        head: Some("head".into()),
        merged,
        closed,
        changes_requested: false,
        ci_told: Vec::new(),
    });
}

fn node_mut<'a>(
    mission: &'a mut crate::missions::Mission,
    key: &str,
) -> &'a mut crate::missions::MissionNode {
    mission
        .nodes
        .iter_mut()
        .find(|node| node.key == key)
        .unwrap()
}

/// `direct`'s branch read in main at `SHA`.
fn direct_in_main(mission: &mut crate::missions::Mission) {
    node_mut(mission, "direct").in_main = Some(crate::missions::Landed {
        branch: "feat/direct".into(),
        commit: SHA.into(),
    });
}

/// Both pull requests merged and the branch in main: nothing waits.
fn all_landed(mission: &mut crate::missions::Mission) {
    see(node_mut(mission, "pr"), true, false);
    see(node_mut(mission, "first"), true, false);
    direct_in_main(mission);
}

/// A fresh runtime holding `case`'s mission at `status`: four done nodes,
/// every line met, `pr` stacked on `first`, then the case's own changes.
fn landing_case(case: &LandingCase, status: MissionStatus) -> Fixture {
    let fixture = conducting();
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Landing"}),
        )
        .unwrap();
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "research", "title": "Research", "kind": "research", "criteria": ["it works"]},
                {"key": "pr", "title": "PR", "kind": "change", "delivery": "pr", "criteria": ["it works"], "depends_on": ["research"]},
                {"key": "direct", "title": "Direct", "kind": "change", "delivery": "direct", "criteria": ["it works"], "depends_on": ["research"]},
                {"key": "first", "title": "First", "kind": "change", "delivery": "pr", "criteria": ["it works"]},
            ], "merge_order": ["first"]}}),
        )
        .unwrap();
        command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        // Every line met, so only the landing, a node's state and whether
        // the mission is gated decide the close.
        for node in &mut mission.nodes {
            node.criteria = if node.delivery == Some(crate::missions::Delivery::Pr) {
                vec![
                    serde_json::from_value(json!({"key": "ci", "kind": "ci", "state": "pass"}))
                        .unwrap(),
                ]
            } else {
                Vec::new()
            };
            node.status = NodeStatus::Done;
            node.finished_at_unix_ms = Some(1);
        }
        node_mut(mission, "pr").prs = vec![11];
        node_mut(mission, "pr").base_pr = Some(10);
        node_mut(mission, "first").prs = vec![10];
        node_mut(mission, "direct").branch = Some("feat/direct".into());
        mission.exempt = case.exempt;
        mission.status = status;
        (case.set)(mission);
    }
    fixture
}

/// mission-23 c2: one matrix of the cases a mission's landing is decided
/// in, asserting both the close Hide makes itself and what the view
/// publishes for the map: each node's landing, and the order of what is
/// left (`mission_landing`).
#[test]
fn a_missions_landing_decides_its_close_and_its_published_actions_from_one_rule() {
    let landed_merged = json!({"state": "landed", "proof": {"by": "merged"}});
    let landed_main = json!({"state": "landed", "proof": {"by": "in_main", "commit": SHA}});
    let left_merge = json!({"state": "left", "action": "merge"});
    let left_join = json!({"state": "left", "action": "join"});
    let unread_merge = json!({"state": "unread", "action": "merge"});
    let unread_join = json!({"state": "unread", "action": "join"});
    let none = Value::Null;
    let cases = vec![
        LandingCase {
            name: "merge_wait, everything in main",
            status: MissionStatus::MergeWait,
            set: all_landed,
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: true,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, pull requests open and the branch not in main, by merge order",
            status: MissionStatus::MergeWait,
            set: |mission| {
                see(node_mut(mission, "pr"), false, false);
                see(node_mut(mission, "first"), false, false);
            },
            direct_unread: false,
            landing: [
                ("pr", left_merge.clone()),
                ("direct", left_join.clone()),
                ("first", left_merge.clone()),
                ("research", none.clone()),
            ],
            order: &["first", "pr", "direct"],
            stack: 1,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a pull request closed unmerged waits on nothing",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                see(node_mut(mission, "pr"), false, true);
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: true,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a pull request GitHub never answered for waits unread",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                node_mut(mission, "pr").pr_seen.clear();
            },
            direct_unread: false,
            landing: [
                ("pr", unread_merge.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 1,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a branch read could not answer",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                node_mut(mission, "direct").in_main = None;
            },
            direct_unread: true,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", unread_join.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a stale proof read for the branch the node named before",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                node_mut(mission, "direct").in_main = Some(crate::missions::Landed {
                    branch: "feat/old".into(),
                    commit: SHA.into(),
                });
            },
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", left_join.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &["direct"],
            stack: 0,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a commit the Conductor recorded is no proof",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let direct = node_mut(mission, "direct");
                direct.in_main = None;
                direct.commit = Some(SHA.into());
            },
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", left_join.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &["direct"],
            stack: 0,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a direct change that names no branch waits unread",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let direct = node_mut(mission, "direct");
                direct.in_main = None;
                direct.branch = None;
            },
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", unread_join.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a node back at work keeps it waiting and has no landing",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                node_mut(mission, "direct").status = NodeStatus::Working;
            },
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", none.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "merge_wait, a dropped node's open pull request waits on nothing",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let pr = node_mut(mission, "pr");
                pr.status = NodeStatus::Dropped;
                see(pr, false, false);
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: true,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "running, what is left shows while the mission runs and nothing closes",
            status: MissionStatus::Running,
            set: |mission| {
                all_landed(mission);
                see(node_mut(mission, "pr"), false, false);
                node_mut(mission, "direct").in_main = None;
            },
            direct_unread: false,
            landing: [
                ("pr", left_merge.clone()),
                ("direct", left_join.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &["pr", "direct"],
            stack: 1,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "cancelled, nothing is left and what landed stays proven",
            status: MissionStatus::Cancelled,
            set: |mission| {
                see(node_mut(mission, "pr"), false, false);
                see(node_mut(mission, "first"), true, false);
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", none.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 1,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "done, an open pull request is still left and a branch no longer read",
            status: MissionStatus::Done,
            set: |mission| see(node_mut(mission, "pr"), false, false),
            direct_unread: false,
            landing: [
                ("pr", left_merge.clone()),
                ("direct", none.clone()),
                ("first", unread_merge.clone()),
                ("research", none.clone()),
            ],
            order: &["pr"],
            stack: 1,
            closes: false,
            exempt: false,
            explicit: None,
        },
        LandingCase {
            name: "exempt merge_wait, an open pull request at work keeps it waiting (#1939 review 1)",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let pr = node_mut(mission, "pr");
                pr.status = NodeStatus::Working;
                see(pr, false, false);
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 1,
            closes: false,
            exempt: true,
            explicit: Some(MissionStatus::MergeWait),
        },
        LandingCase {
            name: "exempt merge_wait, an unread pull request at work keeps it waiting",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let pr = node_mut(mission, "pr");
                pr.status = NodeStatus::Working;
                pr.pr_seen.clear();
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 1,
            closes: false,
            exempt: true,
            explicit: Some(MissionStatus::MergeWait),
        },
        LandingCase {
            name: "exempt merge_wait, a direct change at work whose branch is not in main keeps it waiting",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                let direct = node_mut(mission, "direct");
                direct.status = NodeStatus::Working;
                direct.in_main = None;
            },
            direct_unread: false,
            landing: [
                ("pr", landed_merged.clone()),
                ("direct", none.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: false,
            exempt: true,
            explicit: Some(MissionStatus::MergeWait),
        },
        LandingCase {
            name: "exempt merge_wait, a pull request at work already merged waits on nothing",
            status: MissionStatus::MergeWait,
            set: |mission| {
                all_landed(mission);
                node_mut(mission, "pr").status = NodeStatus::Working;
            },
            direct_unread: false,
            landing: [
                ("pr", none.clone()),
                ("direct", landed_main.clone()),
                ("first", landed_merged.clone()),
                ("research", none.clone()),
            ],
            order: &[],
            stack: 0,
            closes: true,
            exempt: true,
            explicit: Some(MissionStatus::Done),
        },
    ];

    for case in cases {
        let fixture = landing_case(&case, case.status);
        let mut runtime = fixture.runtime.lock().unwrap();
        if case.direct_unread {
            land(
                &mut runtime,
                "direct",
                crate::runtime::mission_outside::Found::Unread("not in this repository".into()),
            );
        }
        let closed = runtime.close_if_landed("mission-1", 2);
        assert_eq!(closed, case.closes, "{}: close", case.name);
        if case.closes {
            assert_eq!(
                mission_status(&runtime),
                MissionStatus::Done,
                "{}",
                case.name
            );
        }
        let view = serde_json::to_value(runtime.mission_view_of("mission-1").unwrap()).unwrap();
        for (key, expected) in &case.landing {
            let node = view["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["key"] == *key)
                .unwrap();
            assert_eq!(&node["landing"], expected, "{}: {key}", case.name);
        }
        assert_eq!(
            view["landing_order"],
            json!(case.order),
            "{}: order",
            case.name
        );
        // No list was ever read here, so a pull request's state is only
        // what GitHub last answered (`pr_seen`): a merge it remembers counts
        // as merged though no list holds it (#1939 review 2).
        assert_eq!(
            view["stack_waiting"],
            json!(case.stack),
            "{}: stack",
            case.name
        );
        drop(runtime);
        if let Some(expected) = case.explicit {
            let fixture = landing_case(&case, MissionStatus::Running);
            let mut runtime = fixture.runtime.lock().unwrap();
            command(
                &mut runtime,
                "sender",
                json!({"op": "close", "end": "merge_wait"}),
            )
            .unwrap();
            assert_eq!(
                mission_status(&runtime),
                expected,
                "{}: explicit close",
                case.name
            );
        }
    }
}

/// PRD mission-node-simple B6, B18: a pull request merged with every line
/// met finishes its node with Hide's mark, and the last node finished
/// closes the mission with the Conductor told.
#[test]
fn hide_finishes_a_merged_node_and_closes_the_mission_after_its_last_node() {
    use crate::missions::{HideAction, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);

    let merged = github(vec![pull(7, HEAD, PullRequestBadge::Merged, None)], false);
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    runtime.drive_mission_health(now);
    let fix = node_view(&runtime, "fix");
    assert_eq!(fix.status, NodeStatus::Done);
    assert_eq!(fix.hide.map(|mark| mark.action), Some(HideAction::Done));
    assert_eq!(
        mission_status(&runtime),
        MissionStatus::Running,
        "docs is open"
    );

    // GitHub's answer goes stale before the last node finishes: the close
    // waits, raising nothing and refusing nothing, until a read comes back.
    let stale = github(vec![pull(7, HEAD, PullRequestBadge::Merged, None)], true);
    runtime.github = stale.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &stale);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "docs", "patch": {"status": "dropped"}}),
    )
    .unwrap();
    runtime.drive_mission_health(now);
    runtime.drive_mission_health(now);
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert!(
        mission.nodes.iter().all(|node| node.attention.is_none()),
        "{:?}",
        mission
            .nodes
            .iter()
            .map(|node| &node.attention)
            .collect::<Vec<_>>()
    );
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    runtime.drive_mission_health(now);
    assert_ne!(mission_status(&runtime), MissionStatus::Running);
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert!(
        mission
            .wakes
            .iter()
            .any(|wake| wake.event == WakeEvent::Closed),
        "{:?}",
        mission.wakes
    );
}

/// PRD mission-node-simple B6, B13, B14: a merge with a line unmet is
/// raised to the Conductor once, with its letter waiting, and is not raised
/// again after the Conductor settles it.
#[test]
fn a_merge_with_a_line_unmet_is_raised_to_the_conductor_once() {
    use crate::missions::{AttentionReason, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "pr": 7}}),
    )
    .unwrap();
    let merged = github(vec![pull(7, HEAD, PullRequestBadge::Merged, None)], false);
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    let fix = node_view(&runtime, "fix");
    assert_eq!(fix.status, NodeStatus::Working);
    let attention = fix.attention.expect("the merge is raised");
    assert_eq!(
        (attention.reason, attention.pr, attention.to),
        (
            AttentionReason::MergedUnproven,
            Some(7),
            crate::missions::AttentionTo::Conductor
        )
    );
    let raised = |runtime: &Runtime| {
        runtime.missions.events["mission-1"]
            .events
            .iter()
            .filter(|event| event.kind == crate::mission_events::EventKind::Raised)
            .count()
    };
    assert_eq!(raised(&runtime), 1);
    let letters = &runtime.missions.book.get("mission-1").unwrap().wakes;
    assert!(
        letters
            .iter()
            .any(|wake| wake.event == WakeEvent::Attention)
    );
    // The Conductor drops the node: the mark goes and nothing is raised again.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "dropped"}}),
    )
    .unwrap();
    runtime.drive_mission_health(now);
    assert!(node_view(&runtime, "fix").attention.is_none());
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
    )
    .unwrap();
    runtime.drive_mission_health(now);
    assert!(node_view(&runtime, "fix").attention.is_none());
    assert_eq!(raised(&runtime), 1);
}

/// PRD mission-node-simple B5: a node that waits on the operator with no
/// Needs You row to lay it on, neither the Conductor nor its worker, is
/// told by notice once; one with a row standing is not.
#[test]
fn a_raise_with_no_row_to_lay_it_on_is_told_to_the_operator_once() {
    use crate::delivery::worker::HumanNoticeKind;
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    let now = crate::runtime::unix_milliseconds();
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    assert!(
        !crate::runtime::mission_health::raise(
            mission,
            "fix",
            AttentionReason::PrClosed,
            Some(7),
            now
        )
        .is_empty()
    );
    runtime.save_missions();
    // Past ten minutes the raise reaches the operator, laid on the
    // Conductor's row while it is there.
    runtime.raise_to_operator(now + 10 * 60 * 1000);
    assert_eq!(
        node_view(&runtime, "fix")
            .attention
            .map(|attention| attention.to),
        Some(crate::missions::AttentionTo::Operator)
    );
    assert!(runtime.claim_mission_notices().is_empty());

    runtime.delivery_observations.remove("sender");
    runtime.snapshot.navigator.agents.clear();
    let notices = runtime.claim_mission_notices();
    assert_eq!(
        notices
            .iter()
            .map(|notice| (notice.kind, notice.id.as_str(), notice.about.as_str()))
            .collect::<Vec<_>>(),
        [(HumanNoticeKind::MissionAttention, "mission-1", "Fix it")]
    );
    assert!(runtime.claim_mission_notices().is_empty(), "told once");
    let stored = runtime.missions.book.get("mission-1").unwrap();
    assert!(
        stored
            .node("fix")
            .unwrap()
            .attention
            .as_ref()
            .unwrap()
            .notified
    );
}

/// PRD mission-node-simple B14, letter 19419: a mission whose node waits on
/// the operator with no row to carry it is counted in its device's Needs You
/// number, the one the Overview, the device rail and the sidebar read, and
/// named on that scope; while the Conductor's row carries it, it is not.
#[test]
fn a_mission_no_row_carries_is_counted_in_its_devices_needs_you() {
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    let now = crate::runtime::unix_milliseconds();
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    assert!(
        !crate::runtime::mission_health::raise(
            mission,
            "fix",
            AttentionReason::PrClosed,
            Some(7),
            now
        )
        .is_empty()
    );
    runtime.save_missions();
    runtime.raise_to_operator(now + 10 * 60 * 1000);
    let device = runtime
        .snapshot
        .missions
        .items
        .first()
        .map(|mission| mission.device_id.clone())
        .unwrap_or_else(|| crate::node::TEST_NODE.to_owned());
    let counted = |runtime: &mut Runtime| {
        runtime.snapshot_delta_payload(0);
        let scope = &runtime.snapshot.navigator.agent_scope;
        let device = runtime
            .snapshot
            .navigator
            .devices
            .iter()
            .find(|row| row.id == device)
            .map(|row| {
                (
                    row.agent_scope.mission_asks.clone(),
                    row.agent_scope.overview_needs_you,
                    row.agent_scope.groups.needs_you,
                )
            });
        (
            scope.mission_asks.clone(),
            scope.overview_needs_you,
            scope.groups.needs_you,
            device,
        )
    };
    let (asks, overview, needs_you, on_device) = counted(&mut runtime);
    assert!(asks.is_empty(), "the Conductor's row carries it: {asks:?}");
    runtime.delivery_observations.remove("sender");
    runtime.snapshot.navigator.agents.clear();
    let (asks, after_overview, after_needs_you, after_device) = counted(&mut runtime);
    assert_eq!(asks, ["mission-1"]);
    // One item either way: the Conductor's row while it carried the raise,
    // the mission once no row does.
    assert_eq!((overview, needs_you), (1, 1));
    assert_eq!((after_overview, after_needs_you), (1, 1));
    let (before, after) = (on_device.unwrap(), after_device.unwrap());
    assert_eq!((before.1, before.2), (1, 1));
    assert_eq!(after, (vec!["mission-1".to_owned()], 1, 1));
    // Counted once however often it is published.
    assert_eq!(counted(&mut runtime).1, after_overview);

    // Review 5 P2-3: cancelled, the mission keeps its record and asks
    // nothing: the count and the list are both empty, and stay so after a
    // restart reads the same book.
    assert!(runtime.mission_end_event("mission-1"));
    assert_eq!(mission_status(&runtime), MissionStatus::Cancelled);
    let kept = &runtime.missions.book.get("mission-1").unwrap().nodes;
    assert!(kept.iter().any(|node| node.attention.is_some()));
    let (asks, overview, needs_you, on_device) = counted(&mut runtime);
    assert!(asks.is_empty(), "{asks:?}");
    assert_eq!((overview, needs_you), (0, 0));
    assert_eq!(on_device.unwrap(), (Vec::<String>::new(), 0, 0));
    assert!(
        runtime.snapshot.missions.items[0]
            .nodes
            .iter()
            .filter_map(|node| node.attention.as_ref())
            .all(|attention| !attention.item)
    );
    let book = runtime.missions.book.clone();
    drop(runtime);
    let restarted = conducting();
    let mut runtime = restarted.runtime.lock().unwrap();
    runtime.missions.book = book;
    let (asks, overview, _, _) = counted(&mut runtime);
    assert!(asks.is_empty(), "{asks:?}");
    assert_eq!(overview, 0);
}

/// PRD mission-node-simple B14, review 4 P1-2: a raise that reached the
/// operator is on a row only while a row shown now carries it. A resting
/// Conductor's row does; a working Conductor's row, a worker whose
/// registration stands with no row shown, and no row at all do not, and
/// then the mission is its device's own Needs You item and the operator is
/// told by notice.
#[test]
fn a_raise_is_on_a_row_only_while_a_row_shown_carries_it() {
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    let now = crate::runtime::unix_milliseconds();
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    assert!(
        !crate::runtime::mission_health::raise(
            mission,
            "fix",
            AttentionReason::PrClosed,
            Some(7),
            now
        )
        .is_empty()
    );
    runtime.save_missions();
    runtime.raise_to_operator(now + 10 * 60 * 1000);
    let shown = |runtime: &mut Runtime| {
        runtime.snapshot_delta_payload(0);
        let on_row = node_view(runtime, "fix")
            .attention
            .map(|attention| attention.on_row);
        let device = runtime
            .snapshot
            .navigator
            .devices
            .iter()
            .find(|device| device.id == crate::node::TEST_NODE)
            .unwrap();
        (on_row, device.agent_scope.mission_asks.clone())
    };

    // A resting Conductor's row carries it.
    assert_eq!(shown(&mut runtime), (Some(true), vec![]));
    assert_eq!(
        runtime.snapshot.navigator.agents[0].ask_subject,
        Some(crate::model::AskSubject::MissionAttention {
            node: "Fix it".into(),
            reason: AttentionReason::PrClosed,
            unmet: 0,
        })
    );
    assert!(runtime.claim_mission_notices().is_empty());

    // A working Conductor's row shows its work and carries nothing.
    let mut working = vec![conductor_row()];
    working[0].activity = "working".into();
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut working);
    runtime.snapshot.navigator.agents = working;
    assert_eq!(
        shown(&mut runtime),
        (Some(false), vec!["mission-1".to_owned()])
    );
    assert_eq!(runtime.claim_mission_notices().len(), 1, "told once");

    // The Conductor gone, a worker whose registration stands with no row
    // shown carries nothing either.
    let mut actor = runtime.delivery_observations["sender"].actor.clone();
    actor.pane_id = "recipient".into();
    runtime.delivery_observations.remove("sender");
    runtime.snapshot.navigator.agents.clear();
    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger.agents.push(crate::coordination::AgentRecord {
        id: "agent-2".into(),
        name: Some("recipient".into()),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: "recipient-session".into(),
        instance: "recipient".into(),
        pane: "recipient".into(),
        parent: None,
        origin: None,
        project: None,
        actor,
        ended: false,
        terminal_id: None,
        forked: false,
    });
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    mission
        .nodes
        .iter_mut()
        .find(|node| node.key == "fix")
        .unwrap()
        .agent_id = Some("agent-2".into());
    runtime.save_missions();
    assert_eq!(
        shown(&mut runtime),
        (Some(false), vec!["mission-1".to_owned()])
    );

    // With no registration either, it stays the mission's own item.
    runtime.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::Ledger::default()));
    assert_eq!(
        shown(&mut runtime),
        (Some(false), vec!["mission-1".to_owned()])
    );
}

/// The operator's report of 2026-11-09: a Conductor's row carrying a raise
/// they had read lit up again, and was announced again, each time a worker's
/// letter woke the Conductor and it rested. The raise they read stays read
/// through the Conductor's own work and rest; another raise is news, one
/// notice (docs/status-model.md, A question the core lays stays read).
#[test]
fn a_raise_the_operator_read_stays_read_while_its_conductor_works_and_rests() {
    use crate::agent_state::{phone, push};
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![conductor_row()];
    running(&mut runtime);
    let mut transitions = push::Transitions::default();
    // What the next published snapshot shows of the Conductor's row, and the
    // notices it sends.
    let mut shown = |runtime: &mut Runtime| {
        runtime.snapshot_delta_payload(0);
        let row = runtime.snapshot.navigator.agents[0].clone();
        let rest = json!({"navigator": {"agents": [serde_json::to_value(&row).unwrap()]}});
        let (notices, _) =
            transitions.observe(&phone::project(&rest, crate::labels::store::LOCAL_TARGET));
        (row, notices)
    };
    // The engine sends the Conductor's row at `seq`, working or finished.
    let engine = |runtime: &mut Runtime, status: &str, seq: u64| {
        let mut row = conductor_row();
        row.activity = if status == "working" {
            "working"
        } else {
            "stopped"
        }
        .into();
        row.completed = status == "done";
        row.state_change_seq = Some(seq);
        let mut rows = vec![row];
        runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut rows);
        runtime.snapshot.navigator.agents = rows;
        runtime.refresh_pane_read_state();
    };
    let raise = |runtime: &mut Runtime, node: &str, now: u64| {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        let reason = AttentionReason::WorkerEnded;
        assert!(
            !crate::runtime::mission_health::raise(mission, node, reason, None, now).is_empty()
        );
        runtime.save_missions();
        runtime.raise_to_operator(now + 10 * 60 * 1000);
    };
    shown(&mut runtime);
    let now = crate::runtime::unix_milliseconds();
    raise(&mut runtime, "fix", now);
    let (asking, notices) = shown(&mut runtime);
    assert_eq!((asking.group.as_str(), asking.unread), ("needs_you", true));
    assert_eq!(notices.len(), 1);

    runtime.set_operator_focus(Some("sender".into()));
    runtime.set_operator_focus(None);
    let (read, notices) = shown(&mut runtime);
    assert!(!read.unread);
    assert!(notices.is_empty());

    // A worker's letter wakes the Conductor; it works, then rests.
    engine(&mut runtime, "working", 2);
    let (working, notices) = shown(&mut runtime);
    assert_eq!(working.ask_subject, None, "a working row carries no raise");
    assert!(notices.is_empty());
    engine(&mut runtime, "done", 3);
    let (rested, notices) = shown(&mut runtime);
    assert_eq!(rested.ask_subject, read.ask_subject);
    assert_eq!((rested.demand.as_str(), rested.unread), ("question", false));
    assert!(notices.is_empty(), "the raise read is not announced again");
    assert!(
        rested.revisit.is_some_and(|revisit| revisit.read_ask),
        "it is still the operator's to answer"
    );

    // Another raise beside it is news, once.
    raise(&mut runtime, "cause", now + 1);
    let (more, notices) = shown(&mut runtime);
    assert_eq!((more.group.as_str(), more.unread), ("needs_you", true));
    assert_eq!(notices.len(), 1);
}

/// The fix node of `running_criteria_mission`, its pull request 7 merged
/// with no line recorded: raised to the Conductor as `merged_unproven`.
fn merged_unproven(runtime: &mut Runtime, now: u64) {
    running_criteria_mission(runtime);
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "pr": 7}}),
    )
    .unwrap();
    let merged = github(vec![pull(7, HEAD, PullRequestBadge::Merged, None)], false);
    runtime.github = merged.clone();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &merged);
    runtime.drive_mission_health(now);
    assert!(node_view(runtime, "fix").attention.is_some());
}

/// The operator's `action` on `node`, made on the raise the map shows now.
fn operator_chose(runtime: &mut Runtime, node: &str, action: &str) {
    let since = node_view(runtime, node)
        .attention
        .map(|attention| attention.since_unix_ms);
    let payload = serde_json::from_value(
        json!({"mission": "mission-1", "node": node, "action": action, "since_unix_ms": since}),
    )
    .unwrap();
    runtime.mission_attention_event(payload);
}

/// PRD mission-node-simple B16, B17, D-15: the operator's `끝으로` finishes
/// the node past each open line as the operator's own decision, and the
/// Conductor is told what was settled instead of the raise.
#[test]
fn the_operators_done_skips_the_open_lines_as_their_decision_and_tells_the_conductor() {
    use crate::mission_criteria::{CriterionState, Role};
    use crate::missions::{AttentionReason, OperatorAction, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let now = crate::runtime::unix_milliseconds();
    merged_unproven(&mut runtime, now);
    // A choice made on a raise that is not the one standing is refused.
    let stale = serde_json::from_value(
        json!({"mission": "mission-1", "node": "fix", "action": "drop", "since_unix_ms": 1}),
    )
    .unwrap();
    runtime.mission_attention_event(stale);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    // The view offers this raise's choices, and one it does not offer is
    // refused on the standing raise too (with-hide/hide#2227).
    let present = runtime.conductor_present(
        &runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .conductor
            .actor,
    );
    assert_eq!(
        node_view(&runtime, "fix").attention.unwrap().choices,
        AttentionReason::MergedUnproven.choices(present)
    );
    operator_chose(&mut runtime, "fix", "attach");
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    assert!(node_view(&runtime, "fix").attention.is_some());
    operator_chose(&mut runtime, "fix", "done");
    let fix = node_view(&runtime, "fix");
    assert_eq!(fix.status, NodeStatus::Done);
    assert!(fix.attention.is_none());
    let skipped: Vec<_> = fix
        .criteria
        .iter()
        .filter(|line| line.state == CriterionState::Skipped)
        .collect();
    assert!(!skipped.is_empty());
    assert!(
        skipped
            .iter()
            .all(|line| { line.by.as_ref().is_some_and(|by| by.role == Role::Operator) })
    );
    let wakes = &runtime.missions.book.get("mission-1").unwrap().wakes;
    assert!(
        !wakes.iter().any(|wake| wake.event == WakeEvent::Attention),
        "the raise's own letter is withdrawn: {wakes:?}"
    );
    assert!(
        wakes
            .iter()
            .any(|wake| wake.event == WakeEvent::Resolved
                && wake.action == Some(OperatorAction::Done)),
        "{wakes:?}"
    );
}

/// PRD mission-node-simple B16, B18: `다시 진행` on a node of a mission Hide
/// closed opens the mission again with the node at work; a mission whose
/// Conductor already conducts another open one is refused with both left
/// as they were.
#[test]
fn the_operators_resume_reopens_a_closed_mission_or_changes_nothing() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    for node in ["cause", "fix"] {
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": node, "patch": {"status": "dropped"}}),
        )
        .unwrap();
    }
    without_gate(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "done", "report": "Done."}),
    )
    .unwrap();
    assert_eq!(mission_status(&runtime), MissionStatus::Done);

    // The same Conductor opens another mission: the first cannot reopen.
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Next"}),
    )
    .unwrap();
    operator_chose(&mut runtime, "fix", "resume");
    assert_eq!(mission_status(&runtime), MissionStatus::Done);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Dropped);

    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    operator_chose(&mut runtime, "fix", "resume");
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
}

/// A node the Conductor takes back to work is not finished again on the
/// pull request it held, and is once the follow-up it names in the same
/// command merges.
#[test]
fn a_node_taken_back_to_work_is_finished_again_only_on_its_next_pull_request() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let now = crate::runtime::unix_milliseconds();
    running_criteria_mission(&mut runtime);
    met_fix(&mut runtime);
    let merged = |runtime: &mut Runtime, pulls: Vec<PullRequestSnapshot>| {
        let read = github(pulls, false);
        runtime.github = read.clone();
        runtime.note_mission_pulls(crate::node::TEST_NODE, &read);
        runtime.drive_mission_health(now);
    };
    merged(
        &mut runtime,
        vec![pull(7, HEAD, PullRequestBadge::Merged, None)],
    );
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Done);

    // Back to work with its follow-up named in the same command.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "pr": 8}}),
    )
    .unwrap();
    merged(
        &mut runtime,
        vec![
            pull(7, HEAD, PullRequestBadge::Merged, None),
            pull(8, HEAD, PullRequestBadge::Open, None),
        ],
    );
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Working);
    merged(
        &mut runtime,
        vec![
            pull(7, HEAD, PullRequestBadge::Merged, None),
            pull(8, HEAD, PullRequestBadge::Merged, None),
        ],
    );
    assert_eq!(node_view(&runtime, "fix").status, NodeStatus::Done);
}

/// PRD mission-node-simple B15: a Conductor still there may be taken over
/// once a raise it was asked first reached the operator, and not while it
/// has its ten minutes or for a raise that goes to the operator at once.
#[test]
fn a_present_conductor_is_taken_over_once_with_the_operators_handover() {
    use crate::missions::{Attention, AttentionReason, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    let raise = |runtime: &mut Runtime, reason, operator_at_unix_ms| {
        runtime.missions.book.get_mut("mission-1").unwrap().nodes[0].attention = Some(Attention {
            reason,
            pr: None,
            agent: None,
            key: 1,
            since_unix_ms: 1,
            operator_at_unix_ms,
            notified: false,
        });
    };
    let take = |runtime: &mut Runtime, pane: &str, token: Option<&str>| {
        let mut asked = json!({"op": "take", "mission": "mission-1"});
        if let Some(token) = token {
            asked["handover"] = json!(token);
        }
        command(runtime, pane, asked)
    };
    let refused = |answer: Result<(Value, bool), String>| {
        crate::coordination::split_refusal(&answer.unwrap_err())
            .0
            .to_owned()
    };
    let token = "0f8e2b9c-4c1d-4b7a-9e3f-5a6b7c8d9e0f";
    // No raise reached the operator: no handover is made, and a take is
    // refused while the Conductor is there.
    raise(&mut runtime, AttentionReason::WorkerEnded, None);
    assert!(!runtime.mission_handover_event("mission-1", token));
    assert_eq!(
        refused(take(&mut runtime, "recipient", Some(token))),
        "mission_conducted"
    );
    // One the operator settles first is no reason either.
    raise(&mut runtime, AttentionReason::ConductorFailed, Some(1));
    assert!(!runtime.mission_handover_event("mission-1", token));
    // Ten minutes unanswered reached the operator, but a take without the
    // operator's handover is still refused (B15).
    raise(&mut runtime, AttentionReason::WorkerEnded, Some(2));
    assert_eq!(
        refused(take(&mut runtime, "recipient", None)),
        "mission_conducted"
    );
    assert!(runtime.mission_handover_event("mission-1", token));
    assert_eq!(
        refused(take(&mut runtime, "recipient", Some("not-the-token-made"))),
        "mission_conducted"
    );
    let (taken, changed) = take(&mut runtime, "recipient", Some(token)).unwrap();
    assert!(changed);
    assert_eq!(taken["mission"]["conductor"]["name"], "recipient");
    // The handover is spent, and what reached the operator because the old
    // Conductor did not answer goes to the new one first, with its own ten
    // minutes, so the old timeout hands it on to no one else.
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert!(mission.handover.is_none());
    let attention = mission.nodes[0].attention.clone().unwrap();
    assert_eq!(attention.operator_at_unix_ms, None);
    assert!(attention.since_unix_ms > 2);
    assert!(
        mission
            .wakes
            .iter()
            .any(|wake| wake.event == WakeEvent::Attention)
    );
    assert!(!runtime.mission_handover_event("mission-1", token));
    assert_eq!(
        refused(take(&mut runtime, "sender", Some(token))),
        "mission_conducted"
    );
}

/// PRD mission-node-simple B18: the operator opens a closed mission again
/// from the map and its Conductor is told; an open one does not move.
#[test]
fn the_operator_opens_a_closed_mission_again_from_the_map() {
    use crate::missions::WakeEvent;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    let closed = runtime.missions.book.get_mut("mission-1").unwrap();
    closed.status = MissionStatus::Done;
    for node in &mut closed.nodes {
        node.status = NodeStatus::Dropped;
    }
    assert!(runtime.mission_reopen_event("mission-1"));
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    // Every node is still finished, and the reopen holds: Hide does not
    // close it again until a node moves.
    runtime.drive_mission_health(crate::runtime::unix_milliseconds());
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    assert!(
        runtime
            .missions
            .book
            .get("mission-1")
            .unwrap()
            .nodes
            .iter()
            .all(|node| node.attention.is_none())
    );
    let wakes = &runtime.missions.book.get("mission-1").unwrap().wakes;
    assert!(
        wakes.iter().any(|wake| wake.event == WakeEvent::Reopened),
        "{wakes:?}"
    );
    assert!(!runtime.mission_reopen_event("mission-1"));
}

/// PRD mission-node-simple B18: a reopen holds the mission open until a node
/// moves, and a node that went back to work and finished again has moved,
/// though it looks as it did, so Hide closes the mission again.
#[test]
fn a_reopened_mission_whose_node_worked_and_finished_again_closes_again() {
    use crate::missions::OperatorAction;
    use crate::runtime::mission_health::MissionAttentionPayload;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    let closed = runtime.missions.book.get_mut("mission-1").unwrap();
    closed.status = MissionStatus::Done;
    for node in &mut closed.nodes {
        node.status = NodeStatus::Dropped;
    }
    // The operator sets a dropped node to work again: the mission opens.
    assert!(runtime.mission_attention_event(MissionAttentionPayload {
        mission: "mission-1".into(),
        node: "docs".into(),
        action: OperatorAction::Resume,
        since_unix_ms: None,
    }));
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    assert_eq!(
        runtime.missions.book.get("mission-1").unwrap().reopened_on,
        None
    );
    // Its Conductor drops it again: every node looks as the reopen found
    // it, and the mission closes.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "docs", "patch": {"status": "dropped"}}),
    )
    .unwrap();
    runtime.drive_mission_health(crate::runtime::unix_milliseconds());
    assert_ne!(mission_status(&runtime), MissionStatus::Running);
}

/// PRD mission-node-simple B28: with every letter slot taken a new wake is
/// refused and those waiting keep their place.
#[test]
fn a_full_wake_queue_keeps_what_waits() {
    use crate::missions::{PENDING_WAKE_LIMIT, WakeEvent};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    for _ in 0..PENDING_WAKE_LIMIT {
        assert!(
            crate::runtime::missions::record_wake(
                mission,
                "a".into(),
                None,
                WakeEvent::Merged,
                None,
                None
            )
            .is_some()
        );
    }
    assert!(
        crate::runtime::missions::record_wake(
            mission,
            "b".into(),
            None,
            WakeEvent::Closed,
            None,
            None
        )
        .is_none()
    );
    assert_eq!(mission.wakes.len(), PENDING_WAKE_LIMIT);
    assert_eq!(mission.wakes[0].seq, 1);
    // A worker's wake is capped apart, so the Conductor's letters never
    // starve it.
    let worker = crate::missions::PendingWake {
        seq: 99,
        node: "a".into(),
        pr: Some(7),
        event: WakeEvent::Fix,
        head: None,
        to: Some("agent-2".into()),
        reason: None,
        why: None,
        action: None,
        warning: None,
        ci: None,
        key: None,
    };
    assert!(crate::runtime::missions::push_wake(mission, worker).is_some());
}

/// PRD mission-node-simple B27: an act Hide's evaluation makes is keyed by
/// the facts it stands on, so the same facts evaluated again, as after a
/// restart that lost the book's last save, write one history event and make
/// the same letter, whatever was saved first.
#[test]
fn the_same_facts_evaluated_twice_write_one_event_and_one_letter() {
    use crate::mission_events::EventKind;
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    let lost = runtime.missions.book.clone();
    let raise = |runtime: &mut Runtime, now: u64| {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        let events = crate::runtime::mission_health::raise(
            mission,
            "fix",
            AttentionReason::WorkerEnded,
            None,
            now,
        );
        let wake = mission.wakes.last().cloned().unwrap();
        let intent = crate::runtime::missions::wake_intent(mission, &wake);
        runtime.record_mission_events("mission-1", events);
        intent
    };
    let first = raise(&mut runtime, 5);
    // The save is lost: the book is as it was before the raise, the history
    // and the letter are not.
    runtime.missions.book = lost;
    let second = raise(&mut runtime, 9);
    assert_eq!(first, second);
    let raised = runtime.missions.events["mission-1"]
        .events
        .iter()
        .filter(|event| event.kind == EventKind::Raised)
        .count();
    assert_eq!(raised, 1);
}

/// PRD mission-node-simple B27, review 4 P2-1: a raise that reaches the
/// operator, written by the missions' one writer to real files, is in the
/// history once across a restart whichever save failed: a book that failed
/// after its history is made again under the same keys, and a history that
/// failed holds the book back, so no restart reads a change its history
/// lost.
#[test]
fn a_raise_to_the_operator_is_in_the_history_once_whichever_save_failed_before_a_restart() {
    use crate::mission_events::{EventKind, MissionEventStore};
    use crate::missions::{AttentionReason, MissionStore, Restored};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running(&mut runtime);
    let before = runtime.missions.book.clone();
    let dir = tempfile::tempdir().unwrap();
    let history = dir.path().join("mission-events");
    let book = dir.path().join("state/missions.json");
    let start = |runtime: &mut Runtime| {
        let store = MissionEventStore::with_book(history.clone(), book.clone());
        let logs = store.restore(["mission-1"].into_iter());
        runtime.install_mission_events(store, logs);
    };
    // One evaluation: the Conductor is asked, and ten minutes later the
    // operator.
    let evaluate = |runtime: &mut Runtime, now: u64| {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        let events = crate::runtime::mission_health::raise(
            mission,
            "fix",
            AttentionReason::PrClosed,
            Some(7),
            now,
        );
        runtime.record_mission_events("mission-1", events);
        runtime.raise_to_operator(now + 10 * 60 * 1000);
    };
    let stop = |runtime: &mut Runtime| drop(runtime.missions.event_store.take());
    let written = || {
        let log = MissionEventStore::new(history.clone()).restore(["mission-1"].into_iter());
        let count = |kind: EventKind| {
            log.get("mission-1").map_or(0, |log| {
                log.events.iter().filter(|event| event.kind == kind).count()
            })
        };
        (count(EventKind::Raised), count(EventKind::RaisedToOperator))
    };
    let saved_book = || match MissionStore::new(book.clone()).restore() {
        Restored::Read(book) => Some(book),
        _ => None,
    };

    // The book cannot be written (a file stands where its folder goes);
    // the history is written first and stays.
    std::fs::write(dir.path().join("state"), "a file").unwrap();
    start(&mut runtime);
    evaluate(&mut runtime, 1_000);
    stop(&mut runtime);
    assert_eq!(written(), (1, 1));
    assert!(saved_book().is_none());
    // A restart reads the book from before the raise and makes it again,
    // later: the history keeps one of each, and the book is written now.
    std::fs::remove_file(dir.path().join("state")).unwrap();
    runtime.missions.book = before.clone();
    start(&mut runtime);
    evaluate(&mut runtime, 5_000);
    stop(&mut runtime);
    assert_eq!(written(), (1, 1));
    let saved = saved_book().expect("the book is written once its folder can be");
    let attention = saved
        .get("mission-1")
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.key == "fix")
        .and_then(|node| node.attention.clone());
    assert!(attention.is_some_and(|attention| attention.operator_at_unix_ms.is_some()));

    // The other order: the history cannot be written, so the book waits
    // and the restart finds neither.
    std::fs::remove_dir_all(&history).unwrap();
    std::fs::remove_file(&book).unwrap();
    std::fs::write(&history, "a file").unwrap();
    runtime.missions.book = before.clone();
    runtime.missions.events.clear();
    start(&mut runtime);
    evaluate(&mut runtime, 9_000);
    stop(&mut runtime);
    assert!(saved_book().is_none(), "the book waits for its history");
    std::fs::remove_file(&history).unwrap();
    runtime.missions.book = before;
    runtime.missions.events.clear();
    start(&mut runtime);
    evaluate(&mut runtime, 12_000);
    stop(&mut runtime);
    assert_eq!(written(), (1, 1));
    assert!(saved_book().is_some());
}

/// PRD mission-node-simple B27: a node that cannot remember a new cause
/// does not raise it either, so nothing is raised that would come again
/// once settled; what it already remembers stays.
#[test]
fn a_node_that_cannot_remember_a_cause_does_not_raise_it() {
    use crate::missions::{AttentionReason, HANDLED_LIMIT};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    for index in 0..HANDLED_LIMIT {
        assert!(mission.nodes[0].note_handled(format!("cause:{index}")));
    }
    let wakes = mission.wakes.len();
    let raised = crate::runtime::mission_health::raise(
        mission,
        "fix",
        AttentionReason::WorkerEnded,
        None,
        5,
    );
    assert!(raised.is_empty());
    assert!(mission.nodes[0].attention.is_none());
    assert_eq!(mission.wakes.len(), wakes);
    assert_eq!(mission.attention_seq, 0);
    assert_eq!(mission.nodes[0].handled.len(), HANDLED_LIMIT);
}

const DEVICE: &str = "mini";
const DEVICE_ROOT: &str = "/Users/example/fixture";

/// `DEVICE`'s link in `state`, its engine session showing `workspaces`.
fn device_status(
    state: &str,
    workspaces: Vec<WorkspaceSnapshot>,
) -> crate::model::RemoteStatusSnapshot {
    crate::model::RemoteStatusSnapshot {
        target_id: DEVICE.into(),
        state: state.into(),
        message: None,
        session: Some(crate::model::RemoteSessionSnapshot {
            workspaces,
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
        }),
        files: crate::model::RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    }
}

/// GitHub's answer for the project at `root`, of the repository GitHub
/// gives the id `id`, or of none it has read.
fn repository_read(root: &str, id: Option<&str>) -> GithubSnapshot {
    let mut project = GithubProjectSnapshot {
        pull_requests_read: true,
        root_path: root.into(),
        ..Default::default()
    };
    project.issues.repository_id = id.map(str::to_owned);
    GithubSnapshot {
        projects: vec![project],
    }
}

/// An agent registered from a pane on `DEVICE`.
fn far_agent(id: &str, pane: &str) -> (Actor, crate::coordination::AgentRecord) {
    let actor = Actor {
        pane_id: format!("remote:{DEVICE}:pane:{pane}"),
        name: pane.into(),
        kind: "codex".into(),
        device_id: DEVICE.into(),
        session: crate::engine::tokens::session_digest(&format!("{pane}-session")),
    };
    let record = crate::coordination::AgentRecord {
        id: id.into(),
        name: Some(pane.into()),
        machine: DEVICE.into(),
        host_scope: DEVICE.into(),
        native_machine: "machine-mini".into(),
        session: format!("{pane}-session"),
        instance: pane.into(),
        pane: pane.into(),
        parent: Some("agent-1".into()),
        origin: None,
        project: None,
        actor: actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    };
    (actor, record)
}

fn register(runtime: &mut Runtime, record: crate::coordination::AgentRecord) {
    let mut ledger = runtime
        .delivery_ledger
        .as_ref()
        .map(|ledger| (**ledger).clone())
        .unwrap_or_default();
    ledger.agents.push(record);
    runtime.delivery_ledger = Ok(Arc::new(ledger));
}

/// A Conductor's worker on another machine than the project's starts only
/// for a pull-request change node, on a connected machine that holds a
/// project of the mission's GitHub repository, known by the id GitHub gives
/// it; each other case is refused with its own reason. A checker, whose
/// intent names no node, is admitted on the same machine facts.
#[test]
fn a_worker_on_another_machine_starts_only_for_a_pull_request_change_of_the_same_repository() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let conductor = fixture.conductor.clone();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug", "cap": 4}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "cause", "title": "Find the cause", "kind": "research", "criteria": ["it is found"]},
            {"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr"},
            {"key": "land", "title": "Land it", "kind": "change", "criteria": ["it works"], "delivery": "direct"},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    runtime.github = repository_read(PROJECT, Some("R_1"));
    let admit = |runtime: &Runtime, machine: Option<&str>, intent: &str| {
        runtime
            .mission_spawn_admission(
                &conductor,
                &worker(machine, "codex", &Default::default()),
                &mut None,
                intent,
                true,
            )
            .map(|admission| admission.map(|admission| admission.mission))
    };
    let refused =
        |runtime: &Runtime, intent: &str| admit(runtime, Some(DEVICE), intent).unwrap_err();

    // A direct change's branch would stay there, and a research node
    // answers on the project's machine.
    for node in ["land", "cause"] {
        assert_eq!(
            crate::coordination::split_refusal(&refused(&runtime, &format!("mission-1:{node}"))),
            ("mission_machine_node_refused", Some(node))
        );
    }
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_machine_unavailable"
    );
    runtime
        .snapshot
        .status
        .remote
        .push(device_status("disconnected", Vec::new()));
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_machine_unavailable"
    );
    runtime.snapshot.status.remote[0] = device_status("connected", Vec::new());
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_projects_loading"
    );
    runtime
        .device_worktrees
        .insert(DEVICE.into(), Default::default());
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_machine_no_repository"
    );
    runtime
        .device_github
        .insert(DEVICE.into(), repository_read(DEVICE_ROOT, Some("R_2")));
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_machine_no_repository",
        "another repository on that machine is not the mission's"
    );
    runtime
        .device_github
        .insert(DEVICE.into(), repository_read(DEVICE_ROOT, Some("R_1")));
    let mission = Some("mission-1".to_owned());
    assert_eq!(
        admit(&runtime, Some(DEVICE), "mission-1:fix").unwrap(),
        mission
    );
    assert_eq!(
        admit(&runtime, Some(DEVICE), "mission-1:review-fix").unwrap(),
        mission,
        "a checker names its node only when it is bound"
    );
    // A mission whose own repository GitHub has not named proves no match.
    runtime.github = repository_read(PROJECT, None);
    assert_eq!(
        refused(&runtime, "mission-1:fix"),
        "mission_machine_no_repository"
    );
    // A read that failed names what to fix, here a gh too old for Hide.
    runtime.github.projects[0].status.failure_category =
        Some(hide_node_link::gh::GithubFailureCategory::Outdated);
    assert_eq!(
        crate::coordination::split_refusal(&refused(&runtime, "mission-1:fix")),
        (
            "mission_machine_no_repository",
            Some("GitHub is not read: gh on the core's machine is older than 2.72.0; update it")
        )
    );
    // The project's own machine runs every node's worker as before.
    assert_eq!(admit(&runtime, None, "mission-1:land").unwrap(), mission);
}

/// `DEVICE` connected with `workspaces` in its session, its project list
/// read, and GitHub naming the mission's repository `R_1` for the mission's
/// project and for `DEVICE_ROOT` there.
fn connect_with_repository(runtime: &mut Runtime, workspaces: Vec<WorkspaceSnapshot>) {
    runtime.github = repository_read(PROJECT, Some("R_1"));
    runtime
        .snapshot
        .status
        .remote
        .retain(|remote| remote.target_id != DEVICE);
    runtime
        .snapshot
        .status
        .remote
        .push(device_status("connected", workspaces));
    runtime
        .device_worktrees
        .insert(DEVICE.into(), Default::default());
    runtime
        .device_github
        .insert(DEVICE.into(), repository_read(DEVICE_ROOT, Some("R_1")));
}

/// An agent registered on another machine is bound to a node, as its worker
/// or its checker, only where a spawn for that node would start it there:
/// a pull-request change, on a connected machine holding the mission's
/// repository. An agent the node holds stays through a time its machine is
/// away, and a replan is refused only for making its node one the
/// project's machine runs.
#[test]
fn an_agent_on_another_machine_is_bound_only_where_a_spawn_there_is_admitted() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let plan = |delivery: &str, title: &str| {
        let review = json!([{"as": "review", "by": [{"agent": "codex", "model": "gpt-6-astra"}]}]);
        json!({"op": "plan", "plan": {"check": "cargo test", "nodes": [
            {"key": "fix", "title": title, "kind": "change", "delivery": delivery,
             "criteria": ["the parser keeps v1 files"], "checks": review},
            {"key": "land", "title": "Land it", "kind": "change", "delivery": "direct",
             "criteria": ["it is in main"], "checks": review},
        ]}})
    };
    command(&mut runtime, "sender", plan("pr", "Fix it")).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(&mut runtime);
    register(&mut runtime, far_agent("agent-9", "w2:p0").1);
    let bind = |node: &str, patch: Value| json!({"op": "node", "node": node, "patch": patch});
    let refused = |runtime: &mut Runtime, command_json: Value| {
        let refusal = command(runtime, "sender", command_json).unwrap_err();
        let (code, detail) = crate::coordination::split_refusal(&refusal);
        (code.to_owned(), detail.map(str::to_owned))
    };
    let code = |code: &str, detail: Option<&str>| (code.to_owned(), detail.map(str::to_owned));

    for patch in [
        json!({"agent_id": "agent-9"}),
        json!({"reviewer": "agent-9"}),
    ] {
        assert_eq!(
            refused(&mut runtime, bind("land", patch.clone())),
            code("mission_machine_node_refused", Some("land"))
        );
        assert_eq!(
            refused(&mut runtime, bind("fix", patch)),
            code("mission_machine_unavailable", None),
            "an agent from outside the mission is held to the spawn's rule"
        );
    }
    connect_with_repository(&mut runtime, Vec::new());
    command(
        &mut runtime,
        "sender",
        bind("fix", json!({"agent_id": "agent-9", "status": "working"})),
    )
    .unwrap();
    // An agent on the project's machine binds to either node.
    command(
        &mut runtime,
        "sender",
        bind(
            "land",
            json!({"agent_id": "agent-2", "reviewer": "agent-3"}),
        ),
    )
    .unwrap();

    // Its machine away for a while, the agent the node holds stays.
    runtime.snapshot.status.remote[0] = device_status("disconnected", Vec::new());
    command(
        &mut runtime,
        "sender",
        bind("fix", json!({"agent_id": "agent-9"})),
    )
    .unwrap();
    command(&mut runtime, "sender", plan("pr", "Fix the parser")).unwrap();
    assert_eq!(
        refused(&mut runtime, plan("direct", "Fix the parser")),
        code("mission_machine_node_refused", Some("fix"))
    );
    assert_eq!(
        node_view(&runtime, "fix").delivery,
        Some(crate::missions::Delivery::Pr)
    );
}

/// The workspace of `DEVICE`'s session with `far`'s checkout at `path`,
/// at `head`.
fn far_checkout(far: &Actor, path: &str, head: &str) -> WorkspaceSnapshot {
    let mut checkout = checkout(
        "remote:mini:workspace:w2",
        "remote:mini:checkout:w2",
        path,
        Some(pane(&far.pane_id, path)),
    );
    checkout.head_oid = Some(head.into());
    let mut workspace = workspace(
        "remote:mini:workspace:w2",
        "fixture",
        DEVICE_ROOT,
        vec![checkout],
    );
    workspace.device_id = DEVICE.into();
    workspace.remote_target_id = Some(DEVICE.into());
    workspace
}

/// The fix node worked by `agent-9` on `DEVICE`, in a checkout there at the
/// same path as the project's machine's, which has recorded its check at
/// `HEAD`; answers that agent and the path.
fn record_far_check(runtime: &mut Runtime) -> (Actor, String) {
    running_criteria_mission(runtime);
    let path = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .path
        .clone();
    let (far, record) = far_agent("agent-9", "w2:p0");
    register(runtime, record);
    connect_with_repository(runtime, vec![far_checkout(&far, &path, HEAD)]);
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-9"}}),
    )
    .unwrap();
    let (_, changed) = runtime
        .record_criterion(
            &far,
            None,
            "fix",
            "check",
            (
                true,
                "cargo test passed".into(),
                None,
                Some(HEAD.into()),
                false,
            ),
            1,
        )
        .unwrap();
    assert!(changed);
    (far, path)
}

const OTHER_HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

/// A worker on another machine records its check from its checkout there,
/// and the line holds against that machine's checkout while the machine
/// answers: the project's machine's checkout at the same path says nothing
/// of it, a machine away proves nothing, and a new commit there asks for
/// the check again.
#[test]
fn a_check_recorded_on_another_machine_holds_against_that_machines_checkout() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let (far, path) = record_far_check(&mut runtime);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
    let node = &runtime.missions.book.get("mission-1").unwrap().nodes[0];
    assert_eq!(
        node.check_folder(crate::node::TEST_NODE),
        Some((DEVICE, path.as_str()))
    );
    checkout_at(&mut runtime, OTHER_HEAD);
    assert_eq!(
        line_state(&runtime, "check"),
        CriterionState::Pass,
        "the same path on the project's machine is another checkout"
    );

    runtime.snapshot.status.remote[0] =
        device_status("disconnected", vec![far_checkout(&far, &path, HEAD)]);
    assert_eq!(
        line_state(&runtime, "check"),
        CriterionState::Open,
        "the last session of a machine away proves no head"
    );
    runtime.snapshot.status.remote[0].state = "connected".into();
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
    runtime.snapshot.status.remote[0] =
        device_status("connected", vec![far_checkout(&far, &path, OTHER_HEAD)]);
    assert_eq!(
        line_state(&runtime, "check"),
        CriterionState::Open,
        "a new commit on that machine's checkout asks for the check again"
    );
}

/// A check on the mission's machine is written in `folder` alone, as builds
/// from before workers ran elsewhere wrote and read it; one on another
/// machine only in `remote_folder`, a field such a build does not know: it
/// reads that check as open, and once it saved the book without the field
/// this build reads it open too, never the device's path as a checkout on
/// the mission's machine.
#[test]
fn a_check_folder_is_read_the_same_by_a_build_that_predates_devices() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    let (_, path) = record_far_check(&mut runtime);
    let saved_by_old_build = |runtime: &mut Runtime| {
        let node = &mut runtime.missions.book.get_mut("mission-1").unwrap().nodes[0];
        let mut written = serde_json::to_value(&*node).unwrap();
        written.as_object_mut().unwrap().remove("remote_folder");
        *node = serde_json::from_value(written.clone()).unwrap();
        written
    };
    let node = &runtime.missions.book.get("mission-1").unwrap().nodes[0];
    assert_eq!(
        serde_json::to_value(node).unwrap()["remote_folder"],
        json!([DEVICE, path])
    );
    let written = saved_by_old_build(&mut runtime);
    assert!(written.get("folder").is_none(), "{written}");
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);

    // Another worker on the project's machine records it there.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"agent_id": "agent-2"}}),
    )
    .unwrap();
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    let written = saved_by_old_build(&mut runtime);
    assert_eq!(written["folder"], json!(path));
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
}

/// The `fix` node's warnings and the warning letters waiting for its
/// Conductor.
fn warnings_of(runtime: &Runtime) -> (Vec<crate::missions::WarningKind>, Vec<String>) {
    let mission = runtime.missions.book.get("mission-1").unwrap();
    let node = mission.nodes.iter().find(|node| node.key == "fix").unwrap();
    let kinds = node.warnings.iter().map(|warning| warning.kind).collect();
    let letters = mission
        .wakes
        .iter()
        .filter(|wake| wake.event == crate::missions::WakeEvent::Warning)
        .map(|wake| {
            assert_eq!(wake.to, None, "a warning is the Conductor's");
            super::super::missions::wake_intent(mission, wake)
        })
        .collect();
    (kinds, letters)
}

/// `hide mission show` prints what the core decided a node waits on: a
/// planned node the nodes before it, then nothing, a working one its first
/// line not met with a failed line its worker's, then its pull request's
/// merge, and a decision that stopped it before any of those.
#[test]
fn a_node_waits_on_what_the_core_decides_from_its_lines_and_its_decision() {
    use crate::missions::WaitingOn;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    let mut plan = criteria_plan();
    plan["plan"]["nodes"][1]["depends_on"] = json!(["fix"]);
    command(&mut runtime, "sender", plan).unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(&mut runtime);
    checkout_at(&mut runtime, HEAD);
    let waits = |runtime: &Runtime, key: &str| node_view(runtime, key).waiting_on;
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Start));
    assert_eq!(
        waits(&runtime, "docs"),
        Some(WaitingOn::Nodes {
            keys: vec!["fix".into()]
        })
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "reviewer": "agent-3"}}),
    )
    .unwrap();
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Worker));
    assert!(node_view(&runtime, "fix").asks_push);
    assert!(!node_view(&runtime, "docs").asks_push);
    command(&mut runtime, "recipient", record_of("fix", "c1", json!({}))).unwrap();
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Review));
    let mut failed = record_of("fix", "review", json!({}));
    failed["pass"] = json!(false);
    command(&mut runtime, "reviewer", failed).unwrap();
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Worker));
    met_fix(&mut runtime);
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Merge));
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "Ship it?", "node": "fix"}),
    )
    .unwrap();
    assert_eq!(waits(&runtime, "fix"), Some(WaitingOn::Decision));
    assert_eq!(
        waits(&runtime, "docs"),
        Some(WaitingOn::Nodes {
            keys: vec!["fix".into()]
        })
    );
}

/// with-hide/hide-private#552 c2: the review failing twice in a row warns
/// the Conductor with one letter and marks the node; nothing reaches the
/// operator, a third failure sends nothing more, and the review passing
/// takes the warning, and a letter not sent yet, away.
#[test]
fn a_review_failed_twice_in_a_row_warns_the_conductor_alone_until_it_passes() {
    use crate::missions::WarningKind;
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "reviewer": "agent-3"}}),
    )
    .unwrap();
    let now = crate::runtime::unix_milliseconds();
    let review = |runtime: &mut Runtime, pass: bool, round: u32| {
        let mut record = record_of(
            "fix",
            "review",
            json!({"evidence": format!("round {round}")}),
        );
        record["pass"] = json!(pass);
        command(runtime, "reviewer", record).unwrap();
        runtime.drive_mission_health(now);
    };
    review(&mut runtime, false, 1);
    assert_eq!(warnings_of(&runtime), (vec![], vec![]));
    review(&mut runtime, false, 2);
    let (kinds, letters) = warnings_of(&runtime);
    assert_eq!(kinds, [WarningKind::ReviewFails]);
    assert_eq!(letters.len(), 1, "{letters:?}");
    let shown = runtime.mission_view_of("mission-1").unwrap();
    assert_eq!(shown.nodes[0].review_fails, 2);
    assert_eq!(shown.nodes[0].warnings.len(), 1);
    // It is no attention: the operator is never asked, however long it
    // stands.
    assert!(shown.nodes[0].attention.is_none());
    assert!(!runtime.raise_to_operator(now + 3 * 60 * 60_000));
    // The same plan applied again keeps the review line, so the count and
    // the warning stand, with no second letter.
    command(&mut runtime, "sender", criteria_plan()).unwrap();
    runtime.drive_mission_health(now);
    assert_eq!(
        warnings_of(&runtime),
        (vec![WarningKind::ReviewFails], letters.clone()),
        "a replan keeps the warning"
    );
    review(&mut runtime, false, 3);
    assert_eq!(
        warnings_of(&runtime).1,
        letters,
        "the same warning, one letter"
    );
    review(&mut runtime, true, 4);
    assert_eq!(warnings_of(&runtime), (vec![], vec![]));
}

/// A second `codex` reviewer, `reviewer2` (`agent-4`), beside the pair
/// `with_worker_and_reviewer` registers.
fn with_second_reviewer(runtime: &mut Runtime) {
    let panes = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0].panes;
    let mut pane = panes
        .iter()
        .find(|pane| pane.id == "reviewer")
        .unwrap()
        .clone();
    pane.id = "reviewer2".into();
    panes.push(pane);
    let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":2,"lineage_session":"sender-session"},
        {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":2,"lineage_session":"recipient-session"},
        {"id":"reviewer","pane_id":"reviewer","agent":"codex","agent_status":"working","state_change_seq":2,"lineage_session":"reviewer-session"},
        {"id":"reviewer2","pane_id":"reviewer2","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"reviewer2-session"},
    ]}))
    .unwrap();
    runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
    let mut second = ledger
        .agents
        .iter()
        .find(|record| record.id == "agent-3")
        .unwrap()
        .clone();
    second.id = "agent-4".into();
    second.name = Some("reviewer2".into());
    second.session = "reviewer2-session".into();
    second.instance = "reviewer2".into();
    second.pane = "reviewer2".into();
    second.actor.pane_id = "reviewer2".into();
    second.actor.name = "reviewer2".into();
    second.actor.session = Some("reviewer2-session".into());
    ledger.agents.push(second);
    runtime.delivery_ledger = Ok(Arc::new(ledger));
}

/// A mission whose node `fix` is worked by `agent-2` and whose review line
/// asks three `codex` checkers by rule `all`, with `reviewer` (`agent-3`)
/// bound to the first slot and, when `second`, `reviewer2` (`agent-4`) to
/// the next; the third slot is never taken, as in mission-35.
fn three_slot_review(second: bool) -> Fixture {
    let fixture = conducting();
    {
        let mut runtime = fixture.runtime.lock().unwrap();
        command(
            &mut runtime,
            "sender",
            json!({"op": "create", "title": "Fix the bug"}),
        )
        .unwrap();
        let slot = json!({"agent": "codex", "model": "gpt-6-astra"});
        let by = json!([slot, slot, slot]);
        command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
                 "criteria": ["the parser keeps v1 files"],
                 "checks": [{"as": "review", "rule": "all", "by": by}]},
            ]}}),
        )
        .unwrap();
        command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
        with_worker_and_reviewer(&mut runtime);
        with_second_reviewer(&mut runtime);
        command(
            &mut runtime,
            "sender",
            json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "reviewer": "agent-3"}}),
        )
        .unwrap();
        if second {
            command(
                &mut runtime,
                "sender",
                json!({"op": "node", "node": "fix", "patch": {"reviewer": "agent-4"}}),
            )
            .unwrap();
        }
    }
    fixture
}

/// The state of node `fix`'s review line.
fn review_state(runtime: &Runtime) -> CriterionState {
    runtime.missions.book.missions[0].nodes[0]
        .criteria
        .iter()
        .find(|line| line.key == "review")
        .unwrap()
        .state
}

/// with-hide/hide-private#655 c1: a review line asking three checkers by
/// rule `all`, with one bound, is decided by that one; the slots nobody took
/// do not hold the line open, and the line opens no sooner than a checker is
/// bound.
#[test]
fn a_review_line_is_decided_by_the_checkers_bound_not_the_empty_slots() {
    let fixture = three_slot_review(false);
    let mut runtime = fixture.runtime.lock().unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Open);
    command(
        &mut runtime,
        "reviewer",
        record_of("fix", "review", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Pass);
    // The same slot failing is a failure of the line, not an open one.
    let mut fail = record_of(
        "fix",
        "review",
        json!({"commit": HEAD, "evidence": "broken"}),
    );
    fail["pass"] = json!(false);
    command(&mut runtime, "reviewer", fail).unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Fail);
}

/// with-hide/hide-private#655 c2: a first review failing a head and a second
/// reviewer passing the next head leaves the line passed, whichever slot
/// each took; a second answer on the same head leaves the failure standing.
#[test]
fn a_pass_on_a_new_head_replaces_the_failure_of_an_older_head() {
    let fixture = three_slot_review(true);
    let mut runtime = fixture.runtime.lock().unwrap();
    let newer = "b2c3d4e5f60718293a4b5c6d7e8f9012345678a1";
    let mut fail = record_of(
        "fix",
        "review",
        json!({"commit": HEAD, "evidence": "broken"}),
    );
    fail["pass"] = json!(false);
    command(&mut runtime, "reviewer", fail).unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Fail);
    // Another reviewer passing the same head cannot clear the failure.
    command(
        &mut runtime,
        "reviewer2",
        record_of("fix", "review", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Fail);
    // The worker pushes; the second reviewer reads the new head and passes
    // it. The first reviewer's failure is of the head before.
    command(
        &mut runtime,
        "reviewer2",
        record_of(
            "fix",
            "review",
            json!({"commit": newer, "evidence": "fixed"}),
        ),
    )
    .unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Pass);
    assert_eq!(
        runtime.missions.book.missions[0].nodes[0]
            .criteria
            .iter()
            .find(|line| line.key == "review")
            .unwrap()
            .fails_in_a_row,
        0
    );
}

/// with-hide/hide-private#655 review: a checker's answer is for the head it
/// names, never one guessed from the pull request when it is recorded, so a
/// review of a node with a pull request is refused without `--commit`, and one
/// of a node without a pull request needs none.
#[test]
fn a_review_of_a_node_with_a_pull_request_names_its_commit() {
    let fixture = three_slot_review(false);
    let mut runtime = fixture.runtime.lock().unwrap();
    let plain = record_of("fix", "review", json!({}));
    command(&mut runtime, "reviewer", plain.clone()).unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Pass);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
    let mut fail = record_of("fix", "review", json!({"evidence": "broken"}));
    fail["pass"] = json!(false);
    assert_eq!(
        command(&mut runtime, "reviewer", fail).unwrap_err(),
        "mission_commit_required"
    );
    assert_eq!(
        review_state(&runtime),
        CriterionState::Pass,
        "a refused answer changes nothing"
    );
    let mut fail = record_of(
        "fix",
        "review",
        json!({"commit": HEAD, "evidence": "broken"}),
    );
    fail["pass"] = json!(false);
    command(&mut runtime, "reviewer", fail).unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Fail);
}

/// with-hide/hide-private#655 review: a checker the Conductor binds after the
/// line passed opens it again, and the node is not done until that checker
/// answers.
#[test]
fn a_checker_bound_after_the_review_passed_keeps_the_node_from_done() {
    let fixture = three_slot_review(false);
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "reviewer",
        record_of("fix", "review", json!({})),
    )
    .unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Pass);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"reviewer": "agent-4"}}),
    )
    .unwrap();
    assert_eq!(review_state(&runtime), CriterionState::Open);
    let shown = runtime.mission_view_of("mission-1").unwrap();
    let line = shown.nodes[0]
        .criteria
        .iter()
        .find(|line| line.key == "review")
        .unwrap();
    assert_eq!(
        line.tally.map(|tally| (tally.passed, tally.total)),
        Some((1, 2))
    );
}

/// with-hide/hide-private#552 review: a warning raised while the
/// Conductor's letters are full stands untold, and the evaluation after the
/// queue has room tells it, once.
#[test]
fn a_warning_raised_while_the_letters_are_full_is_told_once_there_is_room() {
    use crate::missions::{PENDING_WAKE_LIMIT, PendingWake, WakeEvent, WarningKind};
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
    )
    .unwrap();
    let minute = 60_000;
    let worked = crate::runtime::unix_milliseconds() - 120 * minute;
    let mission = runtime.missions.book.get_mut("mission-1").unwrap();
    mission.nodes[0].worked_at_unix_ms = Some(worked);
    mission.wakes = (0..PENDING_WAKE_LIMIT)
        .map(|at| PendingWake {
            seq: at as u64 + 1,
            node: "docs".into(),
            pr: None,
            event: WakeEvent::NewHead,
            head: None,
            to: None,
            reason: None,
            why: None,
            action: None,
            warning: None,
            ci: None,
            key: Some(format!("filler:{at}")),
        })
        .collect();
    runtime.warn_missions(worked + 90 * minute);
    assert_eq!(warnings_of(&runtime), (vec![WarningKind::NoPush], vec![]));
    let fix = |runtime: &Runtime| runtime.missions.book.get("mission-1").unwrap().nodes[0].clone();
    assert_eq!(fix(&runtime).warnings[0].told_unix_ms, None);
    runtime
        .missions
        .book
        .get_mut("mission-1")
        .unwrap()
        .wakes
        .clear();
    assert!(runtime.warn_missions(worked + 91 * minute));
    let (_, letters) = warnings_of(&runtime);
    assert_eq!(letters.len(), 1, "told once there is room");
    assert_eq!(
        fix(&runtime).warnings[0].told_unix_ms,
        Some(worked + 91 * minute)
    );
    assert!(!runtime.warn_missions(worked + 92 * minute));
    runtime
        .missions
        .book
        .get_mut("mission-1")
        .unwrap()
        .wakes
        .clear();
    assert!(!runtime.warn_missions(worked + 93 * minute));
    assert_eq!(warnings_of(&runtime).1, Vec::<String>::new(), "told once");
}

/// with-hide/hide-private#552 c2: a change at work with no push for ninety
/// minutes warns its Conductor at the minute decision, once, and the next
/// push GitHub shows takes the warning away.
#[test]
fn a_change_ninety_minutes_without_a_push_warns_until_github_shows_one() {
    use crate::missions::WarningKind;
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
    )
    .unwrap();
    let minute = 60_000;
    // It went to work two hours ago, so a push read now is after it.
    let worked = crate::runtime::unix_milliseconds() - 120 * minute;
    runtime.missions.book.get_mut("mission-1").unwrap().nodes[0].worked_at_unix_ms = Some(worked);
    assert!(!runtime.warn_missions(worked + 89 * minute));
    assert!(runtime.warn_missions(worked + 90 * minute));
    let (kinds, letters) = warnings_of(&runtime);
    assert_eq!(kinds, [WarningKind::NoPush]);
    assert_eq!(letters.len(), 1);
    assert!(!runtime.warn_missions(worked + 120 * minute));
    let mission = runtime.missions.book.get("mission-1").unwrap();
    let wake = mission
        .wakes
        .iter()
        .find(|wake| wake.event == crate::missions::WakeEvent::Warning)
        .unwrap();
    let body = super::super::missions::wake_body(mission, wake);
    assert!(body.contains("worked 90 minutes with no push"), "{body}");
    // The worker opens its pull request: the head GitHub shows is a push.
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"pr": 7}}),
    )
    .unwrap();
    let pushed = github(vec![pull(7, "b2", PullRequestBadge::Open, None)], false);
    runtime.note_mission_pulls(crate::node::TEST_NODE, &pushed);
    assert!(runtime.warn_missions(crate::runtime::unix_milliseconds()));
    assert_eq!(warnings_of(&runtime), (vec![], vec![]));
}

/// with-hide/hide-private#552 c1: `show <node>` answers that node's view
/// alone with its worker's newest letters, newest first, and refuses a node
/// the plan does not have; without a node it answers the whole mission as
/// before.
#[test]
fn show_of_a_node_answers_its_view_and_its_workers_newest_letters() {
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    let actor = |pane: &str| runtime.delivery_observations[pane].actor.clone();
    let (conductor, worker, reviewer) = (actor("sender"), actor("recipient"), actor("reviewer"));
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    for (n, (from, to)) in [
        (&conductor, &worker),
        (&worker, &conductor),
        (&conductor, &reviewer),
        (&worker, &conductor),
        (&conductor, &worker),
        (&worker, &conductor),
        (&conductor, &worker),
    ]
    .into_iter()
    .enumerate()
    {
        crate::delivery::mailbox::send(
            &mut ledger,
            from,
            to,
            &format!("letter-{n}"),
            &format!("letter {n}\nmore of it"),
            "progress",
            None,
            1_000 + n as u64,
        )
        .unwrap();
    }
    runtime.publish_delivery(Arc::new(ledger), true);

    let (whole, _) = command(&mut runtime, "sender", json!({"op": "show"})).unwrap();
    assert_eq!(whole["mission"]["nodes"].as_array().unwrap().len(), 2);
    let (shown, changed) =
        command(&mut runtime, "sender", json!({"op": "show", "node": "fix"})).unwrap();
    assert!(!changed);
    assert_eq!(shown["mission_id"], "mission-1");
    assert_eq!(shown["node"]["key"], "fix");
    assert_eq!(shown["node"], whole["mission"]["nodes"][0]);
    let lines: Vec<&str> = shown["letters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|letter| letter["line"].as_str().unwrap())
        .collect();
    // The reviewer's letter is not the worker's; five at most, newest first.
    assert_eq!(
        lines,
        ["letter 6", "letter 5", "letter 4", "letter 3", "letter 1"]
    );
    assert_eq!(shown["letters"][0]["from"], "sender");
    assert_eq!(
        command(
            &mut runtime,
            "sender",
            json!({"op": "show", "node": "nope"})
        )
        .unwrap_err(),
        "mission_node_unknown"
    );
}

/// with-hide/hide-private#552: a node a book from before push times kept,
/// whose pull request GitHub shows at the head Hide already read, starts its
/// push clock on the first read rather than counting its quiet from when it
/// started work.
#[test]
fn a_node_from_before_push_times_starts_its_clock_on_the_next_read() {
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "pr": 7}}),
    )
    .unwrap();
    let open = github(vec![pull(7, "a1", PullRequestBadge::Open, None)], false);
    runtime.note_mission_pulls(crate::node::TEST_NODE, &open);
    fn node(runtime: &mut Runtime) -> &mut crate::missions::MissionNode {
        &mut runtime.missions.book.get_mut("mission-1").unwrap().nodes[0]
    }
    assert!(node(&mut runtime).pushed_at_unix_ms.is_some());
    // As a book written before the field reads.
    node(&mut runtime).pushed_at_unix_ms = None;
    let before = crate::runtime::unix_milliseconds();
    runtime.note_mission_pulls(crate::node::TEST_NODE, &open);
    assert!(
        node(&mut runtime)
            .pushed_at_unix_ms
            .is_some_and(|at| at >= before)
    );
}

/// The `pane` agent row Hide shows, `status` as the engine reports it, under
/// the screen's name `shown` (a device's pane under its full address).
fn shown_row(pane: &str, shown: &str, status: &str) -> crate::model::SidebarAgentSnapshot {
    let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents": [
        {"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": format!("t-{pane}"),
         "revision": 1, "focused": false, "agent_status": status, "agent": "codex",
         "name": pane, "cwd": PROJECT, "state_change_seq": 1},
    ]}))
    .unwrap();
    let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
    row.pane_id = shown.into();
    row
}

/// with-hide/hide-private#679: a node worked, or checked, from a device
/// names its pane by the full address its agent row has, which the map's
/// 터미널 button finds and opens; the same node worked here keeps the
/// engine's own id.
#[test]
fn a_device_workers_node_names_the_pane_its_agent_row_has() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    assert_eq!(
        node_view(&runtime, "fix").pane_id.as_deref(),
        Some("recipient")
    );

    let (far, record) = far_agent("agent-9", "w2:p0");
    register(&mut runtime, record);
    let (checker, record) = far_agent("agent-10", "w2:p1");
    register(&mut runtime, record);
    connect_with_repository(&mut runtime, Vec::new());
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-9", "reviewer": "agent-10"}}),
    )
    .unwrap();
    runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .agents = vec![
        shown_row("w2:p0", &far.pane_id, "working"),
        shown_row("w2:p1", &checker.pane_id, "working"),
    ];
    let view = node_view(&runtime, "fix");
    assert_eq!(view.pane_id.as_deref(), Some("remote:mini:pane:w2:p0"));
    assert_eq!(view.machine.as_deref(), Some(DEVICE));
    assert_eq!(
        runtime
            .agent_row(view.pane_id.as_deref().unwrap())
            .map(|row| row.pane_id.as_str()),
        Some(far.pane_id.as_str()),
        "the address finds the worker's row"
    );
    let reviewer = view
        .criteria
        .iter()
        .flat_map(|line| &line.checkers)
        .find(|checker| checker.agent_id.as_deref() == Some("agent-10"))
        .unwrap();
    assert_eq!(reviewer.pane_id.as_deref(), Some("remote:mini:pane:w2:p1"));
    assert_eq!(
        view.work,
        Some(crate::runtime::missions::NodeWork::Working),
        "a device worker's work is read from its row there"
    );
    // A device that dropped keeps its last rows; they no longer say what its
    // worker does, so the card says nothing rather than the last word.
    runtime.snapshot.status.remote[0].state = "not_connected".into();
    let view = node_view(&runtime, "fix");
    assert_eq!(view.work, None, "a worker on a device that does not answer");
    assert_eq!(view.pane_id.as_deref(), Some("remote:mini:pane:w2:p0"));
}

/// with-hide/hide-private#680: a node in progress says whether its worker
/// works now: working while its row works, waiting on what the node waits
/// on while the row rests, no worker once its registration ended or with
/// none bound, and nothing at all while Hide cannot see its row; a node not
/// in progress says nothing.
#[test]
fn a_node_in_progress_says_whether_its_worker_works_waits_or_is_gone() {
    use crate::missions::WaitingOn;
    use crate::runtime::missions::NodeWork;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    assert_eq!(node_view(&runtime, "fix").work, None, "a planned node");
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "fix").work, Some(NodeWork::NoWorker));
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2", "reviewer": "agent-3"}}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "fix").work, None, "no row to read");

    runtime.snapshot.navigator.agents = vec![shown_row("recipient", "recipient", "working")];
    assert_eq!(node_view(&runtime, "fix").work, Some(NodeWork::Working));

    runtime.snapshot.navigator.agents = vec![shown_row("recipient", "recipient", "idle")];
    assert_eq!(
        node_view(&runtime, "fix").work,
        Some(NodeWork::Waiting {
            on: WaitingOn::Worker
        })
    );
    command(&mut runtime, "recipient", record_of("fix", "c1", json!({}))).unwrap();
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(
        node_view(&runtime, "fix").work,
        Some(NodeWork::Waiting {
            on: WaitingOn::Review
        }),
        "a resting worker whose node waits on its review"
    );

    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger
        .agents
        .iter_mut()
        .find(|record| record.id == "agent-2")
        .unwrap()
        .ended = true;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(node_view(&runtime, "fix").work, Some(NodeWork::NoWorker));
}

/// The missions are drawn again when a row they read moves, and only then:
/// an agent outside every mission changing its turn leaves the digest the
/// views are kept by as it was, while the worker's row, or its device
/// dropping, changes it (review of with-hide/hide#2211).
#[test]
fn only_a_missions_own_rows_redraw_its_views() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    let digest = |runtime: &Runtime| {
        let ledger = runtime.delivery_state().unwrap();
        runtime.rows_and_holds_digest(Some(&ledger))
    };
    runtime.snapshot.navigator.agents = vec![
        shown_row("recipient", "recipient", "working"),
        shown_row("elsewhere", "elsewhere", "working"),
    ];
    let before = digest(&runtime);
    runtime.snapshot.navigator.agents[1] = shown_row("elsewhere", "elsewhere", "idle");
    assert_eq!(digest(&runtime), before, "a row no mission holds");
    runtime.snapshot.navigator.agents[0] = shown_row("recipient", "recipient", "idle");
    assert_ne!(digest(&runtime), before, "the worker's own row");
}

/// `runtime`'s letter from the `recipient` pane to the Conductor, written
/// `ago` before now; its id.
fn conductor_letter(runtime: &mut Runtime, ago: u64) -> String {
    let conductor = runtime
        .missions
        .book
        .get("mission-1")
        .unwrap()
        .conductor
        .actor
        .clone();
    let from = runtime.delivery_observations["recipient"].actor.clone();
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let intent = format!("report-{}", ledger.next_id);
    let letter = crate::delivery::mailbox::send(
        &mut ledger,
        &from,
        &conductor,
        &intent,
        "Done with the fix",
        "report",
        None,
        crate::delivery::worker::now() - ago,
    )
    .unwrap();
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    letter.id
}

/// with-hide/hide-private#678: letters to a running mission's Conductor
/// that the doorbell holds for something only the operator can lift, for
/// two minutes or more, are counted on the mission with what to do (a pane
/// it cannot reach named first); a hold that ends on its own and a letter
/// held for less than two minutes are not, and a delivered letter leaves
/// the count, the line going with the last.
#[test]
fn a_conductors_held_letters_show_on_its_mission_until_they_are_delivered() {
    use crate::delivery::doorbell::Hold;
    use crate::delivery::held::HeldFor;
    const MINUTE: u64 = 60_000;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    running_criteria_mission(&mut runtime);
    let held = |runtime: &Runtime| {
        runtime
            .mission_view_of("mission-1")
            .unwrap()
            .conductor_letters
            .map(|held| (held.count, held.reason))
    };
    let draft = conductor_letter(&mut runtime, 5 * MINUTE);
    let quiet = conductor_letter(&mut runtime, 5 * MINUTE);
    let fresh = conductor_letter(&mut runtime, MINUTE);
    runtime.delivery_holds = [
        (draft.clone(), Hold::Draft),
        (quiet, Hold::Quiet),
        (fresh.clone(), Hold::Absent),
    ]
    .into();
    let view = runtime.mission_view_of("mission-1").unwrap();
    assert_eq!(
        view.conductor_letters.map(|held| (held.count, held.reason)),
        Some((1, HeldFor::Draft))
    );

    let gone = conductor_letter(&mut runtime, 3 * MINUTE);
    runtime.delivery_holds.insert(gone.clone(), Hold::Absent);
    let shown = runtime
        .mission_view_of("mission-1")
        .unwrap()
        .conductor_letters
        .unwrap();
    assert_eq!(
        (shown.count, shown.reason),
        (2, HeldFor::Unreachable),
        "an unreachable pane is named before a draft"
    );
    assert!(shown.since_unix_ms <= crate::delivery::worker::now() - 5 * MINUTE);

    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    for letter in &mut ledger.letters {
        if letter.id == gone || letter.id == draft {
            letter.state = crate::delivery::ledger::State::Acknowledged;
        }
    }
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(held(&runtime), None, "delivered letters leave");
    assert!(runtime.delivery_holds.contains_key(&fresh));

    // A letter that ended undelivered counts at once, until its sender gets
    // a later one through to the Conductor.
    let lost = conductor_letter(&mut runtime, 0);
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger
        .letters
        .iter_mut()
        .find(|letter| letter.id == lost)
        .unwrap()
        .state = crate::delivery::ledger::State::Undelivered;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(held(&runtime), Some((1, HeldFor::Undelivered)));
    let again = conductor_letter(&mut runtime, 0);
    assert_eq!(
        held(&runtime),
        Some((1, HeldFor::Undelivered)),
        "sent, not taken in"
    );
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger
        .letters
        .iter_mut()
        .find(|letter| letter.id == again)
        .unwrap()
        .state = crate::delivery::ledger::State::Delivered;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(
        held(&runtime),
        None,
        "the later letter's bell was answered: the Conductor is reached again, before any ack"
    );
}

/// A mission of one direct change (no pull request) whose worker recorded
/// both lines at `HEAD` and finished it there.
fn finished_direct_node(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "plan", "plan": {"check": "cargo test", "nodes": [
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "direct",
             "criteria": ["it works"]},
        ]}}),
    )
    .unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
    with_worker_and_reviewer(runtime);
    checkout_at(runtime, HEAD);
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    command(runtime, "recipient", record_of("fix", "c1", json!({}))).unwrap();
    command(
        runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done"}}),
    )
    .unwrap();
}

/// A finished direct node keeps the proof its check gave: the worker then
/// merges into main and the folder moves on, and the mission still closes
/// with no raise, since the node was judged on the commit it finished on.
#[test]
fn a_finished_direct_nodes_check_survives_its_folder_moving() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    checkout_at(&mut runtime, OTHER_HEAD);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert_ne!(mission.status, MissionStatus::Running);
    assert!(
        mission.nodes.iter().all(|node| node.attention.is_none()),
        "{:?}",
        mission.nodes[0].attention
    );
}

/// The proof frozen at a finish goes with it: a node set working again is
/// judged against its folder, so a check from before asks to be run again.
#[test]
fn a_direct_node_set_working_again_needs_a_fresh_check() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    let done_head = |runtime: &Runtime| {
        runtime.missions.book.get("mission-1").unwrap().nodes[0]
            .done_head
            .clone()
    };
    assert_eq!(done_head(&runtime).as_deref(), Some(HEAD));
    checkout_at(&mut runtime, OTHER_HEAD);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working"}}),
    )
    .unwrap();
    assert_eq!(done_head(&runtime), None);
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
    // A fresh check on the new commit finishes it again, on that commit.
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": OTHER_HEAD})),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "done"}}),
    )
    .unwrap();
    assert_eq!(done_head(&runtime).as_deref(), Some(OTHER_HEAD));
}

/// A node finished before the commit was kept has none and is judged
/// against its folder as before; the mission's close is then refused, and
/// the operator's 완료로 닫기 passes the empty line as their own decision
/// and closes it. Drop is not offered, since the work is finished.
#[test]
fn the_operator_closes_a_refused_mission_as_done_passing_the_empty_lines() {
    use crate::mission_criteria::CriterionState;
    use crate::missions::{AttentionReason, OperatorAction};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    runtime.missions.book.get_mut("mission-1").unwrap().nodes[0].done_head = None;
    checkout_at(&mut runtime, OTHER_HEAD);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    let node = node_view(&runtime, "fix");
    let attention = node.attention.expect("the refused close is raised");
    assert_eq!(attention.reason, AttentionReason::CloseRefused);
    assert_eq!(attention.unmet_nodes, 1);
    assert!(attention.choices.contains(&OperatorAction::Done));
    assert!(!attention.choices.contains(&OperatorAction::Drop));
    let payload = |action| crate::runtime::mission_health::MissionAttentionPayload {
        mission: "mission-1".into(),
        node: "fix".into(),
        action,
        since_unix_ms: Some(attention.since_unix_ms),
    };
    assert_eq!(
        runtime
            .mission_attention_act(&payload(OperatorAction::Drop), now)
            .unwrap_err(),
        "mission_attention_choice_refused"
    );
    assert!(
        runtime
            .mission_attention_act(&payload(OperatorAction::Done), now)
            .unwrap()
    );
    assert_ne!(mission_status(&runtime), MissionStatus::Running);
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert!(mission.nodes[0].attention.is_none());
    let check = mission.nodes[0]
        .criteria
        .iter()
        .find(|line| line.key == "check")
        .unwrap();
    assert_eq!(check.state, CriterionState::Skipped);
    assert_eq!(
        check.skip.as_ref().map(|skip| skip.reason.as_str()),
        Some("운영자 판단")
    );
    assert_eq!(
        check.by.as_ref().map(|by| by.role),
        Some(crate::mission_criteria::Role::Operator)
    );
}

/// The same passing check sent again after the folder moved on changes
/// nothing: the proof the node finished on stays, so the line stays passed.
#[test]
fn a_check_sent_again_unchanged_keeps_the_proof_a_node_finished_on() {
    use crate::mission_criteria::CriterionState;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    checkout_at(&mut runtime, OTHER_HEAD);
    command(
        &mut runtime,
        "recipient",
        record_of("fix", "check", json!({"commit": HEAD})),
    )
    .unwrap();
    assert_eq!(line_state(&runtime, "check"), CriterionState::Pass);
    let done_head = runtime.missions.book.get("mission-1").unwrap().nodes[0]
        .done_head
        .clone();
    assert_eq!(done_head.as_deref(), Some(HEAD));
}

/// 완료로 닫기 answers a question about a mission whose every node was
/// finished: once another node works again it is refused and passes nothing,
/// and the next evaluation takes the question and its choices away.
#[test]
fn a_refused_close_is_not_closed_by_the_operator_once_a_node_works_again() {
    use crate::missions::{AttentionReason, NodeStatus, OperatorAction};
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        let mut second = mission.nodes[0].clone();
        second.key = "second".into();
        mission.nodes.push(second);
        mission.nodes[0].done_head = None;
    }
    checkout_at(&mut runtime, OTHER_HEAD);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    let attention = node_view(&runtime, "fix").attention.unwrap();
    assert_eq!(attention.reason, AttentionReason::CloseRefused);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "second", "patch": {"status": "working"}}),
    )
    .unwrap();
    let payload = crate::runtime::mission_health::MissionAttentionPayload {
        mission: "mission-1".into(),
        node: "fix".into(),
        action: OperatorAction::Done,
        since_unix_ms: Some(attention.since_unix_ms),
    };
    assert_eq!(
        runtime.mission_attention_act(&payload, now + 1),
        Err("mission_attention_moved".into())
    );
    let mission = runtime.missions.book.get("mission-1").unwrap();
    assert_eq!(mission.nodes[1].status, NodeStatus::Working);
    assert!(
        mission.nodes[0]
            .criteria
            .iter()
            .all(|line| line.skip.is_none())
    );
    runtime.drive_mission_health(now + 2);
    assert!(node_view(&runtime, "fix").attention.is_none());
}

/// A refused close the evaluation took away because another node worked
/// again is not held back as handled: once that node finishes again and the
/// close is refused again, the question comes back with its own history
/// entry, so the mission is never left running with nothing to answer.
#[test]
fn a_refused_close_is_asked_again_after_a_node_resumed_and_finished() {
    use crate::mission_criteria::CriterionState;
    use crate::missions::AttentionReason;
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    finished_direct_node(&mut runtime);
    {
        let mission = runtime.missions.book.get_mut("mission-1").unwrap();
        let mut second = mission.nodes[0].clone();
        second.key = "second".into();
        mission.nodes.push(second);
        mission.nodes[0].done_head = None;
    }
    checkout_at(&mut runtime, OTHER_HEAD);
    let now = crate::runtime::unix_milliseconds();
    runtime.drive_mission_health(now);
    assert_eq!(
        node_view(&runtime, "fix").attention.unwrap().reason,
        AttentionReason::CloseRefused
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "second", "patch": {"status": "working"}}),
    )
    .unwrap();
    runtime.drive_mission_health(now + 1);
    assert!(node_view(&runtime, "fix").attention.is_none());
    command(
        &mut runtime,
        "recipient",
        record_of("second", "check", json!({"commit": OTHER_HEAD})),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "second", "patch": {"status": "done"}}),
    )
    .unwrap();
    runtime.drive_mission_health(now + 2);
    assert_eq!(mission_status(&runtime), MissionStatus::Running);
    assert_eq!(line_state(&runtime, "check"), CriterionState::Open);
    assert_eq!(
        node_view(&runtime, "fix")
            .attention
            .map(|raise| raise.reason),
        Some(AttentionReason::CloseRefused),
        "the close was refused again, so the choice to close is reachable again"
    );
    let raised = runtime.missions.events["mission-1"]
        .events
        .iter()
        .filter(|event| event.reason == Some(AttentionReason::CloseRefused))
        .filter_map(|event| event.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        raised.len(),
        2,
        "each question has its own entry: {raised:?}"
    );
}

/// with-hide/hide#2301, #2302: `show <mission id>` reads that mission, as
/// Hide's own letters tell a Conductor to run it; a node's worker and its
/// checker read their mission without naming it, and the worker sets its
/// own node's pull request, branch and line; anything else, and any other
/// node, stays the Conductor's.
#[test]
fn a_nodes_worker_and_checker_use_the_mission_commands_of_their_own_node() {
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
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"reviewer": "agent-3"}}),
    )
    .unwrap();

    for pane in ["sender", "recipient", "reviewer"] {
        let (whole, changed) = command(
            &mut runtime,
            pane,
            json!({"op": "show", "node": "mission-1"}),
        )
        .unwrap();
        assert!(!changed);
        assert_eq!(whole["mission"]["id"], "mission-1", "{pane}");
        assert!(whole.get("node").is_none(), "{pane}");
    }
    for pane in ["recipient", "reviewer"] {
        let (mission, _) = command(&mut runtime, pane, json!({"op": "show"})).unwrap();
        assert_eq!(mission["mission"]["id"], "mission-1", "{pane}");
        let (node, _) = command(&mut runtime, pane, json!({"op": "show", "node": "fix"})).unwrap();
        assert_eq!(node["node"]["key"], "fix", "{pane}");
    }

    let (attached, changed) = command(
        &mut runtime,
        "recipient",
        json!({"op": "node", "node": "fix", "patch": {"pr": 9, "branch": "fix/parser", "line": "PR #9 열림"}}),
    )
    .unwrap();
    assert!(changed);
    let fix = &attached["mission"]["nodes"][0];
    assert_eq!(fix["key"], "fix");
    assert_eq!(fix["prs"][0]["number"], 9);
    assert_eq!(fix["branch"], "fix/parser");
    assert_eq!(fix["line"], "PR #9 열림");
    for (pane, node, patch) in [
        ("recipient", "fix", json!({"status": "done"})),
        ("recipient", "fix", json!({"base_pr": 8})),
        ("recipient", "fix", json!({"reviewer": "agent-3"})),
        ("recipient", "docs", json!({"pr": 10})),
        ("reviewer", "fix", json!({"pr": 11})),
    ] {
        assert_eq!(
            command(
                &mut runtime,
                pane,
                json!({"op": "node", "node": node, "patch": patch})
            )
            .unwrap_err(),
            "mission_not_conductor",
            "{pane} {node} {patch}"
        );
    }
    let fix = runtime
        .missions
        .book
        .get("mission-1")
        .unwrap()
        .node("fix")
        .unwrap();
    assert_eq!(fix.prs, [9]);
    assert_eq!(fix.status, NodeStatus::Working);
}

/// A command that names no mission means the open mission the caller
/// conducts, else the one open mission whose node it works or checks: a
/// worker of two open missions is told to name one, and a Conductor whose
/// own mission ended reads the open mission it now works in.
#[test]
fn a_caller_in_two_open_missions_names_the_one_it_means() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    with_worker_and_reviewer(&mut runtime);
    for (conductor, worker) in [("sender", "agent-2"), ("reviewer", "agent-2")] {
        command(
            &mut runtime,
            conductor,
            json!({"op": "create", "title": "Fix the bug"}),
        )
        .unwrap();
        command(&mut runtime, conductor, criteria_plan()).unwrap();
        command(&mut runtime, conductor, json!({"op": "start"})).unwrap();
        command(
            &mut runtime,
            conductor,
            json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": worker}}),
        )
        .unwrap();
    }

    for unnamed in [
        json!({"op": "show"}),
        json!({"op": "show", "node": "fix"}),
        json!({"op": "node", "node": "fix", "patch": {"pr": 9}}),
        record_of("fix", "c1", json!({})),
    ] {
        assert_eq!(
            command(&mut runtime, "recipient", unnamed.clone()).unwrap_err(),
            "mission_ambiguous",
            "{unnamed}"
        );
    }
    let (named, _) = command(
        &mut runtime,
        "recipient",
        json!({"op": "node", "mission": "mission-2", "node": "fix", "patch": {"pr": 9}}),
    )
    .unwrap();
    assert_eq!(named["mission"]["id"], "mission-2");
    assert_eq!(named["mission"]["nodes"][0]["prs"][0]["number"], 9);
    let (shown, _) = command(
        &mut runtime,
        "recipient",
        json!({"op": "show", "mission": "mission-1"}),
    )
    .unwrap();
    assert_eq!(shown["mission"]["id"], "mission-1");
    let (lettered, _) = command(
        &mut runtime,
        "recipient",
        json!({"op": "show", "node": "mission-2"}),
    )
    .unwrap();
    assert_eq!(lettered["mission"]["id"], "mission-2");

    command(
        &mut runtime,
        "reviewer",
        json!({"op": "node", "node": "docs", "patch": {"status": "working", "agent_id": "agent-1"}}),
    )
    .unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "close", "end": "cancelled"}),
    )
    .unwrap();
    let (taken, _) = command(&mut runtime, "sender", json!({"op": "show"})).unwrap();
    assert_eq!(taken["mission"]["id"], "mission-2");
    let (docs, _) = command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "docs", "patch": {"line": "가이드 작성 중"}}),
    )
    .unwrap();
    assert_eq!(docs["mission"]["id"], "mission-2");
    let (worker, _) = command(&mut runtime, "recipient", json!({"op": "show"})).unwrap();
    assert_eq!(worker["mission"]["id"], "mission-2");
}

/// with-hide/hide#2300: a worker's watch reads it as busy while its node
/// waits on someone else, by the rule its quiet warning reads: a pull
/// request merged or waiting on CI, a review or the merge, or the
/// Conductor's question to the operator about the node; a failed CI is the
/// worker's to run again.
#[test]
fn a_mission_workers_watch_is_quiet_while_its_node_waits_on_the_operator() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    fixing_seven(&mut runtime);
    with_worker_and_reviewer(&mut runtime);
    command(
        &mut runtime,
        "sender",
        json!({"op": "node", "node": "fix", "patch": {"status": "working", "agent_id": "agent-2"}}),
    )
    .unwrap();
    let worker = runtime.delivery_observations["recipient"].actor.clone();
    let node = crate::node::TEST_NODE;
    let read = |runtime: &mut Runtime, answer: GithubSnapshot| {
        runtime.note_mission_pulls(node, &answer);
        runtime.github = answer;
    };
    read(
        &mut runtime,
        checked(7, "c3", PullRequestChecks::Pending, &[]),
    );
    assert!(runtime.mission_worker_waits(&worker), "CI runs");
    read(
        &mut runtime,
        checked(7, "c3", PullRequestChecks::Failed, &["verify"]),
    );
    assert!(
        !runtime.mission_worker_waits(&worker),
        "a failed CI is the worker's"
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": "첫 줄 캡처가 괜찮나요?", "why": "화면이 바뀜", "options": ["OK", "고침"], "node": "fix"}),
    )
    .unwrap();
    assert!(
        runtime.mission_worker_waits(&worker),
        "the operator is asked"
    );
    command(
        &mut runtime,
        "sender",
        json!({"op": "decide", "line": null}),
    )
    .unwrap();
    assert!(!runtime.mission_worker_waits(&worker));
    read(
        &mut runtime,
        github(vec![pull(7, "c3", PullRequestBadge::Merged, None)], false),
    );
    assert!(runtime.mission_worker_waits(&worker), "merged");
}
