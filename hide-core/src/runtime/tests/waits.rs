//! PRD core-wait through the runtime: a waiting node met, failed or past
//! its deadline by each target, the refusals a plan and `hide wait` meet,
//! an agent's own wait from asking to its notice, and a wait kept across a
//! restart.

use super::missions::{PROJECT, checked, command, conducting, github, node_view, pull};
use super::*;
use crate::missions::{AttentionReason, MissionStatus, NodeStatus, WakeEvent};
use crate::model::{PullRequestBadge, PullRequestChecks};
use crate::runtime::mission_outside::Found;
use crate::waits::{WaitOutcome, WaitTarget};
use serde_json::{Value, json};

const HOUR_MS: u64 = 60 * 60 * 1000;
/// Longer than the wait clock's period, so the next tick runs.
const TICK_MS: u64 = 60 * 1000;

/// The project's read coming back with `found` for every ask it holds: the
/// node `key` of mission-1 waiting on `target`, and the agents' waits.
fn read_back(runtime: &mut Runtime, found: crate::runtime::mission_outside::Found) {
    use crate::runtime::mission_outside::{Ask, Owner, Watch};
    let mut asks = Vec::new();
    if let Some(mission) = runtime.missions.book.get("mission-1") {
        for node in &mission.nodes {
            if let Some(target) = node.wait_target.clone() {
                asks.push(Ask {
                    owner: Owner::node("mission-1", &node.key),
                    watch: Watch::Outside {
                        target,
                        ahead: None,
                        reopens: runtime.missions.outside.reopens_of("mission-1"),
                        round: node.outside_round,
                    },
                });
            }
        }
    }
    for wait in runtime.snapshot.ui_state.waits.open() {
        asks.push(Ask {
            owner: Owner::Agent(wait.id.clone()),
            watch: Watch::Agent {
                target: wait.target.clone(),
                ahead: None,
            },
        });
    }
    let place = (crate::node::TEST_NODE.to_owned(), PROJECT.to_owned());
    let answers = asks.into_iter().map(|ask| (ask, found.clone())).collect();
    runtime.ingest_outside(&place, answers);
}

/// `sender` conducts mission-1, which waits on `wait` before `fix`.
fn waiting_on(runtime: &mut Runtime, wait: Value) {
    if runtime.missions.book.missions.is_empty() {
        command(
            runtime,
            "sender",
            json!({"op": "create", "title": "Fix the bug"}),
        )
        .unwrap();
    }
    command(
        runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "wait", "title": "Wait", "kind": "wait", "wait": wait},
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
             "criteria": ["it works"], "depends_on": ["wait"]},
        ]}}),
    )
    .unwrap();
    command(runtime, "sender", json!({"op": "start"})).unwrap();
}

/// `sender` creates mission-1 with one general node, so a second mission
/// can be copied from a whole record.
fn created(runtime: &mut Runtime) {
    command(
        runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    command(
        runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "first", "title": "First", "kind": "general", "criteria": ["it is done"]},
        ]}}),
    )
    .unwrap();
}

/// A second mission on this machine, `id`, with one node `n`, as another
/// Conductor's would be: copied from mission-1 so its record is whole.
fn other_mission(runtime: &mut Runtime, id: &str) {
    let mut other = runtime.missions.book.missions[0].clone();
    other.id = id.into();
    other.conductor.agent_id = Some("agent-other".into());
    other.nodes.truncate(1);
    other.nodes[0].key = "n".into();
    other.nodes[0].kind = crate::missions::NodeKind::General;
    other.nodes[0].wait_target = None;
    other.nodes[0].wait_until_unix_ms = None;
    other.nodes[0].status = NodeStatus::Working;
    other.status = MissionStatus::Running;
    runtime.missions.book.missions.push(other);
    runtime.save_missions();
}

fn set_other(runtime: &mut Runtime, id: &str, change: impl FnOnce(&mut crate::missions::Mission)) {
    change(runtime.missions.book.get_mut(id).unwrap());
    runtime.save_missions();
}

fn reason(runtime: &Runtime, key: &str) -> Option<AttentionReason> {
    node_view(runtime, key)
        .attention
        .map(|attention| attention.reason)
}

