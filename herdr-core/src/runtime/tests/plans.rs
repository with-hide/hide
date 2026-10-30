//! A session's own plan (PRD hide-plan): the `hide plan` commands from the
//! agent's pane, the review a proposed plan waits on, the save through the
//! link worker and the read back after a restart, the cap, and the plan
//! going with its session.

use super::*;
use crate::delivery::Actor;
use crate::handle::ChangeNotifier;
use crate::links::ledger::SessionIdentity;
use crate::model::AskSubject;
use crate::plans::{
    DelegationState, PLAN_LIMIT, Plan, PlanCommand, PlanTurn, Proofs, Review, Step, StepStatus,
};
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
    match turn(runtime, actor, command, &Proofs::default())? {
        PlanTurn::Answered { answer, changed } => Ok((answer, changed)),
        PlanTurn::Prove(proof) => panic!("the change asks for {:?}", proof.asks),
    }
}

/// One turn of a `hide plan` command, judged with `proofs`.
fn turn(
    runtime: &mut Runtime,
    actor: &Actor,
    command: Value,
    proofs: &Proofs,
) -> Result<PlanTurn, String> {
    let command: PlanCommand = serde_json::from_value(command).unwrap();
    runtime.plan_command(
        crate::node::TEST_NODE,
        &actor.pane_id,
        &authority(actor).context,
        command,
        proofs,
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
        ("question", None)
    );
    assert_eq!(
        asking.ask_subject,
        Some(AskSubject::PlanReview { steps: 3 })
    );

    command(&mut runtime, &sender, json!({"verb": "approve"})).unwrap();
    let approved = shown(&mut runtime);
    assert_eq!((approved.user_turn, approved.ask_subject), (None, None));
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

    // B14: the row asks the operator, by the question's own path, and
    // sends what it asks rather than words, so each surface says it in
    // its own language.
    let row = reviewed(&mut runtime);
    assert_eq!(row.demand, "question");
    let wire = serde_json::to_value(&row).unwrap();
    let subject = json!({"cause": "plan_review", "steps": 3});
    assert_eq!(
        (
            &wire["ask_subject"],
            &wire["state"]["ask"]["subject"],
            &wire["state"]["line"]["subject"],
            &wire["state"]["line"]["text"],
            &wire["detail"],
        ),
        (&subject, &subject, &subject, &json!(""), &Value::Null)
    );
    let band =
        crate::agent_state::header::of(&pane("session", "/work/app"), Some(&row), None, None)
            .band
            .unwrap();
    assert_eq!(
        (band.kind.as_str(), band.reason, band.subject),
        ("answer", None, Some(AskSubject::PlanReview { steps: 3 }))
    );
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
    let (answer, _) = command(&mut runtime, &sender, plan(false)).unwrap();
    let session: SessionIdentity = serde_json::from_value(answer["session"].clone()).unwrap();
    runtime
        .snapshot
        .ui_state
        .detail
        .open(crate::model::DetailItem::Plan { session });
    assert!(!runtime.drop_plans_of(&[(crate::node::TEST_NODE.into(), "other".into())]));
    assert!(runtime.drop_plans_of(&[(crate::node::TEST_NODE.into(), "sender-session".into())]));
    // Its map leaves the slot as ✕ leaves it, the slot empty and not drawn.
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default()
    );
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
                    delegate: false,
                    evidence: None,
                    delegation: None,
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
        .map(|step| step.key.as_str())
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

/// The fixture's sender registered as `agent-1` and its recipient as
/// `agent-2`, the sender's child.
fn registered(runtime: &mut Runtime) -> (Actor, Actor) {
    let sender = runtime.delivery_observations["sender"].actor.clone();
    let recipient = runtime.delivery_observations["recipient"].actor.clone();
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    for (id, actor, parent) in [
        ("agent-1", &sender, None),
        ("agent-2", &recipient, Some("agent-1")),
    ] {
        ledger.agents.push(crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(format!("{}-agent", actor.pane_id)),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "scope".into(),
            native_machine: "fixture-machine".into(),
            session: format!("{}-session", actor.pane_id),
            instance: format!("terminal-{}", actor.pane_id),
            pane: actor.pane_id.clone(),
            parent: parent.map(str::to_owned),
            origin: None,
            project: None,
            actor: actor.clone(),
            ended: false,
            terminal_id: None,
            forked: false,
        });
    }
    ledger.next_id = 10;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    (sender, recipient)
}

