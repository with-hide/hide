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
    runtime.snapshot.desk.as_ref().unwrap().jobs[0].state
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
    assert_eq!(runtime.snapshot.desk.as_ref().unwrap().jobs.len(), 1);

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