fn last_wake(runtime: &Runtime) -> Option<WakeEvent> {
    runtime
        .missions
        .book
        .get("mission-1")
        .unwrap()
        .wakes
        .last()
        .map(|wake| wake.event)
}

/// B6 (D-03): a mission target is met only by a `done` close; `merge_wait`
/// keeps it waiting and a cancel raises it to the Conductor. The progress
/// line follows the target mission at once.
#[test]
fn a_mission_target_is_met_by_done_kept_by_merge_wait_and_failed_by_cancel() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    // The target is made first so the plan names a mission this book holds.
    created(&mut runtime);
    other_mission(&mut runtime, "mission-7");
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "wait", "title": "Wait", "kind": "wait", "wait": {"mission": "mission-7"}},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working);
    assert_eq!(
        view.wait_progress
            .map(|progress| (progress.done, progress.total)),
        Some((0, 1))
    );

    set_other(&mut runtime, "mission-7", |mission| {
        mission.nodes[0].status = NodeStatus::Done;
        mission.status = MissionStatus::MergeWait;
    });
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working, "merge_wait keeps waiting");
    assert_eq!(
        view.wait_progress
            .map(|progress| (progress.done, progress.total)),
        Some((1, 1))
    );

    set_other(&mut runtime, "mission-7", |mission| {
        mission.status = MissionStatus::Cancelled;
    });
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working);
    assert_eq!(view.wait_outcome, Some("mission_cancelled"));
    assert_eq!(
        reason(&runtime, "wait"),
        Some(AttentionReason::MissionCancelled)
    );

    // Planned again on a mission that then closes done, it is met.
    other_mission(&mut runtime, "mission-8");
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "wait", "title": "Wait", "kind": "wait", "wait": {"mission": "mission-8"}},
        ]}}),
    )
    .unwrap();
    assert_eq!(node_view(&runtime, "wait").wait_outcome, None);
    runtime.drive_mission_health(crate::runtime::unix_milliseconds());
    assert_eq!(reason(&runtime, "wait"), None);
    set_other(&mut runtime, "mission-8", |mission| {
        mission.status = MissionStatus::Done;
    });
    assert_eq!(node_view(&runtime, "wait").status, NodeStatus::Done);
    assert_eq!(last_wake(&runtime), Some(WakeEvent::OutsideDone));
}

/// B7 (D-04): a node target is met when that node is done, a code node's
/// included before it reaches main, and raised when it is dropped.
#[test]
fn a_node_target_is_met_when_the_node_is_done_and_failed_when_it_is_dropped() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    created(&mut runtime);
    other_mission(&mut runtime, "mission-7");
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "a", "title": "A", "kind": "wait", "wait": {"mission": "mission-7", "node": "n"}},
        ]}}),
    )
    .unwrap();
    command(&mut runtime, "sender", json!({"op": "start"})).unwrap();
    assert_eq!(node_view(&runtime, "a").status, NodeStatus::Working);

    set_other(&mut runtime, "mission-7", |mission| {
        mission.nodes[0].kind = crate::missions::NodeKind::Change;
        mission.nodes[0].status = NodeStatus::Done;
    });
    assert_eq!(node_view(&runtime, "a").status, NodeStatus::Done);

    other_mission(&mut runtime, "mission-8");
    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "a", "title": "A", "kind": "wait", "wait": {"mission": "mission-7", "node": "n"}},
            {"key": "b", "title": "B", "kind": "wait", "wait": {"mission": "mission-8", "node": "n"}},
        ]}}),
    )
    .unwrap();
    set_other(&mut runtime, "mission-8", |mission| {
        mission.nodes[0].status = NodeStatus::Dropped;
    });
    assert_eq!(node_view(&runtime, "b").status, NodeStatus::Working);
    assert_eq!(node_view(&runtime, "b").wait_outcome, Some("node_dropped"));
    assert_eq!(reason(&runtime, "b"), Some(AttentionReason::NodeDropped));
}

