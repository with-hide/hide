//! A session's own plan (PRD hide-plan): the `hide plan` commands from the
//! agent's pane, the review a proposed plan waits on, the save through the
//! link worker and the read back after a restart, the cap, and the plan
//! going with its session.

use super::*;
use crate::delivery::Actor;
use crate::handle::ChangeNotifier;
use crate::links::ledger::SessionIdentity;
use crate::plans::{PLAN_LIMIT, Plan, PlanCommand, Review, Step, StepStatus};
use crate::runtime::delivery::tests::{authority, fixture};
use serde_json::{Value, json};

/// The fixture's `sender` pane, a Codex session `sender-session`, with the
/// link worker running on `root` and the saved plans read.
fn start(root: &std::path::Path) -> (Arc<Mutex<Runtime>>, Actor, crate::links::worker::LinkWorker) {
    let (runtime, sender, _, _) = fixture(root);
    runtime
        .lock()
        .unwrap()
        .install_worker_context(Arc::downgrade(&runtime), ChangeNotifier::noop());
    let worker = super::super::links::spawn_worker(&runtime, ChangeNotifier::noop()).unwrap();
    wait_for("the saved plans", || {
        command(
            &mut runtime.lock().unwrap(),
            &sender,
            json!({"verb": "show"}),
        ) != Err("plan_unavailable".into())
    });
    (runtime, sender, worker)
}

/// Waits until the store holds `count` plans: a worker that stops drops
/// the writes still queued, as it does every ledger write.
fn saved(worker: &crate::links::worker::LinkWorker, count: usize) {
    let client = worker.client();
    wait_for("the plan written", || {
        matches!(
            client.call(crate::links::ledger::LedgerCall::Plans),
            Ok(crate::links::ledger::LedgerAnswer::Plans(plans)) if plans.len() == count
        )
    });
}

fn command(runtime: &mut Runtime, actor: &Actor, command: Value) -> Result<(Value, bool), String> {
    let command: PlanCommand = serde_json::from_value(command).unwrap();
    runtime.plan_command(
        crate::node::TEST_NODE,
        &actor.pane_id,
        &authority(actor).context,
        command,
    )
}

fn plan(ask: bool) -> Value {
    json!({"verb": "set", "ask": ask, "plan": {"nodes": [
        {"key": "spec", "title": "Spec", "depends_on": [], "status": "done"},
        {"key": "core", "title": "Core events", "depends_on": ["spec"], "status": "working"},
        {"key": "pr", "title": "Open the PR", "depends_on": ["core"], "status": "planned"},
    ]}})
}

fn own_session() -> SessionIdentity {
    SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: "sender-session".into(),
    }
}

/// The sender's row at rest, as the sidebar shows it.
fn resting_row() -> crate::model::SidebarAgentSnapshot {
    let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents": [
        {"pane_id": "sender", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t-sender",
         "revision": 1, "focused": false, "agent_status": "idle", "agent": "codex",
         "name": "sender", "cwd": "/checkouts/fixture", "state_change_seq": 1},
    ]}))
    .unwrap();
    let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
    row.session_id = Some("sender-session".into());
    row
}

fn reviewed(runtime: &mut Runtime) -> crate::model::SidebarAgentSnapshot {
    let mut rows = vec![resting_row()];
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut rows);
    rows.remove(0)
}

/// The sender's row as the next published snapshot shows it.
fn shown(runtime: &mut Runtime) -> crate::model::SidebarAgentSnapshot {
    runtime.snapshot_delta_payload(0);
    runtime.snapshot.navigator.agents[0].clone()
}

#[test]
fn the_rows_already_shown_ask_and_stop_asking_as_the_review_moves_without_herdr_sending_them_again()
{
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    runtime.snapshot.navigator.agents = vec![resting_row()];
    let resting = shown(&mut runtime);

    command(&mut runtime, &sender, plan(true)).unwrap();
    let asking = shown(&mut runtime);
    assert_eq!(
        (asking.demand.as_str(), asking.detail.as_deref()),
        ("question", Some("계획 검토 · 3단계"))
    );

    command(&mut runtime, &sender, json!({"verb": "approve"})).unwrap();
    let approved = shown(&mut runtime);
    assert_eq!(approved.user_turn, None);
    assert_eq!(
        (approved.demand.as_str(), approved.group.as_str()),
        (resting.demand.as_str(), resting.group.as_str())
    );

    // A question the agent asked while its plan waited is its own, and
    // stays when the plan stops waiting.
    command(&mut runtime, &sender, plan(true)).unwrap();
    shown(&mut runtime);
    let own = hide_session::turns::UserTurnFact {
        kind: hide_session::turns::UserTurnKind::Question,
        content: Some(hide_session::turns::UserTurnContent::new("어느 쪽?", [])),
    };
    runtime.snapshot.navigator.agents[0].user_turn = Some(own.clone());
    command(&mut runtime, &sender, json!({"verb": "approve"})).unwrap();
    assert_eq!(shown(&mut runtime).user_turn, Some(own));
}

