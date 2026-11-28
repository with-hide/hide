use super::workspace_view::with_new_views;
use super::*;
use crate::agent_groups::GroupRole;
use crate::coordination::started::tests::{Child, register, spawn};
use crate::delivery::ledger::Ledger;
use crate::delivery::{Actor, HIDE_DESK_ROLE};
use crate::model::{AgentGroupSnapshot, DeskJobActivity, DeskJobState};
use serde_json::{Value, json};

// Expected outcomes come from PRD Desk 1차 B20, B37 and B38: a session Desk
// started goes into its checkout's reserved Desk group once, without changing the
// screen in front; the operator's move out of the group and close are kept.
const PATH: &str = "/desk-group";
const TABS: [&str; 3] = ["w-order:t1", "w-order:t2", "w-order:t3"];

/// A runtime running Desk with one checkout of three tabs, the first in
/// front, in which Desk's spawn's session is the pane of the second tab.
fn setup() -> Runtime {
    setup_in(TABS[1])
}

/// The same runtime with Desk's spawn's session in the pane of `tab`.
fn setup_in(tab: &str) -> Runtime {
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "desk-group");
    runtime.ingest_session(Ok(tab_order_payload(PATH, &TABS, &TABS, TABS[0])));
    runtime.sync_workspace_view();
    let mut ledger = Ledger::default();
    let desk = Actor::hide(HIDE_DESK_ROLE, crate::node::TEST_NODE);
    let origin = register(&mut ledger, &desk);
    let pane = format!("{tab}:p");
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
        .find(|group| group.role == Some(GroupRole::Desk))
        .expect("the Desk group was made");
    assert_eq!(desk.tabs, [TABS[1]]);
    // Its row is not read yet, so it is still starting for Desk.
    assert_eq!(job_state(&runtime), DeskJobState::Starting);

    // A second publish places nothing again.
    runtime.refresh_desk_jobs();
    let (_, again) = groups(&runtime);
    assert_eq!(
        again
            .iter()
            .filter(|group| group.role == Some(GroupRole::Desk))
            .count(),
        1
    );
}

/// The tab in front is Desk's session: it stays on screen, so the group in
/// front is the reserved Desk group it went into, never the one it left empty
/// (with-hide/hide-private#466).
#[test]
fn a_session_desk_started_in_the_tab_in_front_stays_on_screen_in_the_desk_group() {
    let mut runtime = setup_in(TABS[0]);
    // The group in front and the tab it shows.
    let shown = |runtime: &Runtime| {
        let view = runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(crate::node::TEST_NODE, PATH)
            .unwrap();
        (
            view.agent_groups.selected.clone(),
            view.selected_agent_layout().active().map(str::to_owned),
        )
    };
    assert_eq!(shown(&runtime), (None, Some(TABS[0].to_owned())));
    runtime.refresh_desk_jobs();
    // The engine's next answer can land before the next publish follows the
    // focused tab, and it reads whatever group is in front then.
    runtime.ingest_session(Ok(tab_order_payload(PATH, &TABS, &TABS, TABS[0])));
    runtime.sync_workspace_view();

    let (_, groups_now) = groups(&runtime);
    let desk = groups_now
        .iter()
        .find(|group| group.role == Some(GroupRole::Desk))
        .expect("the Desk group was made");
    assert_eq!(desk.tabs, [TABS[0]]);
    assert_eq!(shown(&runtime), (desk.id.clone(), Some(TABS[0].to_owned())));
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
            .filter(|group| group.role == Some(GroupRole::Desk))
            .all(|group| !group.tabs.iter().any(|tab| tab == TABS[1]))
    );
}

