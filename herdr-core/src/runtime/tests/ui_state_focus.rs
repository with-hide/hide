//! A whole-state save cannot move what the core owns about focus.
//!
//! The shell's `ui_state_update` carries its last copy of the UI state, and a
//! choice made since (a click on another pane, another checkout, another
//! device) can already have overtaken that copy. The keyboard's pane, the
//! checkout and the device move on their own events, which tell Herdr; a save
//! moves none of them. Issue 672: a save built before a pane click put the
//! core back on the pane the operator had left, and following Herdr back to
//! the clicked pane later took the keyboard from the tab-name field the
//! operator was typing in.

use super::appearance::ui_state_update;
use super::*;

#[derive(Clone, Copy, Debug)]
enum Anchor {
    Pane,
    Checkout,
    Device,
}

/// Two checkouts, each with one pane, and a second device; the core starts on
/// checkout A, its pane, and this machine. A pane focus is refused without a
/// live Herdr, so the runtime has one.
fn fixture() -> Runtime {
    let mut runtime = live_runtime();
    runtime.snapshot.navigator.workspaces = vec![
        workspace(
            "workspace-a",
            "A",
            "/tmp/hide-ui-state-focus-a",
            vec![checkout(
                "workspace-a",
                "checkout-a",
                "/tmp/hide-ui-state-focus-a",
                Some(pane("pane-a", "/tmp/hide-ui-state-focus-a")),
            )],
        ),
        workspace(
            "workspace-b",
            "B",
            "/tmp/hide-ui-state-focus-b",
            vec![checkout(
                "workspace-b",
                "checkout-b",
                "/tmp/hide-ui-state-focus-b",
                Some(pane("pane-b", "/tmp/hide-ui-state-focus-b")),
            )],
        ),
    ];
    runtime.snapshot.navigator.devices.push(DeviceSnapshot {
        agent_scope: Default::default(),
        id: "mini".to_owned(),
        label: Some("Mac mini".to_owned()),
        kind: "remote".to_owned(),
        state: "connected".to_owned(),
        message: None,
        problem: None,
        sign_in: None,
        ssh_alias: Some("mini".to_owned()),
        wsl_distro: None,
        herdr_socket_path: None,
        agent_count: 0,
        test: None,
        host: Default::default(),
        kit: Default::default(),
        machine_name: None,
        dials_in: false,
        herdr_behind: None,
    });
    runtime.snapshot.navigator.focused_device_id = Some(crate::node::TEST_NODE.to_owned());
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace-a".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout-a".to_owned());
    runtime.snapshot.navigator.root_path = Some("/tmp/hide-ui-state-focus-a".to_owned());
    runtime.persist_current_ui_state();
    runtime.select_terminal_pane(Some("pane-a".to_owned()));
    runtime
}

/// The event that moves the anchor, and the value it moves it to.
fn move_anchor(anchor: Anchor) -> (Vec<u8>, &'static str) {
    let (kind, payload, value) = match anchor {
        Anchor::Pane => (
            "focus_pane",
            serde_json::json!({"pane_id": "pane-b", "origin": "operator"}),
            "pane-b",
        ),
        Anchor::Checkout => (
            "focus_checkout",
            serde_json::json!({"workspace_id": "workspace-b", "checkout_id": "checkout-b"}),
            "checkout-b",
        ),
        Anchor::Device => (
            "focus_device",
            serde_json::json!({"device_id": "mini"}),
            "mini",
        ),
    };
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload
    }))
    .expect("focus event");
    (event, value)
}

/// What the core holds for the anchor, and what its UI state records.
fn read(runtime: &Runtime, anchor: Anchor) -> (Option<String>, Option<String>) {
    let snapshot = runtime.snapshot();
    match anchor {
        Anchor::Pane => (
            snapshot.focused.pane_id.clone(),
            snapshot.ui_state.selected_pane_id.clone(),
        ),
        Anchor::Checkout => (
            snapshot.navigator.focused_checkout_id.clone(),
            snapshot.ui_state.focused_checkout_id.clone(),
        ),
        Anchor::Device => (
            snapshot.navigator.focused_device_id.clone(),
            snapshot.ui_state.focused_device_id.clone(),
        ),
    }
}

#[test]
fn a_save_built_before_a_focus_change_leaves_each_anchor_where_that_change_put_it() {
    let mut failures = Vec::new();
    for anchor in [Anchor::Pane, Anchor::Checkout, Anchor::Device] {
        let mut runtime = fixture();
        // The shell's copy from before the change: here, a page reporting
        // that it went out of sight.
        let stale = ui_state_update(&runtime, serde_json::json!({"usage_window_visible": false}));
        let (event, value) = move_anchor(anchor);
        runtime.dispatch_json(&event);
        let moved = read(&runtime, anchor);
        assert_eq!(
            moved.0.as_deref(),
            Some(value),
            "{anchor:?}: the event moved it (error: {:?})",
            runtime.snapshot().status.last_error
        );
        runtime.dispatch_json(&stale);
        let after = read(&runtime, anchor);
        if after != moved {
            failures.push(format!("{anchor:?}: {moved:?} became {after:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "a stale save moved an anchor:\n{}",
        failures.join("\n")
    );
}
