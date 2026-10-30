//! PRD mission-worker-sleep through the runtime: a letter to an agent Hide
//! put to sleep, the wake a `request` or `block` asks for, a finished
//! mission's agents put to sleep, and `hide agent sleep`. The Herdr workers
//! are not run (the live context's runtime is gone, so nothing they do comes
//! back); what they hand back is fed in through the calls a worker makes.

use super::*;
use crate::agent_sleep::{SleepPhase, SleepRecord, WakeMode};
use crate::agent_sleep_herdr::WakeOutcome;
use crate::coordination::AgentRecord;
use crate::delivery::Actor;
use crate::delivery::ledger::{Ledger, State};
use crate::workspace_control::Query;

const CHECKOUT: &str = "/private/tmp/hide-mission-worker-sleep";
const TABS: [&str; 2] = ["w-order:t1", "w-order:t2"];
const LEAD: &str = "w-order:t1:p";
const WORKER: &str = "w-order:t2:p";

/// The agent's conversation id in `pane`, in the shape Claude Code's are.
fn native(pane: &str) -> String {
    let digest = crate::wire::session_digest(pane).unwrap();
    format!(
        "{}-{}-{}-{}-{}",
        &digest[..8],
        &digest[8..12],
        &digest[12..16],
        &digest[16..20],
        &digest[20..32]
    )
}

fn actor(pane: &str) -> Actor {
    Actor {
        pane_id: pane.into(),
        name: format!("{pane}-agent"),
        kind: "claude".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: crate::wire::session_digest(&native(pane)),
    }
}

fn registration(id: &str, pane: &str, parent: Option<&str>) -> AgentRecord {
    AgentRecord {
        id: id.into(),
        name: Some(format!("{pane}-agent")),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: native(pane),
        instance: format!("terminal-{pane}"),
        pane: pane.into(),
        parent: parent.map(str::to_owned),
        origin: None,
        project: None,
        actor: actor(pane),
        ended: false,
        terminal_id: None,
        forked: false,
    }
}

fn record(pane: &str) -> SleepRecord {
    serde_json::from_value(serde_json::json!({
        "phase": "sleeping", "kind": "claude", "session_id": native(pane),
        "identity_label": pane, "last_activity": "0", "workspace_label": "Fixture",
        "since_unix_ms": 1_000, "cwd": CHECKOUT,
    }))
    .unwrap()
}

/// The two-tab session with `front` in front: the lead in t1, and, when
/// `worker` gives its status, the worker in t2.
fn session(front: &str, worker: Option<&str>) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(CHECKOUT, &TABS, &TABS, front);
    let mut agents = vec![serde_json::json!({
        "id": "lead", "pane_id": LEAD, "agent": "claude", "agent_status": "working",
        "state_change_seq": 1, "cwd": CHECKOUT,
        "agent_session": {"kind": "id", "value": native(LEAD)},
        "lineage_session": crate::wire::session_digest(&native(LEAD)),
    })];
    if let Some(status) = worker {
        agents.push(serde_json::json!({
            "id": "worker", "pane_id": WORKER, "agent": "claude", "agent_status": status,
            "state_change_seq": 4, "cwd": CHECKOUT,
            "agent_session": {"kind": "id", "value": native(WORKER)},
            "lineage_session": crate::wire::session_digest(&native(WORKER)),
            "tokens": {"task": "Fix the parser"}
        }));
    }
    let owned: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({ "agents": agents })).unwrap();
    payload.agents.extend(owned.agents);
    payload
}

/// A live runtime showing `session`, with Herdr's agents observed for
/// delivery and registrations `agent-1` (the lead) and `agent-2` (the
/// worker, the lead's child).
fn live(session: SessionSnapshotPayload) -> Runtime {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.snapshot.ui_state.agent_sleep_after_hours = Some(24);
    show(&mut runtime, session);
    let mut ledger = Ledger::default();
    ledger.agents.push(registration("agent-1", LEAD, None));
    ledger
        .agents
        .push(registration("agent-2", WORKER, Some("agent-1")));
    ledger.next_id = 3;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    runtime
}