/// A plan whose `web` step waits on `spec` and is meant for another agent.
fn handing(spec: &str, ask: bool) -> Value {
    json!({"verb": "set", "ask": ask, "plan": {"nodes": [
        {"key": "spec", "title": "Spec", "depends_on": [], "status": spec},
        {"key": "web", "title": "Web", "depends_on": ["spec"], "status": "planned", "delegate": true},
        {"key": "ship", "title": "Ship", "depends_on": ["web"], "status": "planned", "output": "pr"},
    ]}})
}

fn spawn_step(step: &str, parent: Option<&str>) -> crate::delivery::Command {
    crate::delivery::Command::Agents {
        command: crate::coordination::Command::Spawn {
            parent: parent.map(str::to_owned),
            machine: None,
            name: "web".into(),
            intent: "plan".into(),
            kind: "codex".into(),
            repo: "/checkouts/fixture".into(),
            branch: "web".into(),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: Some("Build the web half.".into()),
            mission: None,
            plan_node: Some(step.into()),
            plan: None,
            beside: false,
        },
    }
}

/// The spawn `actor` asks for, up to where the core admits it; a spawn it
/// admits fails only for want of the store this runtime does not run.
fn prepare(runtime: &mut Runtime, actor: &Actor, step: &str, parent: Option<&str>) -> String {
    runtime
        .prepare_delivery(
            crate::node::TEST_NODE,
            &actor.pane_id,
            &authority(actor).context,
            None,
            spawn_step(step, parent),
        )
        .map(drop)
        .unwrap_err()
}

fn admit(runtime: &Runtime, actor: &Actor, step: &str) -> crate::coordination::PlanAdmission {
    runtime
        .plan_spawn_admission(
            actor,
            crate::node::TEST_NODE,
            Some("here"),
            Some(step),
            "codex",
            &Default::default(),
            false,
        )
        .unwrap()
        .unwrap()
}

/// The spawn the store keeps for `attempt` of the sender's step `step`,
/// with its child once bound.
fn linked(runtime: &mut Runtime, step: &str, attempt: u32, child: Option<&str>) {
    let plan = runtime.plans.book[&own_session()].id();
    let link = crate::coordination::PlanLink {
        plan,
        step: step.into(),
        attempt,
        parent_session: own_session(),
    };
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let mut record: crate::coordination::SpawnRecord = serde_json::from_value(json!({
        "id": format!("spawn-{step}-{attempt}"), "parent": "agent-1",
        "intent": crate::coordination::plan_intent(&link), "name": "web", "kind": "codex",
        "repo": "/checkouts/fixture", "branch": "web", "requested_path": null, "args": [],
        "path": null, "pane": null, "child": child,
    }))
    .unwrap();
    record.plan = Some(link);
    record.completed = child.is_some();
    ledger.spawns.push(record);
    runtime.publish_delivery(Arc::new(ledger), true);
}

/// The store's spawn `id` bound its child `agent`.
fn bind(runtime: &mut Runtime, id: &str, agent: &str) {
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let spawn = ledger
        .spawns
        .iter_mut()
        .find(|spawn| spawn.id == id)
        .unwrap();
    spawn.child = Some(agent.into());
    spawn.completed = true;
    runtime.publish_delivery(Arc::new(ledger), true);
}

