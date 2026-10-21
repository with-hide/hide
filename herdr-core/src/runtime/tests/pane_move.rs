use super::*;

const CHECKOUT: &str = "/fixture/pane-move";
const FIRST: &str = "w-order:t1";
const SECOND: &str = "w-order:t2";

fn arranged() -> Runtime {
    let (runtime, _) = tab_order_runtime(CHECKOUT);
    let mut runtime = workspace_view::with_new_views(runtime, "pane-move");
    let mut payload = tab_order_payload(CHECKOUT, &[FIRST, SECOND], &[FIRST, SECOND], FIRST);
    for pane in &mut payload.panes {
        pane.terminal_id = Some(format!("terminal:{}", pane.pane_id));
    }
    runtime.ingest_session(Ok(payload));
    runtime
}

fn move_payload(runtime: &Runtime, id: &str, destination: serde_json::Value) -> serde_json::Value {
    let view = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, CHECKOUT)
        .unwrap();
    let area = &view.selected_agent_layout().tree.area_of(FIRST).unwrap().id;
    serde_json::json!({
        "request_id":id,"workspace":{"device_id":crate::node::TEST_NODE,"checkout_path":CHECKOUT},
        "group_id":null,"source":{"pane_id":format!("{FIRST}:p"),"tab_id":FIRST,"area_id":area},
        "destination":destination
    })
}

fn dispatch(runtime: &mut Runtime, payload: serde_json::Value) {
    assert!(runtime.dispatch_json(&explorer_event("pane_move", payload)));
}

#[test]
fn sole_pane_new_tab_in_own_area_keeps_native_identity_and_title() {
    let mut runtime = arranged();
    let area = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, CHECKOUT)
        .unwrap()
        .selected_agent_layout()
        .tree
        .area_of(FIRST)
        .unwrap()
        .id
        .clone();
    let before = ordered_tab_ids(
        &runtime,
        &workspace::checkout_id_for_path("workspace:order", Path::new(CHECKOUT)),
    );
    let payload = move_payload(
        &runtime,
        "sole",
        serde_json::json!({"type":"new_tab","area_id":area}),
    );
    dispatch(&mut runtime, payload);
    assert_eq!(
        ordered_tab_ids(
            &runtime,
            &workspace::checkout_id_for_path("workspace:order", Path::new(CHECKOUT))
        ),
        before
    );
    let operation = runtime
        .snapshot
        .status
        .async_operations
        .iter()
        .find(|operation| operation.id == "sole")
        .unwrap();
    assert_eq!(operation.phase, "completed");
    assert!(!runtime.pane_move_holds_tab(FIRST));
}

#[test]
fn stale_group_and_self_drop_are_rejected_before_any_effect() {
    for stale_group in [false, true] {
        let mut runtime = arranged();
        let area = runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(crate::node::TEST_NODE, CHECKOUT)
            .unwrap()
            .selected_agent_layout()
            .tree
            .area_of(FIRST)
            .unwrap()
            .id
            .clone();
        let mut payload = move_payload(
            &runtime,
            "refused",
            serde_json::json!({"type":"pane","tab_id":FIRST,"pane_id":format!("{FIRST}:p"),"area_id":area,"edge":"right"}),
        );
        if stale_group {
            payload["group_id"] = serde_json::json!("another-group");
        }
        dispatch(&mut runtime, payload);
        let operation = runtime
            .snapshot
            .status
            .async_operations
            .iter()
            .find(|op| op.id == "refused")
            .unwrap();
        assert_eq!(operation.phase, "failed");
        assert!(operation.retryable);
        assert!(!runtime.pane_move_holds_tab(FIRST));
    }
}

#[test]
fn disconnected_preview_settles_as_failed_without_changing_focus() {
    let mut runtime = arranged();
    let before = runtime.snapshot.terminal.pane_id.clone();
    assert!(runtime.dispatch_json(&explorer_event("pane_move_preview",serde_json::json!({
        "request_id":"preview","workspace":{"device_id":crate::node::TEST_NODE,"checkout_path":CHECKOUT},"group_id":null,"tab_id":SECOND
    }))));
    assert_eq!(
        runtime
            .snapshot
            .status
            .pane_move_preview
            .as_ref()
            .unwrap()
            .phase,
        "failed"
    );
    assert_eq!(runtime.snapshot.terminal.pane_id, before);
    let snapshot = serde_json::to_value(&runtime.snapshot).unwrap();
    assert_eq!(
        snapshot["status"]["pane_move_preview"]["request_id"],
        "preview"
    );
    assert!(
        snapshot["navigator"]["workspaces"]
            .to_string()
            .contains("terminal:w-order:t1:p")
    );
}