/// Gives `runtime` the Workspace views a pane's command is checked against;
/// the folder lives as long as the returned guard.
fn views(runtime: Runtime) -> (Runtime, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    let runtime =
        super::workspace_view::with_views_only(runtime, &folder.path().join("views.json"));
    (runtime, folder)
}

/// The operator brings `tab` on screen.
fn visit(runtime: &mut Runtime, tab: &str) {
    let checkout = workspace::checkout_id_for_path("workspace:order", Path::new(CHECKOUT));
    runtime.dispatch_json(&focus_tab_event(&checkout, tab));
}

fn show(runtime: &mut Runtime, session: SessionSnapshotPayload) {
    runtime.ingest_session(Ok(session.clone()));
    runtime.observe_delivery(crate::node::TEST_NODE, &session, Some("scope"), None);
}

/// Registrations `agent-1` (the lead) and one per pane of `panes`, each
/// asleep in its pane.
fn sleepers(panes: &[&str]) -> Runtime {
    let mut runtime = live(session("w-order:t1", None));
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger.agents.retain(|record| record.id == "agent-1");
    for (index, pane) in panes.iter().enumerate() {
        ledger.agents.push(registration(
            &format!("agent-{}", index + 2),
            pane,
            Some("agent-1"),
        ));
        runtime
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .insert((*pane).into(), record(pane));
    }
    ledger.next_id = panes.len() as u64 + 2;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    runtime
}

/// The lead's letter of `kind` to the agent in `to`, as the store keeps it.
fn letter(runtime: &mut Runtime, to: &str, kind: &str) -> String {
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let intent = format!("{to}-{kind}-{}", ledger.next_id);
    let letter = crate::delivery::mailbox::send(
        &mut ledger,
        &actor(LEAD),
        &actor(to),
        &intent,
        "Fix the next bug",
        kind,
        None,
        unix_milliseconds(),
    )
    .unwrap();
    runtime.publish_delivery(Arc::new(ledger), true);
    letter.id
}

fn phase(runtime: &Runtime, pane: &str) -> Option<SleepPhase> {
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .get(pane)
        .map(|record| record.phase)
}

/// B1: a letter to a registered agent asleep in its pane is taken by the
/// store, pending, by its registration id and by the pane it sleeps in,
/// where it was refused `target_unavailable` before.
#[test]
fn a_letter_to_a_sleeping_agent_is_taken_and_waits() {
    let root = tempfile::tempdir().unwrap();
    let (mut runtime, _views) = views(sleepers(&[WORKER]));
    show(&mut runtime, session("w-order:t1", None));
    assert!(!runtime.delivery_observations.contains_key(WORKER));
    let context = runtime
        .workspace_control_query(crate::node::TEST_NODE, LEAD, Query::Info)
        .unwrap()
        .context;
    let runtime = Arc::new(Mutex::new(runtime));
    let (_worker, client) = crate::delivery::worker::Worker::store(
        Arc::downgrade(&runtime),
        crate::handle::ChangeNotifier::noop(),
        root.path().join("ledger.json"),
    )
    .unwrap();
    runtime.lock().unwrap().install_delivery_client(client);
    for (target, intent) in [("agent-2", "by-id"), (WORKER, "by-pane")] {
        let prepared = runtime
            .lock()
            .unwrap()
            .prepare_delivery(
                crate::node::TEST_NODE,
                LEAD,
                &context,
                None,
                crate::delivery::Command::Send {
                    target: target.into(),
                    intent: intent.into(),
                    body: "Fix the next bug".into(),
                    kind: "report".into(),
                    now: false,
                },
            )
            .unwrap();
        let sent = prepared.run(std::time::Duration::from_secs(10)).unwrap();
        assert_eq!(sent["state"], "pending");
        assert_eq!(sent["recipient"]["pane_id"], WORKER);
    }
    // A report wakes no one (B3).
    let mut guard = runtime.lock().unwrap();
    guard.drive_letter_wakes();
    assert_eq!(phase(&guard, WORKER), Some(SleepPhase::Sleeping));
}