/// B5 (D-05, D-06): a CI target waits while the newest commit's checks run,
/// is raised with the failed checks when one fails, waits again on a new
/// push, and is done when every check passes.
#[test]
fn a_ci_target_follows_the_checks_on_the_newest_commit() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on(&mut runtime, json!({"pr": 1232, "ci": true}));
    let node = crate::node::TEST_NODE;

    runtime.note_mission_pulls(node, &checked(1232, "a1", PullRequestChecks::Pending, &[]));
    assert_eq!(node_view(&runtime, "wait").status, NodeStatus::Working);
    assert_eq!(reason(&runtime, "wait"), None);

    runtime.note_mission_pulls(
        node,
        &checked(1232, "a1", PullRequestChecks::Failed, &["lint", "test"]),
    );
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working);
    assert_eq!(view.wait_outcome, Some("ci_failed"));
    assert!(
        view.line.as_deref().unwrap_or("").contains("lint"),
        "{:?}",
        view.line
    );
    assert_eq!(reason(&runtime, "wait"), Some(AttentionReason::CiFailed));

    runtime.note_mission_pulls(node, &checked(1232, "b2", PullRequestChecks::Pending, &[]));
    assert_eq!(
        node_view(&runtime, "wait").wait_outcome,
        None,
        "a new push waits again"
    );
    runtime.drive_mission_health(crate::runtime::unix_milliseconds());
    assert_eq!(reason(&runtime, "wait"), None);

    runtime.note_mission_pulls(node, &checked(1232, "b2", PullRequestChecks::Passing, &[]));
    assert_eq!(node_view(&runtime, "wait").status, NodeStatus::Done);
    assert_eq!(last_wake(&runtime), Some(WakeEvent::OutsideDone));
}

/// B4: a pull request closed without a merge keeps the node working and
/// raises it; the outcome says why.
#[test]
fn a_pull_request_closed_without_a_merge_is_raised_with_its_reason() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on(&mut runtime, json!({"pr": 1232}));
    runtime.note_mission_pulls(
        crate::node::TEST_NODE,
        &github(
            vec![pull(1232, "a1", PullRequestBadge::Closed, None)],
            false,
        ),
    );
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working);
    assert_eq!(view.wait_outcome, Some("closed_unmerged"));
    assert_eq!(reason(&runtime, "wait"), Some(AttentionReason::PrClosed));
}

/// B8, B11 (D-07): every waiting node has a deadline, 7 days unless `for`
/// says otherwise, between 1h and 30d; past it the node is raised and a
/// replan with a new deadline starts it over.
#[test]
fn a_waiting_node_past_its_deadline_is_raised_until_planned_again() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "create", "title": "Fix the bug"}),
    )
    .unwrap();
    for span in ["5m", "31d", "3x"] {
        let refused = command(
            &mut runtime,
            "sender",
            json!({"op": "plan", "plan": {"nodes": [
                {"key": "wait", "title": "Wait", "kind": "wait", "wait": {"branch": "feat/x", "for": span}},
            ]}}),
        )
        .unwrap_err();
        assert!(
            refused.contains("mission_wait_deadline_invalid"),
            "{span}: {refused}"
        );
    }
    waiting_on(&mut runtime, json!({"branch": "feat/x", "for": "2h"}));
    let until = node_view(&runtime, "wait").wait_until_unix_ms.unwrap();
    let now = crate::runtime::unix_milliseconds();
    assert!(
        until > now + HOUR_MS && until <= now + 2 * HOUR_MS + 1000,
        "{until}"
    );

    // The deadline asks one last read first, and fails the wait only once
    // that read came back with it still open.
    runtime.tick_waits(until + 1);
    assert_eq!(node_view(&runtime, "wait").wait_outcome, None);
    runtime.tick_waits(until + 1 + TICK_MS);
    assert_eq!(node_view(&runtime, "wait").wait_outcome, None);
    read_back(&mut runtime, Found::NoOwnWork { replaced: false });
    assert!(runtime.tick_waits(until + 1 + 2 * TICK_MS));
    let view = node_view(&runtime, "wait");
    assert_eq!(view.status, NodeStatus::Working);
    assert_eq!(view.wait_outcome, Some("expired"));
    assert_eq!(reason(&runtime, "wait"), Some(AttentionReason::WaitExpired));

    command(
        &mut runtime,
        "sender",
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "wait", "title": "Wait", "kind": "wait", "wait": {"branch": "feat/x", "for": "3d"}},
            {"key": "fix", "title": "Fix it", "kind": "change", "delivery": "pr",
             "criteria": ["it works"], "depends_on": ["wait"]},
        ]}}),
    )
    .unwrap();
    let view = node_view(&runtime, "wait");
    assert_eq!(view.wait_outcome, None);
    assert!(view.wait_until_unix_ms.unwrap() > until);
    runtime.drive_mission_health(crate::runtime::unix_milliseconds());
    assert_eq!(reason(&runtime, "wait"), None);
}