#[test]
fn a_plan_put_up_for_review_waits_on_the_operator_until_the_agent_approves_it() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();

    let (answer, changed) = command(&mut runtime, &sender, plan(true)).unwrap();
    assert!(changed);
    assert_eq!(answer["plan"]["review"], "proposed");
    assert_eq!(answer["context"]["plan"]["review"], "proposed");
    assert_eq!(answer["context"]["plan"]["current"]["key"], "core");
    assert_eq!(answer["context"]["plan"]["next"]["key"], "pr");
    assert_eq!(
        (
            &answer["context"]["plan"]["done"],
            &answer["context"]["plan"]["total"]
        ),
        (&json!(1), &json!(3))
    );

    // B14: the row asks the operator, by the question's own path.
    let row = reviewed(&mut runtime);
    assert_eq!(row.demand, "question");
    assert_eq!(row.detail.as_deref(), Some("계획 검토 · 3단계"));
    // A working agent asks nothing yet.
    let mut working = vec![resting_row()];
    working[0].activity = "working".into();
    runtime.lay_plan_reviews(crate::node::TEST_NODE, &mut working);
    assert!(working[0].user_turn.is_none());

    let (answer, changed) = command(&mut runtime, &sender, json!({"verb": "approve"})).unwrap();
    assert!(changed);
    assert_eq!(answer["context"]["plan"]["review"], "approved");
    assert!(reviewed(&mut runtime).user_turn.is_none());
    // Approving again changes nothing.
    assert!(
        !command(&mut runtime, &sender, json!({"verb": "approve"}))
            .unwrap()
            .1
    );
}

#[test]
fn a_step_is_done_only_after_what_it_waits_on_and_an_unknown_step_is_named() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    assert_eq!(
        command(
            &mut runtime,
            &sender,
            json!({"verb": "node", "key": "core", "status": "done"})
        ),
        Err("plan_missing".into())
    );
    command(&mut runtime, &sender, plan(false)).unwrap();

    let refused = command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "pr", "status": "done"}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&refused).0,
        "plan_order_refused"
    );
    let unknown = command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "docs", "status": "done"}),
    )
    .unwrap_err();
    assert_eq!(
        crate::coordination::split_refusal(&unknown).0,
        "plan_node_unknown"
    );

    let (answer, _) = command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "core", "status": "done", "line": "events land"}),
    )
    .unwrap();
    assert_eq!(answer["context"]["plan"]["current"]["key"], "pr");
    assert_eq!(answer["context"]["plan"]["done"], 2);
    // The same change again moves nothing.
    assert!(
        !command(
            &mut runtime,
            &sender,
            json!({"verb": "node", "key": "core", "status": "done", "line": "events land"})
        )
        .unwrap()
        .1
    );
}

#[test]
fn a_plan_is_read_back_after_a_restart_and_a_dropped_one_is_not() {
    let root = tempfile::tempdir().unwrap();
    {
        let (runtime, sender, worker) = start(root.path());
        command(&mut runtime.lock().unwrap(), &sender, plan(true)).unwrap();
        saved(&worker, 1);
        drop(worker);
    }
    let (runtime, sender, worker) = start(root.path());
    let (answer, _) = command(
        &mut runtime.lock().unwrap(),
        &sender,
        json!({"verb": "show"}),
    )
    .unwrap();
    assert_eq!(answer["plan"]["review"], "proposed");
    assert_eq!(answer["plan"]["steps"].as_array().unwrap().len(), 3);

    command(
        &mut runtime.lock().unwrap(),
        &sender,
        json!({"verb": "drop"}),
    )
    .unwrap();
    saved(&worker, 0);
    drop(worker);
    let (runtime, sender, _worker) = start(root.path());
    let (answer, _) = command(
        &mut runtime.lock().unwrap(),
        &sender,
        json!({"verb": "show"}),
    )
    .unwrap();
    assert!(answer["plan"].is_null());
}