/// B2, B3: a `report` or `progress` letter leaves its recipient asleep; a
/// `request` wakes it with its conversation resumed, and more letters to it
/// while it wakes start no second wake.
#[test]
fn a_request_wakes_a_sleeping_agent_and_a_report_waits() {
    let mut runtime = sleepers(&[WORKER]);
    letter(&mut runtime, WORKER, "report");
    letter(&mut runtime, WORKER, "progress");
    runtime.drive_letter_wakes();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Sleeping));
    letter(&mut runtime, WORKER, "request");
    assert!(runtime.drive_letter_wakes());
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Waking));
    letter(&mut runtime, WORKER, "block");
    assert!(!runtime.drive_letter_wakes());
    assert_eq!(runtime.letter_wakes_running(), 1);
}

/// B4: letters to five sleeping agents wake four at once, in the order the
/// letters came; the fifth wakes when one of the four is awake.
#[test]
fn letter_wakes_run_four_at_a_time_in_the_order_the_letters_came() {
    let panes = ["w9:p1", "w9:p2", "w9:p3", "w9:p4", "w9:p5"];
    let mut runtime = sleepers(&panes);
    for pane in panes {
        letter(&mut runtime, pane, "request");
    }
    runtime.drive_letter_wakes();
    let waking = |runtime: &Runtime| {
        panes
            .iter()
            .filter(|pane| phase(runtime, pane) == Some(SleepPhase::Waking))
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(waking(&runtime), panes[..4]);
    // The first is awake: its record goes.
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .remove(panes[0]);
    runtime.letter_wakes.touch();
    runtime.drive_letter_wakes();
    assert_eq!(waking(&runtime), panes[1..]);
}

/// B6: a wake that fails leaves the `request` and `block` letters waiting on
/// it to the store to fail, with the wake's reason, and leaves a `report`
/// waiting; the next `request` tries the wake again.
#[test]
fn a_failed_letter_wake_fails_its_requests_and_the_next_one_tries_again() {
    let mut runtime = sleepers(&[WORKER]);
    let report = letter(&mut runtime, WORKER, "report");
    let request = letter(&mut runtime, WORKER, "request");
    runtime.drive_letter_wakes();
    runtime.ingest_agent_wake(
        WORKER,
        WakeMode::Resume,
        WakeOutcome::Failed {
            reason: crate::agent_sleep_herdr::WAKE_FOLDER_MISSING.into(),
            detail: "fixture".into(),
        },
    );
    runtime.drive_letter_wakes();
    let failed = runtime.delivery_wake_failures();
    assert_eq!(
        failed.get(&request).map(String::as_str),
        Some(crate::agent_sleep_herdr::WAKE_FOLDER_MISSING)
    );
    assert!(!failed.contains_key(&report));
    // The store's pass applies it.
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    assert!(ledger.fail_woken(&failed, unix_milliseconds()));
    let undelivered = ledger
        .letters
        .iter()
        .find(|letter| letter.id == request)
        .unwrap();
    assert_eq!(undelivered.state, State::Undelivered);
    assert!(!undelivered.waiting_answer);
    runtime.publish_delivery(Arc::new(ledger), true);
    runtime.drive_letter_wakes();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Failed));
    letter(&mut runtime, WORKER, "request");
    runtime.drive_letter_wakes();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Waking));
}

/// B7: a restart reads a wake that was running as asleep again, and the
/// request still pending wakes it again.
#[test]
fn a_restart_queues_the_wake_a_pending_request_still_needs() {
    let mut runtime = sleepers(&[WORKER]);
    letter(&mut runtime, WORKER, "request");
    runtime.drive_letter_wakes();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Waking));
    // What a restart reads back.
    runtime.snapshot.ui_state.agent_sleep.after_load();
    runtime.letter_wakes = Default::default();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Sleeping));
    let ledger = runtime.delivery_state().unwrap();
    runtime.publish_delivery(ledger, false);
    runtime.drive_letter_wakes();
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Waking));
}