/// B10 (D-10, D-22): a plan may not wait on its own mission, on one that
/// waits on it, or on a mission or node this machine does not hold.
#[test]
fn a_plan_that_waits_on_itself_a_loop_or_nothing_is_refused() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    created(&mut runtime);
    other_mission(&mut runtime, "mission-7");
    let plan = |wait: Value| {
        json!({"op": "plan", "plan": {"nodes": [
            {"key": "wait", "title": "Wait", "kind": "wait", "wait": wait},
        ]}})
    };
    for (wait, code) in [
        (json!({"mission": "mission-1"}), "mission_wait_self"),
        (json!({"mission": "mission-99"}), "mission_wait_unknown"),
        (
            json!({"mission": "mission-7", "node": "nope"}),
            "mission_wait_unknown",
        ),
    ] {
        let refused = command(&mut runtime, "sender", plan(wait.clone())).unwrap_err();
        assert!(refused.contains(code), "{wait}: {refused}");
    }
    // mission-7 waits on mission-1, so mission-1 may not wait on mission-7.
    set_other(&mut runtime, "mission-7", |mission| {
        let node = &mut mission.nodes[0];
        node.kind = crate::missions::NodeKind::Wait;
        node.wait_target = Some(WaitTarget::Mission("mission-1".into()));
        node.wait_until_unix_ms = Some(u64::MAX);
    });
    let refused = command(
        &mut runtime,
        "sender",
        plan(json!({"mission": "mission-7"})),
    )
    .unwrap_err();
    assert!(refused.contains("mission_wait_self"), "{refused}");
}

/// B2, B19 (D-18, D-19): a book saved before waits reads its `external`
/// node as a waiting node, given seven days from the first read.
#[test]
fn an_old_books_outside_node_reads_as_a_waiting_node_with_seven_days() {
    let mission = {
        let fixture = conducting();
        let mut runtime = fixture.runtime.lock().unwrap();
        waiting_on(&mut runtime, json!({"pr": 1232}));
        runtime.missions.book.missions[0].clone()
    };
    let mut saved: Value =
        serde_json::from_slice(&crate::missions::encode_mission(&mission).unwrap()).unwrap();
    saved["schema_version"] = json!(4);
    let node = &mut saved["missions"][0]["nodes"][0];
    node["kind"] = json!("external");
    let fields = node.as_object_mut().unwrap();
    fields.remove("wait_until_unix_ms");
    let target = fields.remove("wait_target").unwrap();
    fields.insert("external".into(), target);
    let mut read = crate::missions::decode(&serde_json::to_vec(&saved).unwrap())
        .unwrap()
        .book;
    let node = &read.missions[0].nodes[0];
    assert_eq!(node.kind, crate::missions::NodeKind::Wait);
    assert_eq!(node.wait_target, Some(WaitTarget::Pr(1232)));
    assert_eq!(node.wait_until_unix_ms, None);
    let now = 1_000;
    assert!(crate::runtime::waits::settle_wait_deadlines(&mut read, now));
    assert_eq!(
        read.missions[0].nodes[0].wait_until_unix_ms,
        Some(now + 7 * 24 * HOUR_MS)
    );
}

/// The fixture's sender and recipient registered as `agent-1` and
/// `agent-2`: a wait is a registration's (D-22).
fn registered(runtime: &mut Runtime) {
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    for (id, pane) in [("agent-1", "sender"), ("agent-2", "recipient")] {
        let actor = runtime.delivery_observations[pane].actor.clone();
        ledger.agents.push(crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(format!("{pane}-agent")),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "scope".into(),
            native_machine: "fixture-machine".into(),
            session: format!("{pane}-session"),
            instance: format!("terminal-{pane}"),
            pane: pane.into(),
            parent: None,
            origin: None,
            project: None,
            actor,
            ended: false,
            terminal_id: None,
            forked: false,
            successor: None,
        });
    }
    ledger.next_id = 10;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
}