#[test]
fn a_plan_goes_with_its_deleted_session() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    command(&mut runtime, &sender, plan(false)).unwrap();
    assert!(!runtime.drop_plans_of(&[(crate::node::TEST_NODE.into(), "other".into())]));
    assert!(runtime.drop_plans_of(&[(crate::node::TEST_NODE.into(), "sender-session".into())]));
    let (answer, _) = command(&mut runtime, &sender, json!({"verb": "show"})).unwrap();
    assert!(answer["plan"].is_null());
    assert!(answer["context"]["plan"].is_null());
}

#[test]
fn a_new_plan_past_the_cap_takes_the_place_of_the_one_updated_longest_ago() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    for at in 0..PLAN_LIMIT {
        let session = SessionIdentity {
            device_id: crate::node::TEST_NODE.into(),
            agent: "codex".into(),
            session_id: format!("old-{at}"),
        };
        runtime.plans.book.insert(
            session.clone(),
            Plan {
                session,
                steps: vec![Step {
                    key: "a".into(),
                    title: "A".into(),
                    depends_on: Vec::new(),
                    status: StepStatus::Planned,
                    line: None,
                    output: None,
                    changed: false,
                    change: None,
                }],
                review: Review::Approved,
                ask: None,
                created_at_unix_ms: 0,
                updated_at_unix_ms: 1_000 + at as u64,
            },
        );
    }
    let (result, records) =
        crate::diagnostics::capture(|| command(&mut runtime, &sender, plan(false)));
    result.unwrap();
    assert_eq!(runtime.plans.book.len(), PLAN_LIMIT);
    assert!(runtime.plans.book.contains_key(&own_session()));
    assert!(
        !runtime
            .plans
            .book
            .keys()
            .any(|session| session.session_id == "old-0")
    );
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "plans.evicted" && record["session_id"] == "old-0"),
        "{records:?}"
    );
}

#[test]
fn a_plan_is_drawn_for_the_pane_that_runs_its_session() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    command(&mut runtime, &sender, plan(false)).unwrap();
    runtime.refresh_plans();
    assert!(
        runtime.snapshot.plans.items.is_empty(),
        "no pane runs it yet"
    );
    runtime.snapshot.navigator.agents = vec![resting_row()];
    runtime.refresh_plans();
    let items = &runtime.snapshot.plans.items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].pane_id, "sender");
    // `pr` waits on `core`, which is still working (B7's dashed card).
    let waiting: Vec<_> = items[0]
        .steps
        .iter()
        .filter(|step| step.waiting)
        .map(|step| step.step.key.as_str())
        .collect();
    assert_eq!(waiting, ["pr"]);
}

#[test]
fn another_agents_plan_is_read_but_never_changed_and_an_unknown_agent_is_named() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    let recipient = Actor {
        pane_id: "recipient".into(),
        name: "recipient".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some("recipient-session".into()),
    };
    command(&mut runtime, &recipient, plan(true)).unwrap();
    // The recipient is a registered agent, named as `hide plan` names one.
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger.agents.push(crate::coordination::AgentRecord {
        id: "agent-1".into(),
        name: Some("recipient-agent".into()),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: "recipient-session".into(),
        instance: "terminal-recipient".into(),
        pane: "recipient".into(),
        parent: None,
        origin: None,
        project: None,
        actor: runtime.delivery_observations["recipient"].actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    });
    ledger.next_id = 2;
    runtime.delivery_ledger = Ok(Arc::new(ledger));

    let (shown, _) = command(
        &mut runtime,
        &sender,
        json!({"verb": "show", "agent": "recipient-agent"}),
    )
    .unwrap();
    assert_eq!(shown["plan"]["review"], "proposed");
    for verb in ["approve", "drop"] {
        assert_eq!(
            command(
                &mut runtime,
                &sender,
                json!({"verb": verb, "agent": "recipient-agent"})
            ),
            Err("plan_not_yours".into())
        );
        assert_eq!(
            command(
                &mut runtime,
                &sender,
                json!({"verb": verb, "agent": "nobody"})
            ),
            Err("plan_agent_unknown".into())
        );
    }
    let (after, _) = command(&mut runtime, &recipient, json!({"verb": "show"})).unwrap();
    assert_eq!(after["plan"], shown["plan"]);
}