/// D-13: the store's pass reads the asleep recipients with the session the
/// letters to them address.
#[test]
fn the_store_reads_a_sleeper_with_its_session() {
    let runtime = sleepers(&[WORKER]);
    let asleep = runtime.delivery_asleep();
    let sleeper = &asleep[WORKER];
    assert_eq!(sleeper.session, actor(WORKER).session);
    assert_eq!(sleeper.device_id, crate::node::TEST_NODE);
    assert_eq!(sleeper.since, 1_000);
}

fn mission(status: &str) -> crate::missions::Mission {
    serde_json::from_value(serde_json::json!({
        "id": "mission-1", "title": "Fix the bug", "project": CHECKOUT,
        "device_id": crate::node::TEST_NODE,
        "conductor": {"actor": actor(LEAD), "agent_id": "agent-1"},
        "status": "running", "scope": "pr", "cap": 4,
        "created_at_unix_ms": 1, "updated_at_unix_ms": 1,
        "nodes": [{
            "key": "fix", "title": "Fix it", "kind": "change", "status": status,
            "agent_id": "agent-2",
            "criteria": [{"key": "review", "kind": "review", "checkers": [
                {"worker": {"agent": "codex"}, "agent_id": "agent-3"},
                {"worker": {"agent": "claude"}, "agent_id": "agent-1"}
            ]}]
        }]
    }))
    .unwrap()
}

/// A mission whose node `fix` the worker in t2 works, with t1 in front.
fn mission_runtime() -> Runtime {
    let mut runtime = live(session("w-order:t1", Some("idle")));
    runtime.missions.book.missions.push(mission("working"));
    runtime.save_missions();
    runtime
}

fn set_node(runtime: &mut Runtime, status: crate::missions::NodeStatus) {
    runtime.missions.book.missions[0].nodes[0].status = status;
    runtime.save_missions();
}

fn marks(runtime: &Runtime) -> Vec<String> {
    runtime
        .missions
        .sleep_marks
        .iter()
        .map(|mark| mark.agent_id.clone())
        .collect()
}

/// B8, B9, B12: a node that finishes marks its worker and its checkers,
/// never the Conductor; the next minute ends the worker that rests off
/// screen, and the mark is spent only when the end lands, so a wake after
/// it does not put the worker to sleep again.
#[test]
fn a_finished_nodes_worker_sleeps_once() {
    let mut runtime = mission_runtime();
    assert!(marks(&runtime).is_empty());
    set_node(&mut runtime, crate::missions::NodeStatus::Done);
    assert_eq!(marks(&runtime), ["agent-2", "agent-3"]);
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Ending));
    // The checker has no pane: it can never sleep here.
    assert_eq!(marks(&runtime), ["agent-2"]);
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(marks(&runtime), ["agent-2"], "its end still runs");
    runtime.ingest_agent_sleep_end(WORKER, Ok(4));
    assert!(marks(&runtime).is_empty());
    // It wakes, and the same finish asks nothing again.
    runtime.snapshot.ui_state.agent_sleep.records.remove(WORKER);
    runtime.save_missions();
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), None);
}

/// D-19: an end refused because the agent took a turn keeps the mark, and
/// the next minute tries again despite the hour a refused end waits.
#[test]
fn a_refused_mission_sleep_is_tried_again_the_next_minute() {
    let mut runtime = mission_runtime();
    set_node(&mut runtime, crate::missions::NodeStatus::Failed);
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    runtime.ingest_agent_sleep_end(WORKER, Err("busy".into()));
    assert_eq!(phase(&runtime, WORKER), None);
    assert!(marks(&runtime).contains(&"agent-2".to_owned()));
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), Some(SleepPhase::Ending));
}