fn wait(runtime: &mut Runtime, pane: &str, wait: Value) -> Result<(Value, bool), String> {
    command(runtime, pane, json!({"op": "wait", "wait": wait}))
}

/// B14, B15, B19 (D-13, D-21): an agent's wait answers its id, target and
/// deadline; asked again it answers the same one; `list` and `cancel`
/// are the agent's own, and an agent holds at most eight.
#[test]
fn an_agents_wait_is_made_once_listed_cancelled_and_capped() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    let (first, changed) = wait(
        &mut runtime,
        "sender",
        json!({"branch": "feat/a", "for": "3d"}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(first["id"], "wait-1");
    assert_eq!(first["target"], json!({"branch": "feat/a"}));
    let (again, changed) = wait(&mut runtime, "sender", json!({"branch": "feat/a"})).unwrap();
    assert!(!changed);
    assert_eq!(
        (again["id"].clone(), again["existing"].clone()),
        (json!("wait-1"), json!(true))
    );

    let refused = command(
        &mut runtime,
        "recipient",
        json!({"op": "wait_cancel", "id": "wait-1"}),
    )
    .unwrap_err();
    assert!(refused.contains("wait_not_yours"), "{refused}");

    for index in 2..=8 {
        wait(
            &mut runtime,
            "sender",
            json!({"branch": format!("feat/{index}")}),
        )
        .unwrap();
    }
    let refused = wait(&mut runtime, "sender", json!({"branch": "feat/9"})).unwrap_err();
    assert!(refused.contains("wait_agent_full"), "{refused}");
    let (listed, _) = command(&mut runtime, "sender", json!({"op": "wait_list"})).unwrap();
    assert_eq!(listed["waits"].as_array().unwrap().len(), 8);

    command(
        &mut runtime,
        "sender",
        json!({"op": "wait_cancel", "id": "wait-1"}),
    )
    .unwrap();
    wait(&mut runtime, "sender", json!({"branch": "feat/9"})).unwrap();
    for (target, code) in [
        (json!({"pr": 1, "branch": "x"}), "wait_target_invalid"),
        (
            json!({"branch": "x", "for": "40d"}),
            "wait_deadline_invalid",
        ),
        (json!({"mission": "mission-99"}), "wait_mission_unknown"),
    ] {
        let refused = wait(&mut runtime, "recipient", target.clone()).unwrap_err();
        assert!(refused.contains(code), "{target}: {refused}");
    }
}

/// B9, B16, B17 (D-09): an agent waiting on a mission is marked to sleep,
/// and when the mission closes done the wait ends with one Hide notice
/// for that agent, which no longer holds a sleep mark for it.
#[test]
fn an_agents_wait_ends_with_one_notice_when_its_target_is_met() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    created(&mut runtime);
    other_mission(&mut runtime, "mission-7");
    wait(&mut runtime, "recipient", json!({"mission": "mission-7"})).unwrap();
    let agent = runtime.snapshot.ui_state.waits.waits[0].agent_id.clone();
    let marked = |runtime: &Runtime| {
        runtime.missions.sleep_marks.iter().any(|mark| {
            mark.agent_id == agent
                && matches!(
                    &mark.origin,
                    crate::runtime::agent_sleep::missions::Origin::Wait { .. }
                )
        })
    };
    assert!(marked(&runtime));
    let ledger = runtime.delivery_ledger.as_ref().ok().cloned();
    assert!(runtime.wait_cells(ledger.as_deref()).is_empty());

    set_other(&mut runtime, "mission-7", |mission| {
        mission.status = MissionStatus::Done
    });
    let ended = runtime.snapshot.ui_state.waits.waits[0]
        .ended
        .clone()
        .unwrap();
    assert_eq!(ended.outcome, WaitOutcome::MissionDone);
    assert!(!marked(&runtime));
    let cells = runtime.wait_cells(ledger.as_deref());
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].recipient, agent);
    assert!(cells[0].cell.news);
    assert!(
        cells[0].cell.line.contains("closed as done"),
        "{}",
        cells[0].cell.line
    );
    // Ended, it no longer counts: the agent may wait on the same again.
    let (open, _) = command(&mut runtime, "recipient", json!({"op": "wait_list"})).unwrap();
    assert_eq!(open["waits"], json!([]));
}

