use super::*;

const CHECKOUT: &str = "/fixture/pane-move";
const FIRST: &str = "w-order:t1";
const SECOND: &str = "w-order:t2";

pub(in crate::runtime) fn arranged() -> Runtime {
    let (runtime, _) = tab_order_runtime(CHECKOUT);
    let mut runtime = workspace_view::with_new_views(runtime, "pane-move");
    let payload = topology(&[FIRST, SECOND]);
    runtime.ingest_session(Ok(payload));
    runtime
}

pub(in crate::runtime) fn topology(tabs: &[&str]) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(CHECKOUT, tabs, tabs, tabs[0]);
    for pane in &mut payload.panes {
        pane.terminal_id = Some(format!("terminal:{}", pane.pane_id));
    }
    payload
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
    // The catalog groups native workspaces by checkout and gives the group
    // its own ID. A durable move must retain the native layout's identity.
    let catalog_tab = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .find(|tab| tab.id.as_deref() == Some(FIRST))
        .unwrap();
    assert_ne!(catalog_tab.workspace_id.as_deref(), Some("w-order"));
    let saved = &runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, CHECKOUT)
        .unwrap()
        .pane_moves["sole"];
    assert_eq!(
        serde_json::to_value(saved).unwrap()["workspace_id"],
        "w-order"
    );
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

#[test]
fn remote_pane_move_uses_device_catalog_and_native_layout_identity() {
    let mut runtime = arranged();
    let device = "mini";
    let mut row = runtime.snapshot.navigator.devices[0].clone();
    row.id = device.into();
    row.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(row);
    let tab = "remote:mini:tab:w9:t1";
    let pane_id = "remote:mini:pane:w9:t1";
    runtime
        .snapshot
        .status
        .remote
        .push(super::device_catalog::connected_status(device));
    let mut raw = super::device_catalog::session(vec![super::device_catalog::herdr_workspace(
        device,
        "w9",
        CHECKOUT,
        &[("w9:t1", CHECKOUT)],
    )]);
    raw.workspaces[0].checkouts[0].tabs[0].panes[0].terminal_id = Some("remote-terminal".into());
    raw.pane_layouts
        .push(crate::model::RemotePaneLayoutSnapshot {
            workspace_id: "remote:mini:workspace:w9".into(),
            tab_id: tab.into(),
            focused_pane_id: pane_id.into(),
            zoomed: false,
            frames: vec![crate::model::RemotePaneLayoutFrame {
                pane_id: pane_id.into(),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            }],
        });
    runtime.ingest_remote_session(device, Ok(raw));
    let key = (device.to_owned(), CHECKOUT.to_owned());
    assert!(runtime.choose_agent_workspace(&key));
    runtime.sync_workspace_view();
    let view = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(device, CHECKOUT)
        .unwrap();
    let area = view
        .selected_agent_layout()
        .tree
        .area_of(tab)
        .unwrap()
        .id
        .clone();
    dispatch(
        &mut runtime,
        serde_json::json!({
            "request_id":"remote-sole", "workspace":{"device_id":device,"checkout_path":CHECKOUT},
            "group_id":null,"source":{"pane_id":pane_id,"tab_id":tab,"area_id":area},
            "destination":{"type":"new_tab","area_id":area}
        }),
    );
    let operation = runtime
        .snapshot
        .status
        .async_operations
        .iter()
        .find(|operation| operation.id == "remote-sole")
        .unwrap();
    assert_eq!(operation.phase, "completed", "{:?}", operation.message);
    let saved = &runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(device, CHECKOUT)
        .unwrap()
        .pane_moves["remote-sole"];
    assert_eq!(serde_json::to_value(saved).unwrap()["workspace_id"], "w9");
}