/// Where a job stands for Desk and what its agent does are two facts: a
/// session moved out of the Desk group goes on working and asking, and its
/// card reads that from the session's sidebar row, the value the row's mark
/// shows, whatever the card's own place.
#[test]
fn a_moved_out_session_reads_what_its_agent_does_from_its_sidebar_row() {
    let mut runtime = setup();
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    move_tab(&mut runtime, None);
    let row = |runtime: &mut Runtime, status: &str| {
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [
                {"pane_id": format!("{}:p", TABS[1]), "workspace_id": "w-order",
                 "tab_id": TABS[1], "terminal_id": "t2", "revision": 1, "focused": false,
                 "agent_status": status, "agent": "claude", "name": "child-fix",
                 "cwd": PATH, "state_change_seq": 1},
            ]}))
            .unwrap();
        runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
        runtime.refresh_desk_jobs();
        let row = runtime.snapshot.navigator.agents[0].state.clone();
        let card = runtime.snapshot.desk_jobs.as_ref().unwrap().jobs[0].clone();
        (row.working, row.asking, card)
    };

    let (working, asking, card) = row(&mut runtime, "working");
    assert_eq!((working, asking), (true, false));
    assert_eq!(card.state, DeskJobState::MovedOut);
    assert_eq!(card.activity, Some(DeskJobActivity::Working));

    let (working, asking, card) = row(&mut runtime, "blocked");
    assert_eq!((working, asking), (false, true));
    assert_eq!(card.state, DeskJobState::MovedOut);
    assert_eq!(card.activity, Some(DeskJobActivity::Asking));
    assert!(card.ask.is_some(), "its card says what it asks");

    let (working, asking, card) = row(&mut runtime, "done");
    assert_eq!((working, asking), (false, false));
    assert_eq!(card.state, DeskJobState::MovedOut);
    assert_eq!(card.activity, Some(DeskJobActivity::Done));
}

/// A runtime running Desk whose start failed after its tab opened, the
/// pane of the second tab: its pane made and its agent started, no child
/// bound and the spawn not completed, as `brief_not_submitted` leaves it.
fn failed_start() -> Runtime {
    use crate::coordination::{Command, Mutation, apply};
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "desk-group-failed-start");
    runtime.ingest_session(Ok(tab_order_payload(PATH, &TABS, &TABS, TABS[0])));
    runtime.sync_workspace_view();
    let pane = format!("{}:p", TABS[1]);
    let desk = Actor::hide(HIDE_DESK_ROLE, crate::node::TEST_NODE);
    let command = Command::Spawn {
        pick: None,
        mission: None,
        plan_node: None,
        plan: None,
        parent: None,
        machine: None,
        name: "child-fix".into(),
        intent: "intent-fix".into(),
        kind: "claude".into(),
        repo: Some("/repo".into()),
        branch: Some("fix".into()),
        path: None,
        args: Vec::new(),
        launch: Default::default(),
        brief: Some("fix it".into()),
        task: None,
        beside: false,
        profile: None,
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
            made: false,
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
    runtime
}

/// A start of Desk's that failed after its tab opened (its brief not taken
/// in time) leaves its agent in that tab: the tab is Desk's like any other,
/// and the card follows the pane while its agent works, reading failed while
/// it rests, where `다시 시도` hands the brief over in the same pane.
#[test]
fn a_start_that_failed_after_its_tab_opened_joins_the_group_and_its_card_follows_the_pane() {
    let mut runtime = failed_start();
    let pane = format!("{}:p", TABS[1]);
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();

    let (_, groups_now) = groups(&runtime);
    let group = groups_now
        .iter()
        .find(|group| group.role == Some(GroupRole::Desk))
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
    assert_eq!(card(&runtime).state, DeskJobState::Open);
    assert_eq!(card(&runtime).activity, Some(DeskJobActivity::Working));
    assert_eq!(card(&runtime).failure, None);
}