/// `from`'s letter of `kind` to `to`, as the store keeps it.
fn letter(runtime: &mut Runtime, from: &Actor, to: &Actor, kind: &str) {
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let intent = format!("letter-{kind}-{}", ledger.next_id);
    crate::delivery::mailbox::send(
        &mut ledger,
        from,
        to,
        &intent,
        "About the web half",
        kind,
        None,
        unix_milliseconds(),
    )
    .unwrap();
    runtime.publish_delivery(Arc::new(ledger), true);
}

fn delegation(runtime: &Runtime, step: &str) -> Option<crate::plans::Delegation> {
    runtime.plans.book[&own_session()]
        .steps
        .iter()
        .find(|candidate| candidate.key == step)
        .and_then(|step| step.delegation.clone())
}

/// B1, B2, B4: a step is handed by its own plan's session to its own child,
/// only once the steps it waits on are finished and the plan is not
/// waiting for the operator's review, to one of the operator's candidates;
/// each refusal names the step or the reason before anything is made, and
/// the admitted spawn takes the intent the core made for its attempt.
#[test]
fn a_step_is_handed_to_a_child_only_after_its_earlier_steps_and_the_review() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    registered(&mut runtime);
    assert_eq!(
        prepare(&mut runtime, &sender, "web", Some("here")),
        "plan_missing"
    );
    command(&mut runtime, &sender, handing("working", false)).unwrap();
    assert_eq!(
        prepare(&mut runtime, &sender, "nope", Some("here")),
        crate::coordination::refusal("plan_node_unknown", "nope")
    );
    assert_eq!(
        prepare(&mut runtime, &sender, "web", None),
        "plan_delegate_needs_parent"
    );
    assert_eq!(
        prepare(&mut runtime, &sender, "web", Some("here")),
        crate::coordination::refusal("plan_order_refused", "web waits on spec")
    );
    // A plan put up for review hands nothing until it is approved.
    command(&mut runtime, &sender, handing("done", true)).unwrap();
    assert_eq!(
        prepare(&mut runtime, &sender, "web", Some("here")),
        "plan_review_pending"
    );
    command(&mut runtime, &sender, json!({"verb": "approve"})).unwrap();
    // Settings › Conductor's candidates hold a plan's children as a
    // mission's workers.
    runtime.snapshot.ui_state.conductor_settings.candidates =
        vec![serde_json::from_value(json!({"agent": "claude"})).unwrap()];
    assert_eq!(
        prepare(&mut runtime, &sender, "web", Some("here")),
        "plan_candidate_refused"
    );
    runtime
        .snapshot
        .ui_state
        .conductor_settings
        .candidates
        .clear();
    assert_eq!(
        prepare(&mut runtime, &sender, "web", Some("here")),
        "delivery_unavailable",
        "admitted, it would go to the store"
    );
    let admission = admit(&runtime, &sender, "web");
    let plan = runtime.plans.book[&own_session()].id();
    assert_eq!(
        crate::coordination::plan_intent(&admission.link),
        format!("plan:{plan}:web:1")
    );
    assert_eq!(admission.link.parent_session, own_session());
    // A finished step is handed to no one.
    assert_eq!(
        prepare(&mut runtime, &sender, "spec", Some("here")),
        crate::coordination::refusal("plan_node_finished", "spec")
    );
}

/// B7, D-23: a child a plan started, like a mission's worker, starts no
/// agent at all, and a Conductor's spawn is never a plan step's.
#[test]
fn a_plan_child_or_a_mission_worker_starts_no_agents() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    let (_, recipient) = registered(&mut runtime);
    command(&mut runtime, &sender, handing("done", false)).unwrap();
    // Before any step is handed to it, the recipient spawns as it always did.
    assert_eq!(
        runtime.plan_spawn_admission(
            &recipient,
            crate::node::TEST_NODE,
            Some("here"),
            None,
            "codex",
            &Default::default(),
            false,
        ),
        Ok(None)
    );
    linked(&mut runtime, "web", 1, Some("agent-2"));
    for step in [None, Some("web")] {
        assert_eq!(
            runtime.plan_spawn_admission(
                &recipient,
                crate::node::TEST_NODE,
                Some("here"),
                step,
                "codex",
                &Default::default(),
                false,
            ),
            Err("plan_delegate_not_permitted".into())
        );
    }
    // A Conductor hands no plan step: its spawn is its mission's worker.
    assert_eq!(
        runtime.plan_spawn_admission(
            &sender,
            crate::node::TEST_NODE,
            Some("here"),
            Some("web"),
            "codex",
            &Default::default(),
            true,
        ),
        Err("plan_delegate_not_permitted".into())
    );
}

