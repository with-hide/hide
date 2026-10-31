use super::workspace_view::with_new_views;
use super::*;
use crate::coordination::started::tests::{Child, register, spawn};
use crate::delivery::ledger::Ledger;
use crate::delivery::{Actor, HIDE_DESK_ROLE};
use crate::model::{AgentGroupSnapshot, DeskJobState};
use serde_json::{Value, json};

// Expected outcomes come from PRD Desk 1차 B20, B37 and B38: a session Desk
// started goes into its checkout's `◆ Desk` group once, without moving the
// group in front; the operator's move out of the group and close are kept.
const PATH: &str = "/desk-group";
const TABS: [&str; 3] = ["w-order:t1", "w-order:t2", "w-order:t3"];

/// A runtime running Desk with one checkout of three tabs, in which Desk's
/// spawn's session is the pane of the second tab.
fn setup() -> Runtime {
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "desk-group");
    runtime.ingest_session(Ok(tab_order_payload(PATH, &TABS, &TABS, TABS[0])));
    runtime.sync_workspace_view();
    let mut ledger = Ledger::default();
    let desk = Actor::hide(HIDE_DESK_ROLE, crate::node::TEST_NODE);
    let origin = register(&mut ledger, &desk);
    let pane = format!("{}:p", TABS[1]);
    spawn(
        &mut ledger,
        &desk,
        &origin,
        false,
        None,
        "fix",
        Child {
            machine: crate::node::TEST_NODE,
            pane: &pane,
            session: "session",
        },
    )
    .unwrap();
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    runtime.start_test_desk();
    runtime
}

fn groups(runtime: &Runtime) -> (Option<String>, Vec<AgentGroupSnapshot>) {
    let layout = runtime
        .snapshot()
        .workspace_view
        .as_ref()
        .unwrap()
        .agent_layout
        .as_ref()
        .unwrap();
    (layout.group_id.clone(), layout.groups.clone())
}

fn job_state(runtime: &Runtime) -> DeskJobState {
    runtime.snapshot.desk_jobs.as_ref().unwrap().jobs[0].state
}

fn move_tab(runtime: &mut Runtime, group: Option<&str>) {
    let mut payload: Value = json!({"action": "move", "tab_id": TABS[1], "request_id": "move"});
    if let Some(group) = group {
        payload["group_id"] = json!(group);
    }
    payload["workspace"] = json!({"device_id": crate::node::TEST_NODE, "path": PATH});
    runtime.dispatch_json(&explorer_event("agent_group", payload));
    runtime.sync_workspace_view();
}

#[test]
fn a_session_desk_started_joins_the_desk_group_once_and_the_screen_stays() {
    let mut runtime = setup();
    let (selected, _) = groups(&runtime);
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();

    let (after, groups_now) = groups(&runtime);
    assert_eq!(after, selected, "the group in front does not move");
    let desk = groups_now
        .iter()
        .find(|group| group.name.as_deref() == Some("◆ Desk"))
        .expect("the Desk group was made");
    assert_eq!(desk.tabs, [TABS[1]]);
    assert_eq!(job_state(&runtime), DeskJobState::Working);

    // A second publish places nothing again.
    runtime.refresh_desk_jobs();
    let (_, again) = groups(&runtime);
    assert_eq!(
        again
            .iter()
            .filter(|group| group.name.as_deref() == Some("◆ Desk"))
            .count(),
        1
    );
}

#[test]
fn a_tab_moved_out_of_the_desk_group_reads_moved_out_and_stays_out() {
    let mut runtime = setup();
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    move_tab(&mut runtime, None);
    runtime.refresh_desk_jobs();
    assert_eq!(job_state(&runtime), DeskJobState::MovedOut);
    assert_eq!(runtime.snapshot.desk_jobs.as_ref().unwrap().jobs.len(), 1);

    // Desk does not put it back.
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    let (_, groups_now) = groups(&runtime);
    assert!(
        groups_now
            .iter()
            .filter(|group| group.name.as_deref() == Some("◆ Desk"))
            .all(|group| !group.tabs.iter().any(|tab| tab == TABS[1]))
    );
}

/// A start of Desk's that failed after its tab opened (its brief not taken
/// in time) leaves its agent in that tab: the tab is Desk's like any other,
/// and the card follows the pane while its agent works, reading failed while
/// it rests, where `다시 시도` hands the brief over in the same pane.
#[test]
fn a_start_that_failed_after_its_tab_opened_joins_the_group_and_its_card_follows_the_pane() {
    use crate::coordination::{Command, Mutation, apply};
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "desk-group-failed-start");
    runtime.ingest_session(Ok(tab_order_payload(PATH, &TABS, &TABS, TABS[0])));
    runtime.sync_workspace_view();
    let pane = format!("{}:p", TABS[1]);
    let desk = Actor::hide(HIDE_DESK_ROLE, crate::node::TEST_NODE);
    let command = Command::Spawn {
        mission: None,
        plan_node: None,
        plan: None,
        parent: None,
        machine: None,
        name: "child-fix".into(),
        intent: "intent-fix".into(),
        kind: "claude".into(),
        repo: "/repo".into(),
        branch: "fix".into(),
        path: None,
        args: Vec::new(),
        launch: Default::default(),
        brief: Some("fix it".into()),
        task: None,
        beside: false,
    };
    // The ledger as the failed start left it: its pane made and its agent
    // started, no child bound and the spawn not completed.
    let mut ledger = Ledger::default();
    let origin = register(&mut ledger, &desk);
    let reserved = apply(
        &mut ledger,
        &desk,
        &Mutation::Reserve {
            parent: origin,
            command: command.clone(),
        },
        1,
    )
    .unwrap();
    apply(
        &mut ledger,
        &desk,
        &Mutation::Advance {
            seed_tab: None,
            id: reserved["id"].as_str().unwrap().into(),
            path: Some(PATH.into()),
            pane: Some(pane.clone()),
            terminal: None,
            shell: None,
            child: None,
        },
        1,
    )
    .unwrap();
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    runtime.start_test_desk();
    runtime.fail_test_desk_start(command, "brief_not_submitted");
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();

    let (_, groups_now) = groups(&runtime);
    let group = groups_now
        .iter()
        .find(|group| group.name.as_deref() == Some("◆ Desk"))
        .expect("the Desk group was made");
    assert_eq!(group.tabs, [TABS[1]]);
    let card = |runtime: &Runtime| runtime.snapshot.desk_jobs.as_ref().unwrap().jobs[0].clone();
    assert_eq!(card(&runtime).state, DeskJobState::Failed);
    assert_eq!(card(&runtime).pane_id.as_deref(), Some(pane.as_str()));

    let payload: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(json!({"agents": [
            {"pane_id": pane, "workspace_id": "w-order", "tab_id": TABS[1], "terminal_id": "t2",
             "revision": 1, "focused": false, "agent_status": "working", "agent": "claude",
             "name": "child-fix", "cwd": PATH, "state_change_seq": 1},
        ]}))
        .unwrap();
    runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
    runtime.refresh_desk_jobs();
    assert_eq!(card(&runtime).state, DeskJobState::Working);
    assert_eq!(card(&runtime).failure, None);
}
