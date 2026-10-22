//! The Overview map's remembered choices (PRD overview-graph-remembered B1-B6,
//! B16): one operator action is one event, a repeat changes nothing, and a
//! change naming nothing is refused whole.
use super::*;
use crate::model::OverviewGraphFocus;

fn change(runtime: &mut Runtime, change: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event(
        "overview_graph_change",
        serde_json::json!({ "change": change }),
    ))
}

#[test]
fn each_map_action_is_one_event_that_the_ui_state_keeps() {
    let mut runtime = runtime();
    let project = serde_json::json!({"device_id": "n", "workspace_id": "workspace:a"});
    assert!(change(
        &mut runtime,
        serde_json::json!({"kind": "project_hidden", "project": project, "hidden": true})
    ));
    assert!(
        !change(
            &mut runtime,
            serde_json::json!({"kind": "project_hidden", "project": project, "hidden": true})
        ),
        "hiding a hidden project publishes nothing"
    );
    assert!(change(
        &mut runtime,
        serde_json::json!({"kind": "focus", "focus": "turn", "include_resting": true})
    ));
    assert!(change(
        &mut runtime,
        serde_json::json!({"kind": "place", "scope": {"kind": "all", "device_id": "n"},
            "projects": [{"key": project, "x": 3.0, "y": 4.0}], "sessions": []})
    ));
    let choices = &runtime.snapshot.ui_state.overview_graph;
    assert_eq!(choices.focus, OverviewGraphFocus::Turn);
    assert!(choices.include_resting);
    assert_eq!(choices.hidden_projects.len(), 1);
    assert_eq!(choices.project_places.len(), 1);
    assert!(change(
        &mut runtime,
        serde_json::json!({"kind": "reset_places", "scope": {"kind": "all", "device_id": "n"}})
    ));
    let choices = &runtime.snapshot.ui_state.overview_graph;
    assert!(choices.project_places.is_empty());
    assert_eq!(
        choices.hidden_projects.len(),
        1,
        "Reset layout keeps what is hidden"
    );
    assert!(runtime.snapshot.status.last_error.is_none());
}

#[test]
fn a_change_naming_an_empty_project_is_refused_and_changes_nothing() {
    let mut runtime = runtime();
    change(
        &mut runtime,
        serde_json::json!({"kind": "fold_all", "projects": [
            {"device_id": "n", "workspace_id": "workspace:a"},
            {"device_id": "n", "workspace_id": ""},
        ]}),
    );
    assert!(runtime.snapshot.status.last_error.is_some());
    assert!(
        runtime
            .snapshot
            .ui_state
            .overview_graph
            .folded_projects
            .is_empty()
    );
}