/// B10: a worker that works or is on the operator's screen keeps its mark,
/// and a node opened again before it rests drops it.
#[test]
fn a_busy_or_shown_worker_waits_and_a_reopened_node_drops_its_marks() {
    let mut runtime = live(session("w-order:t1", Some("working")));
    runtime.missions.book.missions.push(mission("working"));
    runtime.save_missions();
    set_node(&mut runtime, crate::missions::NodeStatus::Done);
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), None);
    assert!(marks(&runtime).contains(&"agent-2".to_owned()));
    show(&mut runtime, session("w-order:t1", Some("idle")));
    visit(&mut runtime, "w-order:t2");
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), None, "on the operator's screen");
    assert!(marks(&runtime).contains(&"agent-2".to_owned()));
    set_node(&mut runtime, crate::missions::NodeStatus::Working);
    assert!(marks(&runtime).is_empty());
}

/// B9: a merged pull request finishes its node for its agents, and so does
/// the mission's close.
#[test]
fn a_merged_pull_request_or_a_closed_mission_finishes_the_node() {
    let mut runtime = mission_runtime();
    runtime.missions.book.missions[0].nodes[0].pr_seen = Some(crate::missions::PrSeen {
        merged: true,
        ..Default::default()
    });
    runtime.save_missions();
    assert_eq!(marks(&runtime), ["agent-2", "agent-3"]);
    let mut runtime = mission_runtime();
    runtime.missions.book.missions[0].status = crate::missions::MissionStatus::Cancelled;
    runtime.save_missions();
    assert_eq!(marks(&runtime), ["agent-2", "agent-3"]);
}

/// B11: with sleep set to Off, a finish puts nothing to sleep.
#[test]
fn a_finish_sleeps_nothing_while_sleep_is_off() {
    let mut runtime = mission_runtime();
    runtime.snapshot.ui_state.agent_sleep_after_hours = None;
    set_node(&mut runtime, crate::missions::NodeStatus::Done);
    runtime.sleep_finished_mission_agents(unix_milliseconds());
    assert_eq!(phase(&runtime, WORKER), None);
    assert!(marks(&runtime).is_empty());
}

fn sleep_command(runtime: &mut Runtime, id: &str) -> Result<serde_json::Value, String> {
    let context = runtime
        .workspace_control_query(crate::node::TEST_NODE, LEAD, Query::Info)
        .unwrap()
        .context;
    runtime
        .agent_sleep_command(crate::node::TEST_NODE, LEAD, &context, id, None)
        .map(|(answer, _)| answer)
}

/// B14: the parent puts its resting child to sleep and asking again
/// answers the same; B15: a refusal says why and changes nothing.
#[test]
fn hide_agent_sleep_answers_its_state_and_refuses_with_a_reason() {
    let (mut runtime, _views) = views(live(session("w-order:t1", Some("working"))));
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap_err(),
        "agent_sleep_working"
    );
    show(&mut runtime, session("w-order:t1", Some("idle")));
    visit(&mut runtime, "w-order:t2");
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap_err(),
        "agent_sleep_on_screen"
    );
    assert_eq!(phase(&runtime, WORKER), None);
    visit(&mut runtime, "w-order:t1");
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap()["state"],
        "ending"
    );
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap()["state"],
        "ending"
    );
    runtime.ingest_agent_sleep_end(WORKER, Ok(4));
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap()["state"],
        "asleep"
    );
    assert_eq!(
        sleep_command(&mut runtime, "agent-9").unwrap_err(),
        "agent_unavailable"
    );
}

/// D-20: only the agent itself or its parent may put it to sleep.
#[test]
fn hide_agent_sleep_takes_only_the_agent_or_its_parent() {
    let (mut runtime, _views) = views(live(session("w-order:t1", Some("idle"))));
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    // The worker's parent is someone else.
    ledger.agents[1].parent = Some("agent-7".into());
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(
        sleep_command(&mut runtime, "agent-2").unwrap_err(),
        "parent_authority_required"
    );
    assert_eq!(phase(&runtime, WORKER), None);
}