/// A start that failed after Desk put its tab in the Desk group and was
/// started again in a new tab, its first tab gone with its pane: the new tab
/// goes into the group too, the job is not read as moved out, and 한눈에 lists
/// the new session. Desk's placement and its 한눈에 mark were about the first
/// pane; reading them for the new one marked a session moved out seconds
/// after it started and left it out of 한눈에. A second retry ends the same
/// way, and a refresh of the same pane does not list again a session 한눈에
/// let go.
#[test]
fn a_start_retried_in_a_new_tab_joins_the_desk_group_and_is_not_moved_out() {
    let mut runtime = failed_start();
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    for (left, session) in [
        ([TABS[0], TABS[2]], "retried"),
        ([TABS[0], "w-order:t4"], "again"),
    ] {
        let tab = left[1];
        let pane = format!("{tab}:p");
        runtime.ingest_session(Ok(tab_order_payload(PATH, &left, &left, TABS[0])));
        runtime.sync_workspace_view();
        let mut ledger = Ledger::default();
        let desk = Actor::hide(HIDE_DESK_ROLE, crate::node::TEST_NODE);
        let origin = register(&mut ledger, &desk);
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
                session,
            },
        )
        .unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger));
        working_row(&mut runtime, tab);
        runtime.refresh_desk_jobs();
        runtime.sync_workspace_view();

        let (_, groups_now) = groups(&runtime);
        let group = groups_now
            .iter()
            .find(|group| group.role == Some(GroupRole::Desk))
            .expect("the Desk group was kept");
        assert_eq!(group.tabs, [tab], "{session}");
        assert_eq!(job_state(&runtime), DeskJobState::Open, "{session}");
        assert_eq!(glance_sessions(&runtime), vec![pane.clone()], "{session}");
    }

    // 한눈에 let go of it (a full list pushes the oldest out): the same pane
    // is not listed again.
    let pane = "w-order:t4:p";
    runtime
        .test_desk_stored()
        .glance
        .retain(|kept| kept.pane_id != pane);
    runtime.refresh_desk_jobs();
    assert!(glance_sessions(&runtime).is_empty());
}

/// A mark a release saved before marks named their pane is about the pane
/// there now: what it says of 한눈에 and of the tab's move out of the group
/// stays, so an upgrade lists no session again and puts no tab back.
#[test]
fn a_mark_saved_without_its_pane_keeps_what_it_says_of_the_pane_there_now() {
    let mut runtime = setup();
    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    move_tab(&mut runtime, None);
    runtime.refresh_desk_jobs();
    let pane = format!("{}:p", TABS[1]);
    for mark in &mut runtime.test_desk_stored().marks {
        assert_eq!(mark.pane.as_deref(), Some(pane.as_str()));
        mark.pane = None;
    }
    runtime
        .test_desk_stored()
        .glance
        .retain(|kept| kept.pane_id != pane);

    runtime.refresh_desk_jobs();
    runtime.sync_workspace_view();
    assert!(glance_sessions(&runtime).is_empty(), "not listed again");
    assert_eq!(job_state(&runtime), DeskJobState::MovedOut);
    let (_, groups_now) = groups(&runtime);
    assert!(
        groups_now
            .iter()
            .filter(|group| group.role == Some(GroupRole::Desk))
            .all(|group| !group.tabs.iter().any(|tab| tab == TABS[1])),
        "its tab is not put back"
    );
    let marks = &runtime.test_desk_stored().marks;
    assert!(
        marks
            .iter()
            .all(|mark| mark.pane.as_deref() == Some(pane.as_str()))
    );
}

/// One working sidebar row, for the pane of `tab`.
fn working_row(runtime: &mut Runtime, tab: &str) {
    let payload: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(json!({"agents": [
            {"pane_id": format!("{tab}:p"), "workspace_id": "w-order", "tab_id": tab,
             "terminal_id": "t2", "revision": 1, "focused": false, "agent_status": "working",
             "agent": "claude", "name": "child-fix", "cwd": PATH, "state_change_seq": 1},
        ]}))
        .unwrap();
    runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
}

fn glance_sessions(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot
        .desk_jobs
        .as_ref()
        .unwrap()
        .glance
        .sessions
        .clone()
}