/// B8, B13, B16, B17: a handed step follows its child's letters to its
/// main: a report is 보고 옴, a block 막힘, which the child's next progress
/// or the main's letter lifts; the child's pane closing for good without a
/// report is 닫힘. Neither moves the step's status. A blocked or closed step
/// is handed again under a new attempt, whose done counts no report from
/// before it.
#[test]
fn a_handed_step_follows_its_childs_letters_and_end_and_a_new_attempt_starts_clean() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    let (main, child) = registered(&mut runtime);
    command(&mut runtime, &sender, handing("done", false)).unwrap();
    linked(&mut runtime, "web", 1, None);
    let handed = delegation(&runtime, "web").unwrap();
    assert_eq!(
        (handed.attempt, handed.agent_id),
        (1, None),
        "still starting"
    );
    let step = |runtime: &Runtime| {
        runtime.plans.book[&own_session()]
            .steps
            .iter()
            .find(|step| step.key == "web")
            .unwrap()
            .status
    };
    assert_eq!(step(&runtime), StepStatus::Working);
    // The child bound: the same attempt names it.
    bind(&mut runtime, "spawn-web-1", "agent-2");
    assert_eq!(
        delegation(&runtime, "web").unwrap().agent_id.as_deref(),
        Some("agent-2")
    );
    // Same command again while it works: the same attempt.
    assert_eq!(admit(&runtime, &sender, "web").link.attempt, 1);

    letter(&mut runtime, &child, &main, "block");
    assert_eq!(
        delegation(&runtime, "web").unwrap().state,
        DelegationState::Blocked
    );
    letter(&mut runtime, &main, &child, "request");
    assert_eq!(
        delegation(&runtime, "web").unwrap().state,
        DelegationState::Working
    );
    letter(&mut runtime, &child, &main, "block");
    letter(&mut runtime, &child, &main, "progress");
    assert_eq!(
        delegation(&runtime, "web").unwrap().state,
        DelegationState::Working
    );
    letter(&mut runtime, &child, &main, "report");
    let reported = delegation(&runtime, "web").unwrap();
    assert_eq!(reported.state, DelegationState::Reported);
    assert!(reported.reported_at_unix_ms.is_some());
    assert_eq!(
        step(&runtime),
        StepStatus::Working,
        "a report finishes nothing"
    );
    // A letter folded once is not read again.
    runtime.fold_delegations();
    assert_eq!(delegation(&runtime, "web").unwrap(), reported);

    // A child that blocked is handed again under the next attempt, whose
    // done waits for its own report.
    letter(&mut runtime, &child, &main, "block");
    assert_eq!(admit(&runtime, &sender, "web").link.attempt, 2);
    linked(&mut runtime, "web", 2, None);
    let second = delegation(&runtime, "web").unwrap();
    assert_eq!(
        (second.attempt, second.state, second.reported_at_unix_ms),
        (2, DelegationState::Working, None)
    );
    assert_eq!(
        command(
            &mut runtime,
            &sender,
            json!({"verb": "node", "key": "web", "status": "done"})
        ),
        Err(crate::coordination::refusal("plan_report_missing", "web"))
    );

    // The first child closing for good does not touch the second attempt;
    // the second closing without a report is 닫힘.
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let mut second_child = ledger.agents[1].clone();
    second_child.id = "agent-3".into();
    ledger.agents.push(second_child);
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    bind(&mut runtime, "spawn-web-2", "agent-3");
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    ledger
        .agents
        .iter_mut()
        .find(|agent| agent.id == "agent-3")
        .unwrap()
        .ended = true;
    runtime.publish_delivery(Arc::new(ledger), true);
    assert_eq!(
        delegation(&runtime, "web").unwrap().state,
        DelegationState::Closed
    );
    assert_eq!(admit(&runtime, &sender, "web").link.attempt, 3);
}