/// B26: an agent's wait is kept in the core's state file, and one whose
/// deadline passed while Hide was off ends as expired at the first tick.
#[test]
fn an_agents_wait_outlives_a_restart_and_expires_at_the_first_tick() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    let (made, _) = wait(
        &mut runtime,
        "sender",
        json!({"branch": "feat/a", "for": "1h"}),
    )
    .unwrap();
    let until = made["until_unix_ms"].as_u64().unwrap();
    let (stored, _, _) = crate::persistence::load(&runtime.state_path);
    assert_eq!(stored.waits, runtime.snapshot.ui_state.waits);
    assert!(stored.waits.open().any(|wait| wait.id == "wait-1"));

    // What a restart reads, then its first tick after the deadline.
    runtime.snapshot.ui_state.waits = Default::default();
    runtime.snapshot.ui_state.waits = stored.waits;
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(until + 1);
    assert!(runtime.snapshot.ui_state.waits.waits[0].ended.is_none());
    read_back(&mut runtime, Found::NoOwnWork { replaced: false });
    assert!(runtime.tick_waits(until + 1 + TICK_MS));
    let ended = runtime.snapshot.ui_state.waits.waits[0]
        .ended
        .clone()
        .unwrap();
    assert_eq!(ended.outcome, WaitOutcome::Expired);
    let (stored, _, _) = crate::persistence::load(&runtime.state_path);
    assert!(stored.waits.open().next().is_none());
}

/// O2 of the review (D-09): an agent holding two waits sleeps for the second
/// after the first ended, so the wait clock marks it again; one asleep is
/// left alone.
#[test]
fn an_agent_with_another_open_wait_is_marked_again_once_one_ends() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    created(&mut runtime);
    other_mission(&mut runtime, "mission-7");
    other_mission(&mut runtime, "mission-8");
    wait(&mut runtime, "recipient", json!({"mission": "mission-7"})).unwrap();
    wait(&mut runtime, "recipient", json!({"mission": "mission-8"})).unwrap();
    let agent = runtime.snapshot.ui_state.waits.waits[0].agent_id.clone();
    let marked_for = |runtime: &Runtime| {
        runtime
            .missions
            .sleep_marks
            .iter()
            .filter(|mark| mark.agent_id == agent)
            .find_map(|mark| match &mark.origin {
                crate::runtime::agent_sleep::missions::Origin::Wait { wait } => Some(wait.clone()),
                _ => None,
            })
    };
    assert_eq!(marked_for(&runtime).as_deref(), Some("wait-1"));

    set_other(&mut runtime, "mission-7", |mission| {
        mission.status = MissionStatus::Done
    });
    assert_eq!(marked_for(&runtime), None);
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(crate::runtime::unix_milliseconds());
    assert_eq!(marked_for(&runtime).as_deref(), Some("wait-2"));
}

/// O1 of the review (D-07): an agent's wait on a branch past its deadline
/// is read once more before it fails, so a target met in the last look's
/// gap is met, and no read back (one held at the host's rate floor) keeps
/// it open.
#[test]
fn a_code_host_wait_past_its_deadline_fails_only_after_one_more_read() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    let (made, _) = wait(
        &mut runtime,
        "sender",
        json!({"branch": "feat/a", "for": "1h"}),
    )
    .unwrap();
    let until = made["until_unix_ms"].as_u64().unwrap();
    assert!(runtime.snapshot.ui_state.waits.waits[0].project.is_some());

    let outcome = |runtime: &Runtime, at: usize| {
        runtime.snapshot.ui_state.waits.waits[at]
            .ended
            .as_ref()
            .map(|ended| ended.outcome.clone())
    };
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(until + 1);
    runtime.tick_waits(until + 1 + TICK_MS);
    assert_eq!(outcome(&runtime, 0), None, "no read came back yet");
    read_back(&mut runtime, Found::NoOwnWork { replaced: false });
    assert!(runtime.tick_waits(until + 1 + 2 * TICK_MS));
    assert_eq!(outcome(&runtime, 0), Some(WaitOutcome::Expired));

    // The read the deadline asks finds the branch in main: met, not expired.
    let (made, _) = wait(
        &mut runtime,
        "sender",
        json!({"branch": "feat/b", "for": "1h"}),
    )
    .unwrap();
    let until = made["until_unix_ms"].as_u64().unwrap();
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(until + 1);
    read_back(&mut runtime, Found::InMain(None));
    assert_eq!(outcome(&runtime, 1), Some(WaitOutcome::Merged));

    // A read that never comes back (a device away) holds the deadline for
    // half an hour, no longer.
    let (made, _) = wait(
        &mut runtime,
        "sender",
        json!({"branch": "feat/c", "for": "1h"}),
    )
    .unwrap();
    let until = made["until_unix_ms"].as_u64().unwrap();
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(until + 1);
    runtime.tick_waits(until + 29 * 60_000);
    assert_eq!(outcome(&runtime, 2), None);
    runtime.tick_waits(until + 30 * 60_000);
    assert_eq!(outcome(&runtime, 2), Some(WaitOutcome::Expired));
}

