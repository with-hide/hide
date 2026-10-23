//! The sidebar's lens, the 태스크 view option and a task revealed from
//! outside the list (PRD sidebar-task-lens B2, B3, B40, D-47, D-48).
use super::*;
use crate::model::{DetailItem, SidebarLens, TaskView};

fn event(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event(kind, payload))
}

/// A 현황 tile narrows both lists and turns the sidebar to one in one event;
/// the chip's ✕ lifts the narrowing and leaves the lens where it is.
#[test]
fn the_sidebar_project_narrows_both_lists_until_lifted() {
    let mut runtime = runtime();
    assert_eq!(runtime.snapshot.ui_state.sidebar_project, None);
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    let project = serde_json::json!({"device_id": "mini", "workspace_id": "remote:mini:project:p"});
    assert!(event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": project, "lens": "prs", "show": true})
    ));
    let state = &runtime.snapshot.ui_state;
    assert_eq!(
        state.sidebar_project.as_ref().map(|p| p.workspace_id.as_str()),
        Some("remote:mini:project:p")
    );
    assert_eq!(state.sidebar_lens, SidebarLens::Prs);
    assert!(state.left_sidebar_visible);
    assert!(!event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": project})
    ));
    assert!(event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": null})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_project, None);
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Prs);
}

#[test]
fn the_lens_and_view_option_are_kept_and_a_hidden_sidebar_opens_on_the_other_lens() {
    let mut runtime = runtime();
    assert_eq!(
        runtime.snapshot.ui_state.sidebar_lens,
        SidebarLens::Projects
    );
    assert_eq!(runtime.snapshot.ui_state.task_view, TaskView::Status);
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    assert!(event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "tasks", "show": true})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Tasks);
    assert!(
        runtime.snapshot.ui_state.left_sidebar_visible,
        "⇧⌘B with the sidebar hidden shows it in the same event"
    );
    assert!(!event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "tasks"})
    ));
    assert!(event(
        &mut runtime,
        "task_view_set",
        serde_json::json!({"view": "project"})
    ));
    assert_eq!(runtime.snapshot.ui_state.task_view, TaskView::Project);
    // The PR list is the third lens (PRD lane-two-columns B41).
    assert!(event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "prs"})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Prs);
    event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "sessions"}),
    );
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(
        runtime.snapshot.ui_state.sidebar_lens,
        SidebarLens::Prs,
        "a lens this build does not know changes nothing"
    );
}

#[test]
fn a_revealed_task_unfolds_its_ancestors_and_opens_the_agent_s_record() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents = project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id":"root", "agent":"claude", "agent_status":"working", "state_change_seq":1},
            {"pane_id":"middle", "agent":"claude", "agent_status":"working", "state_change_seq":2},
            {"pane_id":"leaf", "agent":"claude", "agent_status":"idle", "state_change_seq":3}
        ]}))
        .unwrap(),
    )
    .agents;
    runtime.snapshot.navigator.agents[1].lineage_parent_pane_id = Some("root".into());
    runtime.snapshot.navigator.agents[2].lineage_parent_pane_id = Some("middle".into());
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    let session = crate::links::ledger::SessionIdentity {
        device_id: "local".into(),
        agent: "claude".into(),
        session_id: "s-leaf".into(),
    };
    assert!(event(
        &mut runtime,
        "task_reveal",
        serde_json::json!({"pane_id": "leaf", "session": session})
    ));
    let state = &runtime.snapshot.ui_state;
    assert!(state.left_sidebar_visible);
    assert_eq!(state.sidebar_lens, SidebarLens::Tasks);
    assert_eq!(state.sessions_expanded_agent_pane_ids, ["middle", "root"]);
    assert_eq!(state.detail.items, [DetailItem::Record { session }]);
    assert!(state.detail.shown);
}

/// The slot steps back through at most 20 items, the oldest leaving first,
/// and opening one it already holds moves it to the front rather than adding
/// a second entry (B47).
#[test]
fn the_detail_slot_keeps_the_newest_twenty_items_and_reopening_one_moves_it_forward() {
    let mut runtime = runtime();
    let record = |index: usize| DetailItem::Record {
        session: crate::links::ledger::SessionIdentity {
            device_id: "local".into(),
            agent: "claude".into(),
            session_id: format!("s-{index}"),
        },
    };
    for index in 0..22 {
        assert!(event(
            &mut runtime,
            "detail_open",
            serde_json::json!({"item": record(index)})
        ));
    }
    let items = &runtime.snapshot.ui_state.detail.items;
    assert_eq!(items.len(), crate::model::DETAIL_HISTORY_LIMIT);
    assert_eq!(items.first(), Some(&record(2)), "the two oldest left first");
    assert_eq!(items.last(), Some(&record(21)));
    event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": record(5)}),
    );
    let items = &runtime.snapshot.ui_state.detail.items;
    assert_eq!(items.len(), crate::model::DETAIL_HISTORY_LIMIT);
    assert_eq!(items.last(), Some(&record(5)));
    assert_eq!(items.iter().filter(|item| **item == record(5)).count(), 1);
}