/// B13, B17, B18: the main's done on a handed step, with its child's report,
/// asks the child to sleep without waiting for it; taking a blocked step
/// back makes it the main's own again and asks the same.
#[test]
fn a_handed_step_done_or_taken_back_asks_its_child_to_sleep() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    let (main, child) = registered(&mut runtime);
    command(&mut runtime, &sender, handing("done", false)).unwrap();
    linked(&mut runtime, "web", 1, Some("agent-2"));
    letter(&mut runtime, &child, &main, "report");
    let marks = |runtime: &Runtime| {
        runtime
            .missions
            .sleep_marks
            .iter()
            .map(|mark| mark.agent_id.clone())
            .collect::<Vec<_>>()
    };
    assert!(marks(&runtime).is_empty());
    let (done, changed) = command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "web", "status": "done"}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(done["plan"]["steps"][1]["delegation"]["name"], "web");
    assert_eq!(marks(&runtime), ["agent-2"]);

    // Another plan whose step the main takes back from its blocked child.
    runtime.missions.sleep_marks.clear();
    command(&mut runtime, &sender, json!({"verb": "drop"})).unwrap();
    command(&mut runtime, &sender, handing("done", false)).unwrap();
    linked(&mut runtime, "web", 1, Some("agent-2"));
    letter(&mut runtime, &child, &main, "block");
    let (mine, _) = command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "web", "status": "working"}),
    )
    .unwrap();
    assert!(mine["plan"]["steps"][1]["delegation"].is_null());
    assert_eq!(marks(&runtime), ["agent-2"]);
    // Done by the main itself needs no report now.
    command(
        &mut runtime,
        &sender,
        json!({"verb": "node", "key": "web", "status": "done"}),
    )
    .unwrap();
}

/// B20: the shown plan carries each handed step's child, its state, attempt,
/// name and kind, its pane and session, and the child's own plan rides
/// with it even though the child's pane is not shown.
#[test]
fn a_shown_plan_carries_its_child_and_the_childs_plan() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _worker) = start(root.path());
    let mut runtime = runtime.lock().unwrap();
    let (_, child) = registered(&mut runtime);
    command(&mut runtime, &sender, handing("done", false)).unwrap();
    command(&mut runtime, &child, plan(false)).unwrap();
    linked(&mut runtime, "web", 1, Some("agent-2"));
    runtime.snapshot.navigator.agents = vec![resting_row()];
    runtime.refresh_plans();
    let items = &runtime.snapshot.plans.items;
    assert_eq!(items.len(), 2, "{items:?}");
    let web = items[0]
        .steps
        .iter()
        .find(|step| step.key == "web")
        .unwrap();
    let delegation = web.delegation.as_ref().unwrap();
    assert_eq!(delegation.state, DelegationState::Working);
    assert_eq!(delegation.attempt, 1);
    assert_eq!(delegation.agent_id.as_deref(), Some("agent-2"));
    assert_eq!(
        (delegation.name.as_str(), delegation.agent.as_str()),
        ("web", "codex")
    );
    assert_eq!(delegation.pane_id, None, "its pane is not shown");
    let session = delegation.session.clone().unwrap();
    assert_eq!(session.session_id, "recipient-session");
    assert_eq!(items[1].session, session);
    let wire = serde_json::to_value(web).unwrap();
    assert_eq!(wire["delegate"], true);
    assert_eq!(wire["evidence"], Value::Null);
    assert_eq!(wire["delegation"]["state"], "working");
}