/// N1 of the review and with-hide/hide#2254 (D-05): a read that cannot say
/// how a CI wait's checks ended (they could not be read, or the host reads
/// none by number) is unread: a failure stands through it with its raise,
/// and the wait carries the unread mark instead of reading as open.
#[test]
fn a_ci_failure_stands_through_a_read_that_does_not_see_the_checks() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    waiting_on(&mut runtime, json!({"pr": 1232, "ci": true}));
    read_back(
        &mut runtime,
        Found::Ended(WaitOutcome::CiFailed {
            checks: vec!["lint".into()],
        }),
    );
    assert_eq!(node_view(&runtime, "wait").wait_outcome, Some("ci_failed"));
    read_back(
        &mut runtime,
        Found::Unread("The change request's checks could not be read".into()),
    );
    let view = node_view(&runtime, "wait");
    assert_eq!(view.wait_outcome, Some("ci_failed"));
    assert_eq!(reason(&runtime, "wait"), Some(AttentionReason::CiFailed));
    assert!(runtime.missions.outside.unread("mission-1", "wait"));
}

/// M2 of the review: a store whose waits all went is still kept, so the
/// next wait after a restart takes a new id and its notice a new revision,
/// which a seen record of the earlier one cannot hide.
#[test]
fn a_new_wait_after_a_restart_never_takes_an_ended_waits_id() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    wait(&mut runtime, "sender", json!({"branch": "feat/a"})).unwrap();
    command(
        &mut runtime,
        "sender",
        json!({"op": "wait_cancel", "id": "wait-1"}),
    )
    .unwrap();
    let (stored, _, _) = crate::persistence::load(&runtime.state_path);
    assert!(stored.waits.open().next().is_none());

    runtime.snapshot.ui_state.waits = stored.waits;
    let (made, _) = wait(&mut runtime, "sender", json!({"branch": "feat/b"})).unwrap();
    assert_eq!(made["id"], "wait-2");
}

/// R1 of the review (D-07, D-13): a host resting at its rate floor holds a
/// pull request wait's last read back, and its deadline with it, until half
/// an hour after the rest ends.
#[test]
fn a_rest_at_the_rate_floor_holds_a_pull_request_deadline_until_it_ends() {
    let fixture = conducting();
    let mut runtime = fixture.runtime.lock().unwrap();
    registered(&mut runtime);
    let (made, _) = wait(&mut runtime, "sender", json!({"pr": 1232, "for": "1h"})).unwrap();
    let until = made["until_unix_ms"].as_u64().unwrap();
    let key = runtime
        .github_place(None, PROJECT)
        .unwrap()
        .host()
        .unit()
        .key()
        .to_owned();
    let rest_ends = until + 2 * HOUR_MS;
    runtime.missions.hosts_resting.insert(key, rest_ends / 1000);
    let ended = |runtime: &Runtime| runtime.snapshot.ui_state.waits.waits[0].ended.is_some();
    runtime.missions.waits_next_tick_unix_ms = 0;
    runtime.tick_waits(until + 1);
    runtime.tick_waits(until + 31 * 60_000);
    assert!(!ended(&runtime), "the rest holds the deadline");
    // Once the rest ends the half hour still counts from its end.
    runtime.missions.hosts_resting.clear();
    runtime.tick_waits(rest_ends + 10 * 60_000);
    assert!(!ended(&runtime), "the half hour counts from the rest's end");
    runtime.tick_waits(rest_ends + 30 * 60_000);
    assert!(ended(&runtime));
}
