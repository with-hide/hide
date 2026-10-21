use super::*;
use crate::live::FocusReadback;

#[test]
fn eligible_agent_panes_open_on_the_terminal_and_keep_the_conversation_they_asked_for() {
    let mut runtime = runtime();
    let payload = |pane_ids: &[&str]| -> SessionSnapshotPayload {
        let agents: Vec<_> = pane_ids
            .iter()
            .enumerate()
            .map(|(index, pane_id)| {
                serde_json::json!({
                    "pane_id": pane_id,
                    "workspace_label": "Fixture",
                    "agent": "codex",
                    "agent_status": "idle",
                    "tokens": {"status_idle": "\u{25cb}", "activity": format!("000000000000{index}")}
                })
            })
            .collect();
        let panes: Vec<_> = pane_ids
            .iter()
            .map(|pane_id| {
                serde_json::json!({"pane_id": pane_id, "cwd": "/private/tmp/hide-conversation-default"})
            })
            .collect();
        // One tab per pane: a shared tab would need Herdr's split tree.
        let tabs: Vec<_> = pane_ids
            .iter()
            .enumerate()
            .map(|(index, _)| {
                serde_json::json!({
                    "workspace_id": "w1",
                    "tab_id": format!("w1:t{index}"),
                    "label": format!("{index}")
                })
            })
            .collect();
        let layouts: Vec<_> = pane_ids
            .iter()
            .enumerate()
            .map(|(index, pane_id)| {
                serde_json::json!({
                    "workspace_id": "w1",
                    "tab_id": format!("w1:t{index}"),
                    "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                    "focused_pane_id": pane_id,
                    "panes": [{"pane_id": pane_id, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                    "splits": []
                })
            })
            .collect();
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": agents,
            "workspaces": [{"workspace_id": "w1", "label": "fixture"}],
            "panes": panes,
            "tabs": tabs,
            "layouts": layouts
        }))
        .expect("conversation fixture")
    };
    let toggle = |pane_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "toggle_conversation",
            "payload": {"pane_id": pane_id}
        }))
        .expect("conversation toggle event")
    };
    let conversation =
        |runtime: &Runtime| runtime.snapshot().ui_state.conversation_pane_ids.clone();

    // A new agent pane is a terminal until the operator asks otherwise.
    runtime.ingest_session(Ok(payload(&["w1:p1", "w1:p2"])));
    assert_eq!(runtime.snapshot().navigator.agents.len(), 2);
    assert!(conversation(&runtime).is_empty());

    // Asking is one toggle, and the choice is per pane.
    assert!(runtime.dispatch_json(&toggle("w1:p1")));
    assert_eq!(
        conversation(&runtime).into_iter().collect::<Vec<_>>(),
        ["w1:p1"]
    );

    // A refresh keeps the choice rather than treating the pane as new again.
    runtime.ingest_session(Ok(payload(&["w1:p1", "w1:p2"])));
    assert_eq!(
        conversation(&runtime).into_iter().collect::<Vec<_>>(),
        ["w1:p1"]
    );

    // Toggling again returns the pane to its terminal.
    assert!(runtime.dispatch_json(&toggle("w1:p1")));
    assert!(conversation(&runtime).is_empty());

    // A choice does not outlive the pane that made it.
    assert!(runtime.dispatch_json(&toggle("w1:p2")));
    assert_eq!(
        conversation(&runtime).into_iter().collect::<Vec<_>>(),
        ["w1:p2"]
    );
    runtime.ingest_session(Ok(payload(&["w1:p1"])));
    assert!(conversation(&runtime).is_empty());
}

#[test]
fn local_tab_creation_acknowledgement_preserves_the_created_pane_focus() {
    let mut runtime = runtime();
    runtime.snapshot.terminal.pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w1:p1".to_owned());
    runtime.snapshot.ui_state.selected_pane_id = Some("w1:p1".to_owned());

    assert!(runtime.ingest_local_control_result(
        RemoteControlAction::CreateTab {
            workspace_id: "w1".to_owned(),
            cwd: "/tmp/project".to_owned(),
            label: "2".to_owned(),
            area_id: None,
            group_id: None,
            admission_id: None,
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w1:t2".to_owned()),
            created_pane_id: Some("w1:p2".to_owned()),
        }),
        4,
    ));

    assert_eq!(runtime.snapshot.terminal.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(
        runtime.snapshot.ui_state.selected_pane_id.as_deref(),
        Some("w1:p2")
    );
}

// A refused `pane.move` used to drop its request stamp, so the next
// session update re-sent it: one refusal per tick for as long as the
// tab stayed zoomed (2026-09-10 audit, ~2 refusals per second).
#[test]
fn a_refused_relocation_keeps_its_stamp() {
    let mut runtime = runtime();
    let asked_at = unix_milliseconds();
    runtime
        .pane_relocations_in_flight
        .insert("w1:p2".to_owned(), asked_at);

    runtime.ingest_pane_control_result(
        PaneControlAction::MoveToNewTab {
            pane_id: "w1:p2".to_owned(),
            workspace_id: "w1".to_owned(),
            label: "Child".to_owned(),
        },
        Err("Herdr declined the move: ZoomedTab".to_owned()),
        3,
    );
    assert_eq!(
        runtime.pane_relocations_in_flight.get("w1:p2"),
        Some(&asked_at),
        "a refusal waits out the retry interval"
    );
}

// Herdr acknowledges `pane.move` before the layout that shows the child in
// its own tab reaches Hide. Dropping the stamp on the acknowledgement let the
// session update in between move the child a second time, from its new tab
// into another (issue 231, a loaded CI runner).
#[test]
fn an_acknowledged_relocation_stays_in_flight_until_the_layout_shows_the_move() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(
        super::lineage::split_lineage_json(),
    )
    .expect("split payload")));
    let asked_at = unix_milliseconds();
    runtime
        .pane_relocations_in_flight
        .insert("w1:p2".to_owned(), asked_at);

    runtime.ingest_pane_control_result(
        PaneControlAction::MoveToNewTab {
            pane_id: "w1:p2".to_owned(),
            workspace_id: "w1".to_owned(),
            label: "Implementor".to_owned(),
        },
        Ok(live::PaneControlOutcome::Acknowledged {
            created_pane_id: None,
        }),
        3,
    );
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(
        super::lineage::split_lineage_json(),
    )
    .expect("split payload")));
    assert_eq!(
        runtime.pane_relocations_in_flight.get("w1:p2"),
        Some(&asked_at),
        "a layout that still shares the parent's tab is the move not yet shown"
    );

    let mut moved = super::lineage::split_lineage_json();
    moved["tabs"] = serde_json::json!([
        {"workspace_id":"w1","tab_id":"t1","label":""},
        {"workspace_id":"w1","tab_id":"t2","label":"Implementor"}
    ]);
    moved["layouts"] = serde_json::json!([
        {"workspace_id":"w1","tab_id":"t1","zoomed":false,
         "area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":"w1:p1",
         "panes":[{"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]},
        {"workspace_id":"w1","tab_id":"t2","zoomed":false,
         "area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":"w1:p2",
         "panes":[{"pane_id":"w1:p2","rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]}
    ]);
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(moved).expect("moved payload")
    ));
    assert!(
        !runtime.pane_relocations_in_flight.contains_key("w1:p2"),
        "the child in its own tab is the move done"
    );
}

/// R11/AC16: the chords move one pane's scale within bounds and reset it,
/// and the store keeps only the panes the user actually changed.
#[test]
fn pane_text_scale_steps_within_bounds_and_leaves_other_panes_alone() {
    let mut runtime = runtime();
    let scale = |pane: &str, direction: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "pane_text_scale",
            "payload": {"pane_id": pane, "direction": direction}
        }))
        .expect("pane text scale event")
    };

    assert!(runtime.dispatch_json(&scale("w1:p1", "in")));
    assert_eq!(
        runtime.snapshot().ui_state.pane_text_scales.get("w1:p1"),
        Some(&1.1)
    );
    // A second pane is untouched by the first pane's zoom.
    assert!(
        !runtime
            .snapshot()
            .ui_state
            .pane_text_scales
            .contains_key("w1:p2")
    );

    // The upper bound holds however many times it is pressed, and a press
    // that changes nothing reports no change.
    for _ in 0..40 {
        runtime.dispatch_json(&scale("w1:p1", "in"));
    }
    assert_eq!(
        runtime.snapshot().ui_state.pane_text_scales.get("w1:p1"),
        Some(&MAX_PANE_TEXT_SCALE)
    );
    assert!(!runtime.dispatch_json(&scale("w1:p1", "in")));

    for _ in 0..40 {
        runtime.dispatch_json(&scale("w1:p1", "out"));
    }
    assert_eq!(
        runtime.snapshot().ui_state.pane_text_scales.get("w1:p1"),
        Some(&MIN_PANE_TEXT_SCALE)
    );

    // Reset drops the row rather than storing the default.
    assert!(runtime.dispatch_json(&scale("w1:p1", "reset")));
    assert!(runtime.snapshot().ui_state.pane_text_scales.is_empty());
    assert!(!runtime.dispatch_json(&scale("w1:p1", "reset")));

    // An unknown direction is surfaced, not silently ignored.
    assert!(runtime.dispatch_json(&scale("w1:p1", "sideways")));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.clone()),
        Some("pane.text_scale_unknown_direction".to_owned())
    );
}

#[test]
fn project_checkouts_fold_applies_workspace_expansion_without_waiting_for_sync() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "workspace-a",
        "A",
        "/tmp/hide-runtime-a",
        Vec::new(),
    )];
    let fold = |runtime: &mut Runtime, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "project_checkouts_fold",
            "payload": payload
        }))
        .expect("fold event");
        runtime.dispatch_json(&event)
    };

    // Absent `expanded` flips: two flips sent before either is echoed land
    // back where they started, which a set computed from a stale snapshot
    // could not do.
    assert!(fold(
        &mut runtime,
        serde_json::json!({"workspace_id": "workspace-a"})
    ));
    assert!(!runtime.snapshot().navigator.workspaces[0].expanded);
    assert_eq!(
        runtime.snapshot().ui_state.collapsed_workspace_ids,
        ["workspace-a"]
    );
    assert!(fold(
        &mut runtime,
        serde_json::json!({"workspace_id": "workspace-a"})
    ));
    assert!(runtime.snapshot().navigator.workspaces[0].expanded);
    assert!(
        runtime
            .snapshot()
            .ui_state
            .collapsed_workspace_ids
            .is_empty()
    );

    // A given `expanded` sets, so repeating the intent changes nothing.
    assert!(fold(
        &mut runtime,
        serde_json::json!({"workspace_id": "workspace-a", "expanded": false})
    ));
    assert!(!fold(
        &mut runtime,
        serde_json::json!({"workspace_id": "workspace-a", "expanded": false})
    ));
    assert!(!runtime.snapshot().navigator.workspaces[0].expanded);
    assert_eq!(
        runtime.snapshot().ui_state.collapsed_workspace_ids,
        ["workspace-a"]
    );

    // A whole-state save, which the shell sends for unrelated settings, neither
    // reads nor writes the folds, so a stale copy of them cannot undo one.
    let save = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "collapsed_workspace_ids": [],
            "collapsed_checkout_ids": ["checkout-a"],
            "selected_path": null,
            "selected_pane_id": null,
            "shortcut_bindings": {}
        }
    }))
    .expect("whole-state save");
    assert!(runtime.dispatch_json(&save));
    assert!(!runtime.snapshot().navigator.workspaces[0].expanded);
    assert_eq!(
        runtime.snapshot().ui_state.collapsed_workspace_ids,
        ["workspace-a"]
    );
    assert_eq!(
        runtime.snapshot().ui_state.collapsed_checkout_ids,
        ["checkout-a"]
    );
}

#[test]
fn focusing_checkout_selects_only_the_target_checkout_pane() {
    let mut runtime = runtime();
    let workspace_a = workspace(
        "workspace-a",
        "A",
        "/tmp/hide-runtime-a",
        vec![checkout(
            "workspace-a",
            "checkout-a",
            "/tmp/hide-runtime-a",
            Some(pane("pane-a", "/tmp/hide-runtime-a")),
        )],
    );
    let workspace_b = workspace(
        "workspace-b",
        "B",
        "/tmp/hide-runtime-b",
        vec![
            checkout(
                "workspace-b",
                "checkout-b",
                "/tmp/hide-runtime-b",
                Some(pane("pane-b", "/tmp/hide-runtime-b")),
            ),
            checkout(
                "workspace-b",
                "checkout-empty",
                "/tmp/hide-runtime-empty",
                None,
            ),
        ],
    );
    runtime.snapshot.navigator.workspaces = vec![workspace_a, workspace_b];
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace-a".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout-a".to_owned());
    runtime.snapshot.navigator.root_path = Some("/tmp/hide-runtime-a".to_owned());
    runtime.snapshot.terminal.pane_id = Some("pane-a".to_owned());
    runtime.snapshot.terminal.panes = vec![TerminalPaneSnapshot {
        pane_id: "pane-a".to_owned(),
        closed: false,
        exit_code: None,
        ..TerminalPaneSnapshot::default()
    }];
    runtime.snapshot.ui_state.selected_pane_id = Some("pane-a".to_owned());

    let focus_b = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_checkout",
        "payload": {"workspace_id": "workspace-b", "checkout_id": "checkout-b"}
    }))
    .expect("focus B event");
    assert!(runtime.dispatch_json(&focus_b));
    assert_eq!(
        runtime.snapshot().navigator.focused_workspace_id.as_deref(),
        Some("workspace-b")
    );
    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some("checkout-b")
    );
    assert_eq!(
        runtime.snapshot().navigator.root_path.as_deref(),
        Some("/tmp/hide-runtime-b")
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("pane-b")
    );
    assert_eq!(
        runtime.snapshot().ui_state.selected_pane_id.as_deref(),
        Some("pane-b")
    );
    // The selection moved; the entry of the pane left behind is the
    // session's, and only the session drops it (ARCHITECTURE.md, Panes,
    // tabs and the attach window).
    assert_eq!(projected_pane_ids(&runtime), ["pane-a"]);

    let focus_empty = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_checkout",
        "payload": {"workspace_id": "workspace-b", "checkout_id": "checkout-empty"}
    }))
    .expect("focus pane-less checkout event");
    assert!(runtime.dispatch_json(&focus_empty));
    assert_eq!(
        runtime.snapshot().navigator.root_path.as_deref(),
        Some("/tmp/hide-runtime-empty")
    );
    assert!(runtime.snapshot().terminal.pane_id.is_none());
    assert!(runtime.snapshot().active_pane_layout().is_none());
    assert_eq!(projected_pane_ids(&runtime), ["pane-a"]);
}

/// Every tab in the session ships its own layout, keyed by its tab id.
/// Before this the snapshot carried one layout, so a tab the operator was
/// not looking at had no geometry to draw and switching to it had to wait
/// for Herdr to send one.
#[test]
fn tab_layouts_carry_every_tab_in_the_session() {
    let checkout_path = "/private/tmp/hide-tab-layouts-all";
    let (mut runtime, _checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));

    assert_eq!(
        runtime
            .snapshot()
            .pane_layouts
            .iter()
            .map(|layout| layout.tab_id.clone())
            .collect::<Vec<_>>(),
        tabs.map(str::to_owned).to_vec()
    );
    // Each entry is that tab's own geometry rather than a copy of the
    // visible one.
    for tab_id in tabs {
        let layout = runtime
            .snapshot()
            .pane_layouts
            .iter()
            .find(|layout| layout.tab_id == tab_id)
            .expect("every tab has a layout")
            .clone();
        assert_eq!(layout.focused_pane_id, format!("{tab_id}:p"));
    }
}

/// Switching tabs empties nothing. The tab being selected already has its
/// geometry, so the canvas draws it on the same dispatch instead of
/// showing an empty canvas until Herdr confirms the focus.
#[test]
fn tab_layouts_survive_a_tab_switch_with_no_empty_canvas() {
    let checkout_path = "/private/tmp/hide-tab-layouts-switch";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.tab_id.clone()),
        Some("w-order:t1".to_owned())
    );

    let focus = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_tab",
        "payload": {
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "tab_id": "w-order:t3"
        }
    }))
    .expect("focus tab event");
    assert!(runtime.dispatch_json(&focus));

    // Every tab still has its layout, and the canvas is already drawing
    // the one that was asked for.
    assert_eq!(
        runtime
            .snapshot()
            .pane_layouts
            .iter()
            .map(|layout| layout.tab_id.clone())
            .collect::<Vec<_>>(),
        tabs.map(str::to_owned).to_vec()
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.tab_id.clone()),
        Some("w-order:t3".to_owned())
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t3:p")
    );
}

pub(super) fn finish_running_pane_focus(runtime: &mut Runtime, result: Result<(), &str>) {
    let control = runtime.pane_focus_in_flight.clone().expect("running focus");
    let result = result
        .map(|()| {
            let mut layout = runtime
                .layout_holding_pane(&control.target_id)
                .expect("focus target layout")
                .clone();
            layout.focused_pane_id = control.target_id.clone();
            layout
        })
        .map_err(|message| live::ControlFailure::Definite(message.to_owned()));
    runtime.complete_lane_pane_focus(control, result, 8);
}

/// PRD B24. A relationship Open is completed only by the outcome carrying
/// its request id. An unrelated app-wide error and the already projected
/// layout cannot answer it while Herdr still reports the prior focus.
#[test]
fn pane_focus_request_waits_for_its_matching_authoritative_confirmation() {
    let checkout_path = "/private/tmp/hide-pane-focus-request";
    let (mut runtime, _) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));

    assert!(runtime.dispatch_json(&correlated_pane_focus_event(
        "w-order:t2:p",
        "relationship-1",
    )));
    let request = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the request is projected");
    assert_eq!(request.phase, "pending");
    assert_eq!(request.request_id, "relationship-1");

    runtime.set_error("pane.focus_failed", "an unrelated pane failed", true);
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.phase.as_str()),
        Some("pending"),
        "neither global last_error nor the prior layout is this request's answer"
    );

    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "pending"
    );
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.phase.as_str()),
        Some("succeeded")
    );
}

/// A child can live in another registered checkout. Opening it is still one
/// pane-focus intent: Hide moves the local project context on the dispatch,
/// then the session event from that pane confirms the same request.
#[test]
fn pane_focus_request_moves_to_the_checkout_that_owns_the_target() {
    let fixture_root_folder = scratch_dir("herdr-core-cross-checkout-");
    let fixture_root = fixture_root_folder.path().to_path_buf();
    let root_a = fixture_root.join("alpha");
    let root_b = fixture_root.join("beta");
    std::fs::create_dir_all(&root_a).expect("first registered checkout");
    std::fs::create_dir_all(&root_b).expect("second registered checkout");
    let path_a = root_a.to_string_lossy().into_owned();
    let path_b = root_b.to_string_lossy().into_owned();
    let project_a = "project:a";
    let project_b = "project:b";
    let checkout_a = workspace::checkout_id_for_path(project_a, &root_a);
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![
        WorkspaceRegistration {
            primary_checkout_id: None,
            id: project_a.to_owned(),
            label: "Alpha".to_owned(),
            path: path_a.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        },
        WorkspaceRegistration {
            primary_checkout_id: None,
            id: project_b.to_owned(),
            label: "Beta".to_owned(),
            path: path_b.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        },
    ];
    runtime.rebuild_catalog();
    runtime.snapshot.navigator.focused_workspace_id = Some(project_a.to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_a.clone());
    runtime.snapshot.navigator.root_path = Some(path_a.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_a);
    let socket_path = std::env::temp_dir()
        .join(format!(
            "herdr-core-cross-checkout-focus-{}-{}.sock",
            std::process::id(),
            NEXT_RUNTIME_STATE_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    runtime.live = Some(live::LiveContext {
        socket_path: socket_path.clone().into(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    });
    let payload = |focused_workspace_id: &str, focused_pane_id: &str| {
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [],
            "focused_workspace_id": focused_workspace_id,
            "focused_pane_id": focused_pane_id,
            "workspaces": [
                {"workspace_id": "herdr-a", "label": "Alpha", "active_tab_id": "herdr-a:t1"},
                {"workspace_id": "herdr-b", "label": "Beta", "active_tab_id": "herdr-b:t1"}
            ],
            "tabs": [
                {"workspace_id": "herdr-a", "tab_id": "herdr-a:t1", "label": "1"},
                {"workspace_id": "herdr-b", "tab_id": "herdr-b:t1", "label": "1"}
            ],
            "panes": [
                {"pane_id": "herdr-a:p1", "cwd": path_a},
                {"pane_id": "herdr-b:p1", "cwd": path_b}
            ],
            "layouts": [
                {
                    "workspace_id": "herdr-a", "tab_id": "herdr-a:t1", "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                    "focused_pane_id": "herdr-a:p1",
                    "panes": [{"pane_id": "herdr-a:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                    "splits": []
                },
                {
                    "workspace_id": "herdr-b", "tab_id": "herdr-b:t1", "zoomed": false,
                    "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                    "focused_pane_id": "herdr-b:p1",
                    "panes": [{"pane_id": "herdr-b:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                    "splits": []
                }
            ]
        }))
        .expect("two-checkout session payload")
    };

    runtime.ingest_session(Ok(payload("herdr-a", "herdr-a:p1")));
    let checkout_b = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == project_b)
        .and_then(|workspace| workspace.checkouts.first())
        .map(|checkout| checkout.id.clone())
        .expect("second registered checkout is projected");
    assert!(runtime.dispatch_json(&correlated_pane_focus_event(
        "herdr-b:p1",
        "relationship-cross-checkout",
    )));
    assert_eq!(
        runtime.snapshot().navigator.focused_workspace_id.as_deref(),
        Some(project_b)
    );
    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some(checkout_b.as_str())
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("herdr-b:p1")
    );

    runtime.ingest_session(Ok(payload("herdr-b", "herdr-b:p1")));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .map(|request| request.phase.as_str()),
        Some("succeeded")
    );
    assert_eq!(diagnostic_count(&runtime, "view_focus.timed_out"), 0);
    std::fs::remove_dir_all(fixture_root).ok();
}

/// PRD B24, engineering rule 11. Replaying one request id produces no
/// second pane-control effect. A retry receives a new id only after the
/// bounded socket operation has ended the first wait.
#[test]
fn pane_focus_request_blocks_duplicates_waits_for_transport_and_accepts_a_retry() {
    let checkout_path = "/private/tmp/hide-pane-focus-timeout";
    let (mut runtime, _) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let request = correlated_pane_focus_event("w-order:t2:p", "relationship-1");
    assert!(runtime.dispatch_json(&request));
    let requested_at = runtime
        .pending_pane_focus
        .as_ref()
        .expect("one pane focus is in flight")
        .requested_at_unix_ms;

    assert!(runtime.dispatch_json(&request));
    assert_eq!(
        runtime
            .pending_pane_focus
            .as_ref()
            .map(|pending| pending.requested_at_unix_ms),
        Some(requested_at),
        "the duplicate did not replace or repeat the request"
    );
    assert_eq!(
        diagnostic_count(&runtime, "pane.focus.duplicate_ignored"),
        1
    );

    assert!(!runtime.expire_pending_view_focus(requested_at + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "pending"
    );
    finish_running_pane_focus(
        &mut runtime,
        Err("Herdr did not confirm pane focus: transport timed out"),
    );
    let timed_out = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the timeout is projected");
    assert_eq!(timed_out.phase, "failed");
    assert!(timed_out.retryable);
    assert!(
        timed_out
            .message
            .as_deref()
            .is_some_and(|message| message.contains("did not confirm"))
    );

    assert!(runtime.dispatch_json(&correlated_pane_focus_event(
        "w-order:t2:p",
        "relationship-2",
    )));
    let retry = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the retry replaces the settled receipt");
    assert_eq!(retry.request_id, "relationship-2");
    assert_eq!(retry.phase, "pending");
}

#[test]
fn pane_focus_refusal_and_target_retirement_end_the_matching_request() {
    let checkout_path = "/private/tmp/hide-pane-focus-retirement";
    let (mut runtime, _) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&correlated_pane_focus_event(
        "w-order:t2:p",
        "relationship-refused",
    )));
    finish_running_pane_focus(&mut runtime, Err("focus refused"));
    let refusal = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the refusal is projected");
    assert_eq!(refusal.phase, "failed");
    assert_eq!(refusal.message.as_deref(), Some("focus refused"));
    assert!(refusal.retryable);

    assert!(runtime.dispatch_json(&correlated_pane_focus_event(
        "w-order:t2:p",
        "relationship-retired",
    )));
    let remaining = ["w-order:t1"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &remaining,
        &remaining,
        "w-order:t1",
    )));
    let retirement = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("retirement is projected");
    assert_eq!(retirement.request_id, "relationship-retired");
    assert_eq!(retirement.phase, "failed");
    assert!(retirement.retryable);
    assert!(
        retirement
            .message
            .as_deref()
            .is_some_and(|message| message.contains("retired"))
    );
}

/// AC1, SC1. The strip's active mark and the canvas are the same field, so
/// asking for a tab moves both on the dispatch that asked, without waiting
/// for Herdr to answer.
#[test]
fn view_authority_a_tab_switch_moves_the_strip_and_the_canvas_in_one_snapshot() {
    let checkout_path = "/private/tmp/hide-view-authority-switch";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));

    let snapshot = runtime.snapshot();
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3"),
        "the strip's active mark moves on the frame the operator asked for"
    );
    assert_eq!(snapshot.tab.id.as_deref(), Some("w-order:t3"));
    assert_eq!(
        snapshot
            .active_pane_layout()
            .map(|layout| layout.tab_id.as_str()),
        Some("w-order:t3"),
        "the canvas is drawing the requested tab in the same snapshot"
    );
}

/// AC1, SC1. Herdr's next session update still names the tab the operator
/// left, because the notification has not landed. That is the answer to a
/// question already asked, not a new focus, so it must not pull the canvas
/// back.
#[test]
fn view_authority_a_stale_herdr_tab_does_not_undo_an_unconfirmed_switch() {
    let checkout_path = "/private/tmp/hide-view-authority-stale-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let mut herdr = HerdrMoves::new();
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));

    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));

    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3")
    );
    assert_eq!(
        diagnostic_count(&runtime, "tab.focus.followed"),
        0,
        "an unconfirmed notification is not an external focus"
    );

    // The confirmation ends the wait, and the value is unchanged by it.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t3"], herdr_on("w-order:t3"))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3")
    );
    assert!(sent_tab_moves(&runtime).is_empty());
}

/// Issue #413. The operator drops a tab into a new area (t2) and clicks back
/// into the first area (t1) before Herdr has answered the drop. Herdr's
/// session still names t1 from before the drop, which looks like the answer
/// to the click, and only then does Herdr's answer to the drop arrive. That
/// late t2 is Hide's own superseded request, not an outside focus: following
/// it moved the keyboard to t2's pane in the middle of typing into t1.
#[test]
fn view_authority_a_late_answer_to_a_superseded_switch_is_not_followed() {
    let checkout_path = "/private/tmp/hide-view-authority-superseded-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));

    // The session from before the drop, then each request's answer, with the
    // lane sending t1 once Herdr has answered t2.
    for (step, answer) in ["w-order:t1", "w-order:t2", "w-order:t1"]
        .into_iter()
        .enumerate()
    {
        if step > 0 {
            let sent = if step == 1 {
                "w-order:t2"
            } else {
                "w-order:t1"
            };
            runtime.complete_lane_tab(
                RemoteControlAction::FocusTab {
                    tab_id: sent.to_owned(),
                },
                Ok(RemoteControlOutcome::Acknowledged {
                    created_tab_id: None,
                    created_pane_id: None,
                }),
                3,
            );
        }
        // The first session is from before the drop: Herdr has moved nowhere.
        let moved: &[&str] = if step == 0 { &[] } else { &[answer] };
        runtime.ingest_session(Ok(herdr.after(moved, herdr_on(answer))));
        assert_eq!(
            checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
            Some("w-order:t1"),
            "Herdr naming {answer} does not move the tab the operator chose"
        );
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t1:p"),
            "the keyboard stays in the operator's pane while Herdr names {answer}"
        );
    }
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);

    // Once Herdr has answered both, a move it makes on its own is followed.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
}

/// Issue #594. Each Agent cycle commit is one pane focus, and a pane in
/// another tab moves Herdr's tab too. Herdr answers the focus at once, but
/// its session stream reports the tab moves afterwards, in order. Those are
/// the answers to Hide's own requests, not outside focus: following them
/// replayed every tab the operator passed through and left the screen on an
/// older one, which the shell then recorded as a visit.
#[test]
fn view_authority_a_pane_focus_s_late_tab_move_is_its_answer_not_an_outside_focus() {
    let checkout_path = "/private/tmp/hide-view-authority-pane-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    for tab in ["w-order:t3", "w-order:t1", "w-order:t2", "w-order:t1"] {
        assert!(runtime.dispatch_json(&operator_focus_event(&format!("{tab}:p"))));
        finish_running_pane_focus(&mut runtime, Ok(()));
    }

    for answer in ["w-order:t3", "w-order:t1", "w-order:t2", "w-order:t1"] {
        runtime.ingest_session(Ok(herdr.after(&[answer], herdr_on(answer))));
        assert_eq!(
            checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
            Some("w-order:t1"),
            "Herdr naming {answer} does not move the tab the operator chose last"
        );
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t1:p")
        );
    }
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
    assert!(
        sent_tab_moves(&runtime).is_empty(),
        "Herdr arriving answers every move"
    );

    // Once Herdr has answered, a tab it focuses on its own is followed.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t3"], herdr_on("w-order:t3"))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
}

/// Issue #629. Two tab clicks and an Agent cycle commit meet a slow Herdr:
/// t2 is on the wire, t1 waits on the lane behind it, and the commit's pane
/// focus replaces t1's wait with its own. A session from before the clicks,
/// still on t3, settles that wait, and t1 leaves anyway, because a pane
/// focus does not take a tab focus off the lane. Herdr's answer to t1 is
/// Hide's own: following it put the screen back on t1 and the shell
/// recorded t1's pane as a visit, so the next cycle started from it.
#[test]
fn view_authority_a_tab_focus_that_leaves_after_its_wait_was_replaced_is_superseded() {
    let checkout_path = "/private/tmp/hide-view-authority-left-replaced";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t3"))));
    let focus = |tab_id: &str| RemoteControlAction::FocusTab {
        tab_id: tab_id.to_owned(),
    };
    let acknowledged = || {
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        })
    };

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t3:p")));
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t3"))));
    // The lane sends t2, then t1, then the pane focus.
    runtime.complete_lane_tab(focus("w-order:t2"), acknowledged(), 3);
    runtime.complete_lane_tab(focus("w-order:t1"), acknowledged(), 3);
    finish_running_pane_focus(&mut runtime, Ok(()));

    for answer in ["w-order:t2", "w-order:t1", "w-order:t3"] {
        runtime.ingest_session(Ok(herdr.after(&[answer], herdr_on(answer))));
        assert_eq!(
            checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
            Some("w-order:t3"),
            "Herdr naming {answer} does not move the tab the operator chose last"
        );
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t3:p")
        );
    }
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
    assert!(sent_tab_moves(&runtime).is_empty());

    // Each answer was consumed, so a tab Herdr focuses on its own is followed.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
}

/// A pane focus still waiting on the lane is not confirmed by a session
/// from before it, even one whose layout already names its pane: Herdr is
/// about to move away for the tab focuses ahead of it, so the focus has to
/// leave. Taking that session as its answer left Herdr on t1 while Hide
/// showed t3 (#629).
#[test]
fn view_authority_a_pane_focus_that_has_not_left_is_not_confirmed_by_an_earlier_session() {
    let checkout_path = "/private/tmp/hide-view-authority-unsent-pane";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| Ok(tab_order_payload(checkout_path, &tabs, &tabs, tab));
    runtime.ingest_session(herdr_on("w-order:t3"));
    let focus = |tab_id: &str| RemoteControlAction::FocusTab {
        tab_id: tab_id.to_owned(),
    };
    let acknowledged = || {
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        })
    };

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t3:p")));
    runtime.ingest_session(herdr_on("w-order:t3"));
    runtime.complete_lane_tab(focus("w-order:t2"), acknowledged(), 3);
    runtime.complete_lane_tab(focus("w-order:t1"), acknowledged(), 3);
    assert_eq!(
        runtime
            .pane_focus_in_flight
            .as_ref()
            .map(|control| control.target_id.as_str()),
        Some("w-order:t3:p"),
        "the pane focus leaves after the tab focuses ahead of it"
    );
}

/// A tab focus waiting on the lane is not confirmed by Herdr moving there on
/// its own. Confirmed early, the request still left, and once it left it
/// was remembered as superseded, so Herdr's next move to that tab within
/// the deadline was consumed as its answer instead of followed.
#[test]
fn view_authority_a_tab_focus_that_has_not_left_is_not_confirmed_by_herdr_s_own_move() {
    let checkout_path = "/private/tmp/hide-view-authority-unsent-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    let focus = |tab_id: &str| RemoteControlAction::FocusTab {
        tab_id: tab_id.to_owned(),
    };
    let acknowledged = || {
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        })
    };

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));
    // Herdr applies t2, then moves to t3 on its own while t3 still waits.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));
    runtime.ingest_session(Ok(herdr.after(&["w-order:t3"], herdr_on("w-order:t3"))));
    runtime.complete_lane_tab(focus("w-order:t2"), acknowledged(), 3);
    runtime.complete_lane_tab(focus("w-order:t3"), acknowledged(), 3);

    for outside in ["w-order:t1", "w-order:t3"] {
        runtime.ingest_session(Ok(herdr.after(&[outside], herdr_on(outside))));
        assert_eq!(
            checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
            Some(outside),
            "Herdr moving to {outside} on its own is followed"
        );
    }
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 2);
}

/// Herdr publishes no event for focusing the tab it already shows, so a tab
/// focus that lands there is confirmed by Herdr's answer. Waiting for an
/// event held every move Herdr made on its own until the deadline.
#[test]
fn view_authority_a_tab_focus_on_the_tab_herdr_shows_is_confirmed_by_its_answer() {
    let checkout_path = "/private/tmp/hide-view-authority-shown-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| Ok(tab_order_payload(checkout_path, &tabs, &tabs, tab));
    runtime.ingest_session(herdr_on("w-order:t1"));

    // t2 is refused, so Herdr stays on t1, where t1 then lands.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    runtime.complete_lane_tab(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t2".to_owned(),
        },
        Err(live::ControlFailure::Definite("tab.focus rejected".into())),
        3,
    );
    runtime.complete_lane_tab(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t1".to_owned(),
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        }),
        3,
    );

    runtime.ingest_session(herdr_on("w-order:t3"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3"),
        "Herdr's own move is followed at once, not after the deadline"
    );
}

/// Issue #653. Quick clicks t2, t1, t2, t1, t2 meet Herdr events that trail
/// its answers. After t2 leaves, Hide still holds a session from before t1
/// was applied, which shows t2 from the first click; that is Herdr from
/// before both requests, not the answer to the second t2. Read as one, it
/// let the late t2 and t1 of the third and fourth clicks be followed as
/// Herdr's own moves, and the screen ended on t1 while the last click was
/// t2. At every step the screen is on the tab of the last click.
#[test]
fn view_authority_a_session_from_before_an_earlier_request_does_not_answer_a_later_one() {
    let checkout_path = "/private/tmp/hide-view-authority-aba";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    let answer = |runtime: &mut Runtime, tab_id: &str| {
        runtime.complete_lane_tab(
            RemoteControlAction::FocusTab {
                tab_id: tab_id.to_owned(),
            },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            }),
            3,
        );
    };
    let on_screen = |runtime: &Runtime| checkout_active_tab_id(runtime, &checkout_id);
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    answer(&mut runtime, "w-order:t2");
    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));

    // Second and third clicks: both answered, neither event in yet; a
    // session still from before the second arrives in between.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    answer(&mut runtime, "w-order:t1");
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t2"))));
    answer(&mut runtime, "w-order:t2");
    runtime.ingest_session(Ok(herdr.after(&["w-order:t1"], herdr_on("w-order:t1"))));
    assert_eq!(on_screen(&runtime).as_deref(), Some("w-order:t2"));

    // Fourth click, answered before the third click's event arrives.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1")));
    answer(&mut runtime, "w-order:t1");
    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));
    assert_eq!(
        on_screen(&runtime).as_deref(),
        Some("w-order:t1"),
        "the third click's late t2 is its answer, not Herdr's own move"
    );

    // Fifth click, then the fourth and fifth clicks' events.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    answer(&mut runtime, "w-order:t2");
    for event in ["w-order:t1", "w-order:t2"] {
        runtime.ingest_session(Ok(herdr.after(&[event], herdr_on(event))));
        assert_eq!(
            on_screen(&runtime).as_deref(),
            Some("w-order:t2"),
            "Herdr naming {event} leaves the screen on the last click"
        );
    }
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);

    // Every request is answered, so a move Herdr makes on its own is followed.
    runtime.ingest_session(Ok(herdr.after(&["w-order:t3"], herdr_on("w-order:t3"))));
    assert_eq!(on_screen(&runtime).as_deref(), Some("w-order:t3"));
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
}

/// The orders Herdr's answers and moves can arrive in, one row each. Herdr
/// applies Hide's tab focuses in the order they were sent and reports each
/// move in that order, so however its sessions fold or split the moves, the
/// screen is on the tab of the last request sent; once each is answered (by
/// its move, by an answer that moves nothing, by a refusal, or by the
/// deadline) a move Herdr makes on its own is followed.
#[test]
fn view_authority_the_screen_stays_on_the_last_request_whatever_order_herdr_answers_in() {
    #[derive(Debug)]
    enum Step {
        Click(&'static str),
        Answer(&'static str),
        Refuse(&'static str),
        /// A session published after Herdr made these moves, showing the
        /// last of them.
        Herdr(&'static [&'static str]),
        Deadline,
    }
    use Step::*;
    let cases: [(&str, &[Step]); 11] = [
        (
            "in order",
            &[
                Click("t2"),
                Answer("t2"),
                Herdr(&["t2"]),
                Click("t3"),
                Answer("t3"),
                Herdr(&["t3"]),
            ],
        ),
        (
            "two moves folded into one session",
            &[
                Click("t2"),
                Answer("t2"),
                Click("t3"),
                Answer("t3"),
                Herdr(&["t2", "t3"]),
            ],
        ),
        (
            "A to B to A folded into one session",
            &[
                Click("t2"),
                Answer("t2"),
                Click("t1"),
                Answer("t1"),
                Herdr(&["t2", "t1"]),
            ],
        ),
        (
            "t2, t3, t2 folded into one session",
            &[
                Click("t2"),
                Answer("t2"),
                Click("t3"),
                Answer("t3"),
                Click("t2"),
                Answer("t2"),
                Herdr(&["t2", "t3", "t2"]),
            ],
        ),
        (
            "t2, t3, t2 each in its own session after all three were answered",
            &[
                Click("t2"),
                Answer("t2"),
                Click("t3"),
                Answer("t3"),
                Click("t2"),
                Answer("t2"),
                Herdr(&["t2"]),
                Herdr(&["t3"]),
                Herdr(&["t2"]),
            ],
        ),
        // #660: an Agent drop (t2), a click back into the first area (t1), then
        // a click into the second (t2), each answered before Herdr reports the
        // move before it. Following the late t1 moved the keyboard out of the
        // second area in the middle of typing there.
        (
            "a click back and a click forward each answered before the move before it",
            &[
                Click("t2"),
                Answer("t2"),
                Click("t1"),
                Answer("t1"),
                Herdr(&["t2"]),
                Click("t2"),
                Answer("t2"),
                Herdr(&["t1"]),
                Herdr(&["t2"]),
            ],
        ),
        (
            "a burst's second request for the same tab moves nothing and settles on its answer",
            &[
                Click("t2"),
                Click("t2"),
                Answer("t2"),
                Answer("t2"),
                Herdr(&["t2"]),
            ],
        ),
        (
            "focusing the tab Herdr shows publishes no event",
            &[Click("t2"), Refuse("t2"), Click("t1"), Answer("t1")],
        ),
        (
            "the shown tab's answer comes after Herdr moved on its own",
            &[
                Click("t2"),
                Refuse("t2"),
                Click("t1"),
                Herdr(&["t3"]),
                Answer("t1"),
            ],
        ),
        (
            "refused while the next waits",
            &[
                Click("t2"),
                Click("t3"),
                Refuse("t2"),
                Answer("t3"),
                Herdr(&["t3"]),
            ],
        ),
        ("never reported", &[Click("t2"), Answer("t2"), Deadline]),
    ];
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    for (index, (name, steps)) in cases.into_iter().enumerate() {
        let checkout_path = format!("/private/tmp/hide-view-authority-order-{index}");
        let (mut runtime, checkout_id) = live_tab_order_runtime(&checkout_path);
        let mut herdr = HerdrMoves::new();
        let session = |tab: &str| tab_order_payload(&checkout_path, &tabs, &tabs, tab);
        let tab = |short: &str| format!("w-order:{short}");
        let control = |runtime: &mut Runtime, short: &str, result| {
            runtime.complete_lane_tab(
                RemoteControlAction::FocusTab { tab_id: tab(short) },
                result,
                3,
            );
        };
        runtime.ingest_session(Ok(herdr.after(&[], session("w-order:t1"))));
        let mut last_sent = tab("t1");
        for step in steps {
            match step {
                Click(short) => {
                    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, &tab(short))));
                    last_sent = tab(short);
                }
                Answer(short) => control(
                    &mut runtime,
                    short,
                    Ok(RemoteControlOutcome::Acknowledged {
                        created_tab_id: None,
                        created_pane_id: None,
                    }),
                ),
                Refuse(short) => control(
                    &mut runtime,
                    short,
                    Err(live::ControlFailure::Definite("tab.focus rejected".into())),
                ),
                Herdr(moves) => {
                    let moves = moves.iter().map(|short| tab(short)).collect::<Vec<_>>();
                    let moves = moves.iter().map(String::as_str).collect::<Vec<_>>();
                    let shown = *moves.last().expect("a session after at least one move");
                    runtime.ingest_session(Ok(herdr.after(&moves, session(shown))));
                }
                Deadline => {
                    let deadline = newest_sent_at(&runtime) + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS;
                    runtime.expire_pending_view_focus(deadline);
                }
            }
            assert_eq!(
                checkout_active_tab_id(&runtime, &checkout_id),
                Some(last_sent.clone()),
                "{name}: after {step:?} the screen is on the last request sent"
            );
        }
        assert!(
            sent_tab_moves(&runtime).is_empty(),
            "{name}: every request is answered"
        );
        assert_eq!(
            diagnostic_count(&runtime, "tab.focus.followed"),
            0,
            "{name}"
        );

        runtime.ingest_session(Ok(herdr.after(&["w-order:t4"], session("w-order:t4"))));
        assert_eq!(
            checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
            Some("w-order:t4"),
            "{name}: then a move Herdr makes on its own is followed"
        );
        assert_eq!(
            diagnostic_count(&runtime, "tab.focus.followed"),
            1,
            "{name}"
        );
    }
}

/// Herdr's moves between two sessions can be more than the replica keeps,
/// or come before a new replica after a reconnect. Then the tabs newly
/// active since the last session stand in for them, and the gap is
/// recorded; consuming only the moves kept would leave the dropped
/// requests waiting out the deadline. With no request waiting there is
/// nothing to stand in for, and a reconnect records no gap.
#[test]
fn view_authority_dropped_moves_fall_back_to_the_tabs_newly_active() {
    let checkout_path = "/private/tmp/hide-view-authority-moves-gap";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let answer = |runtime: &mut Runtime, tab_id: &str| {
        runtime.complete_lane_tab(
            RemoteControlAction::FocusTab {
                tab_id: tab_id.to_owned(),
            },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            }),
            3,
        );
    };
    let on_screen = |runtime: &Runtime| checkout_active_tab_id(runtime, &checkout_id);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));

    // t2's move is the oldest of more than the replica keeps.
    for tab in ["w-order:t2", "w-order:t3"] {
        assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, tab)));
        answer(&mut runtime, tab);
    }
    let mut moves = vec!["w-order:t2"];
    moves.extend(["w-order:t4"; TAB_FOCUS_LIMIT - 1]);
    moves.push("w-order:t3");
    let gaps = |records: &[serde_json::Value]| {
        records
            .iter()
            .filter(|record| record["kind"] == "tab.focus.moves_gap")
            .count()
    };
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.ingest_session(Ok(herdr.after(&moves, herdr_on("w-order:t3"))));
    });
    assert!(sent_tab_moves(&runtime).is_empty());
    assert_eq!(on_screen(&runtime).as_deref(), Some("w-order:t3"));
    assert_eq!(gaps(&records), 1, "{records:?}");

    // A new replica counts its moves from its own snapshot.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    answer(&mut runtime, "w-order:t2");
    let mut herdr = herdr.reconnected();
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t2"))));
    });
    assert!(sent_tab_moves(&runtime).is_empty());
    assert_eq!(on_screen(&runtime).as_deref(), Some("w-order:t2"));
    assert_eq!(gaps(&records), 1, "{records:?}");
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);

    runtime.ingest_session(Ok(herdr.after(&["w-order:t4"], herdr_on("w-order:t4"))));
    assert_eq!(on_screen(&runtime).as_deref(), Some("w-order:t4"));
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);

    let mut herdr = herdr.reconnected();
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t4"))));
    });
    assert_eq!(gaps(&records), 0, "{records:?}");
}

/// A snapshot read outside the event stream (a close's or a creation's
/// fresh projection, the editor's status answer) has no place in Herdr's
/// order and carries no moves: one that already shows the tab Hide asked
/// for answers nothing, and the stream's next session, carrying the move,
/// does.
#[test]
fn view_authority_a_snapshot_read_outside_the_stream_answers_no_request() {
    let checkout_path = "/private/tmp/hide-view-authority-raw-snapshot";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    let herdr_on = |tab: &str| tab_order_payload(checkout_path, &tabs, &tabs, tab);
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(&[], herdr_on("w-order:t1"))));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    runtime.complete_lane_tab(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t2".to_owned(),
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: None,
        }),
        3,
    );
    assert_eq!(sent_tab_moves(&runtime), ["w-order:t2"]);

    let read_outside = herdr_on("w-order:t2");
    assert!(read_outside.tab_moves.is_none());
    runtime.ingest_session(Ok(read_outside));
    assert_eq!(
        sent_tab_moves(&runtime),
        ["w-order:t2"],
        "a snapshot outside the stream answers nothing"
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );

    runtime.ingest_session(Ok(herdr.after(&["w-order:t2"], herdr_on("w-order:t2"))));
    assert!(sent_tab_moves(&runtime).is_empty());
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
}

/// The tab wait a pane focus arms always ends: Herdr refusing the focus
/// moves no tab, a focus in the tab Herdr already shows arms nothing, and a
/// tab move Herdr never reports runs out with a tab focus's deadline. After
/// each, a tab Herdr focuses on its own is followed.
#[test]
fn view_authority_a_pane_focus_tab_wait_ends_when_no_tab_move_will_arrive() {
    let checkout_path = "/private/tmp/hide-view-authority-pane-tab-ends";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));

    // Refused: Herdr stays on t1, so nothing of this focus will arrive.
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    assert_eq!(
        sent_tab_moves(&runtime),
        ["w-order:t2"],
        "a focus in another tab moves Herdr's tab"
    );
    finish_running_pane_focus(&mut runtime, Err("pane focus refused"));
    assert!(sent_tab_moves(&runtime).is_empty());

    // Herdr already shows t1, so the answer to focusing its pane is all of it.
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert!(sent_tab_moves(&runtime).is_empty());
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t3",
    )));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);

    // Never reported: the wait runs out at the tab focus deadline.
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    finish_running_pane_focus(&mut runtime, Ok(()));
    let requested_at = newest_sent_at(&runtime);
    assert!(runtime.expire_pending_view_focus(requested_at + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS));
    assert!(sent_tab_moves(&runtime).is_empty());
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 2);
}

/// The sent tab moves are bounded in count and in time, and one Herdr
/// refused is not waited on; each of those ends the claim, so Herdr then
/// focusing that tab is followed. A request whose result was lost may have
/// been applied, so it keeps its claim.
#[test]
fn view_authority_sent_tab_moves_stop_claiming_their_tab_when_capped_expired_or_refused() {
    let checkout_path = "/private/tmp/hide-view-authority-sent-bounds";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = (1..=20)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = tabs.iter().map(String::as_str).collect::<Vec<_>>();
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t20",
    )));

    // Each request is answered before the next leaves, and none of their
    // moves has arrived.
    for (index, tab) in tabs[..=TAB_FOCUS_LIMIT + 1].iter().enumerate() {
        assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, tab)));
        if index > 0 {
            runtime.complete_lane_tab(
                RemoteControlAction::FocusTab {
                    tab_id: tabs[index - 1].to_owned(),
                },
                Ok(RemoteControlOutcome::Acknowledged {
                    created_tab_id: None,
                    created_pane_id: None,
                }),
                3,
            );
        }
    }
    assert_eq!(sent_tab_moves(&runtime).len(), TAB_FOCUS_LIMIT);
    assert!(!sent_tab_moves(&runtime).contains(&"w-order:t1".to_owned()));
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);

    runtime.ingest_local_control_result(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t4".to_owned(),
        },
        Err("tab.focus rejected".to_owned()),
        3,
    );
    runtime.ingest_local_control_failure(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t5".to_owned(),
        },
        Err(live::ControlFailure::Ambiguous(
            "tab.focus result is unknown".into(),
        )),
        3,
    );
    assert!(!sent_tab_moves(&runtime).contains(&"w-order:t4".to_owned()));
    assert!(sent_tab_moves(&runtime).contains(&"w-order:t5".to_owned()));

    // The newest request is the youngest, so its deadline is past every one.
    let requested_at = newest_sent_at(&runtime);
    runtime.expire_pending_view_focus(requested_at + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS);
    assert!(sent_tab_moves(&runtime).is_empty());

    // With every claim gone, Herdr focusing t2 is an outside move again.
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
}

/// AC1, R1. With nothing in flight, a Herdr session naming another tab is
/// somebody focusing that tab outside Hide. Hide follows it and says so.
#[test]
fn view_authority_an_external_tab_focus_is_followed_and_reported() {
    let checkout_path = "/private/tmp/hide-view-authority-external-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(sent_tab_moves(&runtime).is_empty());

    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2"
    ))));

    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
    assert_eq!(
        runtime.snapshot().tab.id.as_deref(),
        Some("w-order:t2"),
        "the canvas follows the tab, not only the strip"
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p"),
        "and the keyboard lands in that tab rather than staying on a pane nobody can see"
    );
}

/// AC1, SC1. Registering a device rebuilds the catalog from scratch, and a
/// freshly built checkout names no active tab. The visible tab is Hide's,
/// so it survives a rebuild Herdr had no part in.
#[test]
fn view_authority_a_catalog_rebuild_keeps_the_visible_tab() {
    let checkout_path = "/private/tmp/hide-view-authority-rebuild";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));

    let register_device = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "register_device",
        "payload": {
            "id": "device-rebuild",
            "label": "Rebuild",
            "ssh_alias": "rebuild-host"
        }
    }))
    .expect("register device event");
    assert!(runtime.dispatch_json(&register_device));

    // The rebuilt catalog carries no tabs until the next session update,
    // which is what makes this the interesting moment: the tab the
    // operator chose has to survive the gap. It survives because a
    // rebuild is not a reconcile - the catalog says nothing about which
    // tab is visible, so nothing on this path may read it as Herdr
    // naming another one.
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));

    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3"),
        "a rebuild is not Herdr moving the tab"
    );
    assert_eq!(
        runtime.snapshot().tab.id.as_deref(),
        Some("w-order:t3"),
        "and the canvas comes back on the tab it was showing"
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
}

/// AC2, R1. Herdr refusing the notification does not move the operator's
/// screen. The tab stays where they put it and the refusal is reported.
#[test]
fn view_authority_a_refused_tab_focus_keeps_the_tab_and_reports_it() {
    let checkout_path = "/private/tmp/hide-view-authority-refused-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));
    assert!(!sent_tab_moves(&runtime).is_empty());

    runtime.ingest_local_control_result(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t3".to_owned(),
        },
        Err("tab.focus rejected".to_owned()),
        12,
    );

    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3"),
        "a refusal is reported, not acted on by moving the screen"
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.refused"), 1);
    assert!(sent_tab_moves(&runtime).is_empty());

    // A refusal that arrives after its request's deadline is still the
    // answer to the last thing asked, so it is reported too.
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    // This fixture's lane is moved on by hand, as Herdr's answer would.
    let _ = runtime.advance_lane();
    let deadline = newest_sent_at(&runtime) + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS;
    runtime.expire_pending_view_focus(deadline);
    assert!(sent_tab_moves(&runtime).is_empty());
    runtime.complete_lane_tab(
        RemoteControlAction::FocusTab {
            tab_id: "w-order:t2".to_owned(),
        },
        Err(live::ControlFailure::Definite("tab.focus rejected".into())),
        12,
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.refused"), 2);
}

/// AC2, R1. A notification Herdr never answers stops being pending, the
/// value Hide chose is kept, and the silence is reported rather than
/// waited on forever.
#[test]
fn view_authority_an_unanswered_notification_times_out_and_keeps_its_value() {
    let checkout_path = "/private/tmp/hide-view-authority-timeout";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t3")));
    let requested_at = newest_sent_at(&runtime);

    assert!(!runtime.expire_pending_view_focus(requested_at + 1));
    assert!(!sent_tab_moves(&runtime).is_empty());
    assert!(runtime.expire_pending_view_focus(requested_at + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS));

    assert!(sent_tab_moves(&runtime).is_empty());
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3"),
        "a silent Herdr is a reason to report, not to move the screen"
    );
    assert_eq!(diagnostic_count(&runtime, "view_focus.timed_out"), 1);
}

/// Two Herdr workspaces at one path share one checkout, and the second
/// one in payload order names a different active tab. A tab focus on the
/// first workspace is answered by Herdr's move to that tab, whichever
/// workspace the session says holds Herdr's keyboard.
#[test]
fn split_checkout_a_tab_focus_is_confirmed_by_the_workspace_that_owns_the_tab() {
    let checkout_path = "/private/tmp/hide-split-checkout-confirm";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let before: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t1"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(
        herdr.after(&[], split_checkout_payload(checkout_path, &before, "wb"))
    ));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wb:t1"),
        "with no tab of its own Hide shows the tab Herdr has focused"
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wb:t1:p"),
        "and the keyboard is in that tab"
    );

    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "wa:t2")));
    assert!(!sent_tab_moves(&runtime).is_empty());
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wa:t2:p")
    );

    // Herdr shows the tab in its workspace; its keyboard stays in wb.
    let confirmed: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t2"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(herdr.after(
        &["wa:t2"],
        split_checkout_payload(checkout_path, &confirmed, "wb"),
    )));

    assert!(
        sent_tab_moves(&runtime).is_empty(),
        "the notification is confirmed"
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
    assert_eq!(diagnostic_count(&runtime, "view_focus.timed_out"), 0);
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.tab_id.as_str()),
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        "the tab drawn is the tab attached"
    );
}

/// The active tab of a workspace Herdr's keyboard is not in is that
/// workspace's memory, not a focus. It never moves the canvas; a move of
/// Herdr's keyboard into that workspace does.
#[test]
fn split_checkout_another_workspaces_active_tab_is_not_followed_until_herdr_focuses_it() {
    let checkout_path = "/private/tmp/hide-split-checkout-follow";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let start: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1"], "wa:t1"),
        ("wb", &["wb:t1", "wb:t2"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &start, "wa")));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t1")
    );

    // wb remembers another tab; Herdr's keyboard is still in wa.
    let wb_moved: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1"], "wa:t1"),
        ("wb", &["wb:t1", "wb:t2"], "wb:t2"),
    ];
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &wb_moved, "wa")));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t1"),
        "a non-focused workspace's active tab is not followed"
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);

    // Herdr's keyboard moves into wb: that is a focus, and it is followed.
    assert!(runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &wb_moved, "wb"))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wb:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 1);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wb:t2:p"),
        "the keyboard follows the tab"
    );
}

/// A restore keeps the persisted pane and draws the tab that holds it,
/// not the tab Herdr's last workspace happens to name. The canvas drawn
/// is the canvas attached.
#[test]
fn split_checkout_a_restore_draws_the_tab_holding_the_persisted_pane() {
    let checkout_path = "/private/tmp/hide-split-checkout-restore";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    runtime.snapshot.terminal.pane_id = Some("wa:t2:p".to_owned());
    runtime.snapshot.focused.pane_id = Some("wa:t2:p".to_owned());
    runtime.snapshot.ui_state.selected_pane_id = Some("wa:t2:p".to_owned());
    let session: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t1"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &session, "wb")));

    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wa:t2:p"),
        "the persisted pane survives the first session"
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t2"),
        "the visible tab is the one holding that pane"
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.tab_id.as_str()),
        Some("wa:t2"),
        "and it is the layout attached"
    );
}

/// A timed-out notification keeps Hide's tab through the reconcile that
/// follows it: Herdr's focus has not moved, so there is nothing to
/// follow.
#[test]
fn split_checkout_an_expired_notification_keeps_its_tab_through_the_next_reconcile() {
    let checkout_path = "/private/tmp/hide-split-checkout-expiry";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let session: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t1"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &session, "wa")));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "wa:t2")));
    let requested_at = newest_sent_at(&runtime);
    assert!(runtime.expire_pending_view_focus(requested_at + VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS));
    assert!(sent_tab_moves(&runtime).is_empty());

    // Herdr never answered; its focus is where it was.
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &session, "wa")));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t2"),
        "a silent Herdr is a reason to report, not to move the screen"
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
    assert_eq!(diagnostic_count(&runtime, "view_focus.timed_out"), 1);
}

/// A created tab is the visible tab from Herdr's acknowledgment, and the
/// `tab_focused` that follows confirms it rather than being followed.
#[test]
fn split_checkout_a_created_tab_is_visible_on_the_acknowledgment() {
    let checkout_path = "/private/tmp/hide-split-checkout-create";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let before: [(&str, &[&str], &str); 2] =
        [("wa", &["wa:t1"], "wa:t1"), ("wb", &["wb:t1"], "wb:t1")];
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(
        herdr.after(&[], split_checkout_payload(checkout_path, &before, "wa"))
    ));

    runtime.ingest_local_control_result(
        RemoteControlAction::CreateTab {
            workspace_id: "wa".to_owned(),
            cwd: checkout_path.to_owned(),
            label: "2".to_owned(),
            area_id: None,
            group_id: None,
            admission_id: None,
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("wa:t2".to_owned()),
            created_pane_id: Some("wa:t2:p".to_owned()),
        }),
        5,
    );
    assert_eq!(
        runtime
            .visible_tab_ids
            .get(&checkout_id)
            .map(String::as_str),
        Some("wa:t2"),
        "the created tab is Hide's visible tab before Herdr lists it"
    );
    assert!(!sent_tab_moves(&runtime).is_empty());
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("wa:t2:p")
    );

    let after: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t2"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(herdr.after(
        &["wa:t2"],
        split_checkout_payload(checkout_path, &after, "wa"),
    )));
    assert!(sent_tab_moves(&runtime).is_empty());
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t2")
    );
    assert_eq!(diagnostic_count(&runtime, "tab.focus.followed"), 0);
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.tab_id.as_str()),
        Some("wa:t2")
    );
}

/// Focusing a pane in a tab the checkout is not showing brings that tab
/// forward in the same dispatch, so the keyboard never lands on a pane
/// nobody can see.
#[test]
fn split_checkout_a_pane_focus_in_a_hidden_tab_brings_the_tab_forward() {
    let checkout_path = "/private/tmp/hide-split-checkout-align";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let session: [(&str, &[&str], &str); 2] = [
        ("wa", &["wa:t1", "wa:t2"], "wa:t1"),
        ("wb", &["wb:t1"], "wb:t1"),
    ];
    runtime.ingest_session(Ok(split_checkout_payload(checkout_path, &session, "wa")));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wa:t1")
    );

    assert!(runtime.dispatch_json(&operator_focus_event("wb:t1:p")));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("wb:t1"),
        "the tab holding the focused pane is visible on the same dispatch"
    );
    assert_eq!(diagnostic_count(&runtime, "tab.visible_aligned"), 1);
    assert_eq!(
        runtime.snapshot().ui_state.selected_pane_id.as_deref(),
        Some("wb:t1:p"),
        "the persisted selection follows the ring"
    );
}

/// Closing the visible tab lands on the tab Herdr moved to. Three tabs;
/// the operator is on the third and closes it. The snapshot that follows
/// names the second tab active and no focused pane yet, and the keyboard
/// goes to that tab's pane, not to the checkout's first.
#[test]
fn closing_the_visible_tab_lands_on_the_tab_herdr_moved_to() {
    let checkout_path = "/private/tmp/hide-close-lands-on-herdr-tab";
    let (mut runtime, checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t3",
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t3:p")));
    assert_eq!(
        runtime
            .visible_tab_ids
            .get(&checkout_id)
            .map(String::as_str),
        Some("w-order:t3")
    );

    let remaining = ["w-order:t1", "w-order:t2"];
    let mut after_close = tab_order_payload(checkout_path, &remaining, &remaining, "w-order:t2");
    after_close.focused_pane_id = None;
    runtime.ingest_session(Ok(after_close));

    assert_eq!(
        runtime.snapshot.ui_state.selected_pane_id.as_deref(),
        Some("w-order:t2:p"),
        "the keyboard follows Herdr to the tab it focused after the close"
    );
    assert_eq!(
        runtime
            .visible_tab_ids
            .get(&checkout_id)
            .map(String::as_str),
        Some("w-order:t2"),
        "the strip draws the tab Herdr moved to"
    );
}

/// AC1, AC7, SC3. The focus ring moves on the click, and the layout that
/// was already on its way carrying the old focus does not take it back.
/// The read record stays on the clicked pane through that arrival.
#[test]
fn view_authority_a_pane_click_moves_focus_and_a_stale_layout_does_not_undo_it() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));
    assert_eq!(
        runtime.snapshot().focused.pane_id.as_deref(),
        Some("w1:p3"),
        "the ring moves on the click, not on Herdr's confirming event"
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p3")
    );

    // Herdr's in-flight frame still names the pane the tab came forward
    // with. Its geometry is taken; its focus is not.
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    assert_eq!(
        unread_panes(&runtime),
        vec!["w1:p1", "w1:p2"],
        "the clicked row stays read through the stale arrival"
    );
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
}

#[test]
fn pane_focus_burst_keeps_only_the_latest_successor_and_resumes_external_follow() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    let first = runtime.pane_focus_in_flight.clone().unwrap();
    for _ in 0..100 {
        runtime.dispatch_json(&operator_focus_event("w1:p1"));
        runtime.dispatch_json(&operator_focus_event("w1:p3"));
    }
    assert_eq!(runtime.pane_focus_in_flight.as_ref(), Some(&first));
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime.pane_focus_in_flight.as_ref().unwrap().target_id,
        "w1:p3"
    );
    assert!(runtime.pending_pane_focus.is_some());
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert!(runtime.pane_focus_in_flight.is_none());
    assert!(runtime.pending_pane_focus.is_none());
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 1);
}

#[test]
fn pane_focus_rejects_a_stale_stream_after_confirmation_and_follows_a_verified_external_move() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    finish_running_pane_focus(&mut runtime, Ok(()));

    let stale = finished_tab_payload(&panes, "w1:p1");
    let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale.clone()), None);
    let identity = identity.unwrap();
    // The update is ingested at once and Herdr's differing focus waits.
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    let (_, next) = runtime.finish_pane_focus_readback(identity, FocusReadback::Elsewhere);
    assert!(next.is_none());
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p2")
    );
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
    assert!(runtime.pending_pane_focus.is_none());
    assert!(runtime.pane_focus_readback.is_none());

    // The same proposed move becomes external authority when Herdr's fresh
    // layout and active tab really agree, without any new operator request.
    let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale), None);
    runtime.finish_pane_focus_readback(identity.unwrap(), FocusReadback::Confirmed);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p1")
    );
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 1);
}

/// Every `pane.focus.stream_readback_failed` line an installed build wrote
/// on 2026-10-14 had one shape: Herdr answered a readback of the stream's
/// older focus with the pane the operator had just focused. That is the
/// readback refuting a stale layout, not a failure. Only a readback Herdr
/// could not answer while it was still the latest is one, logged with
/// Herdr's reason; one a newer focus overtook ends quietly.
#[test]
fn only_the_latest_readback_herdr_could_not_answer_is_a_failure_line() {
    let failures = |records: &[serde_json::Value]| {
        records
            .iter()
            .filter(|record| record["kind"] == "pane.focus.stream_readback_failed")
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    finish_running_pane_focus(&mut runtime, Ok(()));
    let stale = finished_tab_payload(&panes, "w1:p1");

    // Herdr already shows the operator's own pane.
    let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale.clone()), None);
    let ((_, next), records) = crate::diagnostics::capture(|| {
        runtime.finish_pane_focus_readback(identity.unwrap(), FocusReadback::Elsewhere)
    });
    assert!(next.is_none());
    assert_eq!(failures(&records), Vec::<serde_json::Value>::new());

    // The operator focused another pane before Herdr answered at all.
    let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale.clone()), None);
    runtime.dispatch_json(&operator_focus_event("w1:p3"));
    finish_running_pane_focus(&mut runtime, Ok(()));
    let ((_, next), records) = crate::diagnostics::capture(|| {
        runtime.finish_pane_focus_readback(
            identity.unwrap(),
            FocusReadback::Unread("pane.layout failed: timed out".into()),
        )
    });
    assert_eq!(failures(&records), Vec::<serde_json::Value>::new());
    if let Some(next) = next {
        runtime.finish_pane_focus_readback(next, FocusReadback::Elsewhere);
    }

    // The latest readback, unanswered.
    let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale), None);
    let (_, records) = crate::diagnostics::capture(|| {
        runtime.finish_pane_focus_readback(
            identity.unwrap(),
            FocusReadback::Unread("pane.layout failed: Herdr is not running".into()),
        )
    });
    let failures = failures(&records);
    assert_eq!(failures.len(), 1, "{records:?}");
    assert_eq!(failures[0]["pane_id"], "w1:p1");
    assert_eq!(
        failures[0]["message"],
        "pane.layout failed: Herdr is not running"
    );
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    assert!(runtime.pane_focus_readback.is_none());
}

#[test]
fn pane_focus_readback_cannot_outlive_a_new_selection_or_connection() {
    for reconnect in [false, true] {
        let mut runtime = live_runtime();
        let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
        runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
        runtime.dispatch_json(&operator_focus_event("w1:p2"));
        finish_running_pane_focus(&mut runtime, Ok(()));
        let stale = finished_tab_payload(&panes, "w1:p1");
        let (_, identity) = runtime.ingest_session_awaiting_focus_readback(Ok(stale), None);
        let identity = identity.unwrap();
        let expected = if reconnect {
            runtime.set_live(runtime.live.as_ref().unwrap().clone());
            "w1:p2"
        } else {
            runtime.dispatch_json(&operator_focus_event("w1:p3"));
            finish_running_pane_focus(&mut runtime, Ok(()));
            "w1:p3"
        };

        // The old answer moves nothing. A read handed on instead is for the
        // current selection and connection, and Herdr's answer to that one
        // is the one that counts.
        let (_, next) =
            runtime.finish_pane_focus_readback(identity.clone(), FocusReadback::Confirmed);
        assert_eq!(
            runtime.snapshot().focused.pane_id.as_deref(),
            Some(expected)
        );
        assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
        if let Some(next) = next {
            assert_ne!(next, identity);
            runtime.finish_pane_focus_readback(next, FocusReadback::Elsewhere);
        }
        assert!(runtime.pane_focus_readback.is_none());
    }
}

/// Two tabs: `t1` holds three panes side by side, `t2` one.
fn two_tab_focus_payload(t1_focused: &str) -> SessionSnapshotPayload {
    let pane_rect = |pane_id: &str, x: u32, width: u32| serde_json::json!({"pane_id": pane_id, "rect": {"x": x, "y": 0, "width": width, "height": 24}});
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "tabs": [
            {"workspace_id": "w1", "tab_id": "t1", "label": ""},
            {"workspace_id": "w1", "tab_id": "t2", "label": ""}
        ],
        "layouts": [{
            "workspace_id": "w1", "tab_id": "t1", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 90, "height": 24},
            "focused_pane_id": t1_focused,
            "panes": [pane_rect("w1:p1", 0, 30), pane_rect("w1:p2", 30, 30), pane_rect("w1:p3", 60, 30)],
            "splits": [
                {"direction": "right", "ratio": 0.333_333_34,
                 "rect": {"x": 0, "y": 0, "width": 90, "height": 24}},
                {"direction": "right", "ratio": 0.5,
                 "rect": {"x": 30, "y": 0, "width": 60, "height": 24}}
            ]
        }, {
            "workspace_id": "w1", "tab_id": "t2", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 90, "height": 24},
            "focused_pane_id": "w1:p4",
            "panes": [pane_rect("w1:p4", 0, 90)],
            "splits": []
        }]
    }))
    .expect("session payload")
}

#[test]
fn a_pane_focus_readback_for_another_tab_cannot_pull_focus_back() {
    let mut runtime = live_runtime();
    runtime.ingest_session(Ok(two_tab_focus_payload("w1:p1")));
    // Herdr's stream moves tab t1's focus while Hide has p1 selected.
    let (_, identity) =
        runtime.ingest_session_awaiting_focus_readback(Ok(two_tab_focus_payload("w1:p2")), None);
    let identity = identity.unwrap();
    // The operator selects the pane of the other tab during the read.
    runtime.dispatch_json(&operator_focus_event("w1:p4"));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p4"));

    runtime.finish_pane_focus_readback(identity, FocusReadback::Confirmed);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p4"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
}

#[test]
fn pane_focus_completion_during_session_read_does_not_authorize_an_unchecked_stream_move() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    let stale = finished_tab_payload(&panes, "w1:p1");
    let (_, readback) = runtime.ingest_session_awaiting_focus_readback(Ok(stale), None);
    assert!(readback.is_none());
    finish_running_pane_focus(&mut runtime, Ok(()));

    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
}

#[test]
fn only_one_pane_focus_readback_is_outstanding_and_a_superseded_one_hands_on() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    let (_, first) = runtime
        .ingest_session_awaiting_focus_readback(Ok(finished_tab_payload(&panes, "w1:p2")), None);
    let first = first.unwrap();
    assert_eq!(first.target_id, "w1:p2");
    // Herdr moved again while the first read is out: no second read starts.
    let (_, second) = runtime
        .ingest_session_awaiting_focus_readback(Ok(finished_tab_payload(&panes, "w1:p3")), None);
    assert!(second.is_none());
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));

    // The answer is for a focus the layout no longer shows, so it covers
    // nothing and the worker is handed the one the layout does show.
    let (_, next) = runtime.finish_pane_focus_readback(first, FocusReadback::Confirmed);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    let next = next.unwrap();
    assert_eq!(next.target_id, "w1:p3");
    runtime.finish_pane_focus_readback(next, FocusReadback::Confirmed);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    assert!(runtime.pane_focus_readback.is_none());
}

#[test]
fn a_move_that_arrives_during_an_unconfirmed_readback_is_read_again() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    // A late layout from an earlier burst shows p2 and starts a read.
    let (_, stale) = runtime
        .ingest_session_awaiting_focus_readback(Ok(finished_tab_payload(&panes, "w1:p2")), None);
    let stale = stale.unwrap();
    // Herdr really moves to p2 while that read is out: the layout looks the
    // same, and the one slot is taken.
    let (_, moved) = runtime
        .ingest_session_awaiting_focus_readback(Ok(finished_tab_payload(&panes, "w1:p2")), None);
    assert!(moved.is_none());

    // The read was answered before the move, so it does not confirm. The
    // move came after the read began, so it is read again rather than lost.
    let (_, next) = runtime.finish_pane_focus_readback(stale, FocusReadback::Elsewhere);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    let next = next.expect("the move that arrived during the read is read again");
    assert_eq!(next.target_id, "w1:p2");
    runtime.finish_pane_focus_readback(next, FocusReadback::Confirmed);
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 1);
    assert!(runtime.pane_focus_readback.is_none());
}

#[test]
fn superseded_same_pane_focus_failure_cannot_fail_a_newer_correlated_request() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&correlated_pane_focus_event("w1:p2", "first"));
    let first = runtime.pane_focus_in_flight.clone().unwrap();
    runtime.dispatch_json(&operator_focus_event("w1:p1"));
    runtime.dispatch_json(&correlated_pane_focus_event("w1:p2", "latest"));
    finish_running_pane_focus(&mut runtime, Err("older request refused"));
    let request = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .unwrap();
    assert_eq!(request.request_id, "latest");
    assert_eq!(request.phase, "pending");
    assert!(runtime.snapshot().status.last_error.is_none());
    assert!(!runtime.ingest_pane_focus_completion(
        first,
        Err("late duplicate".to_owned().into()),
        9
    ));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "succeeded"
    );
}

#[test]
fn unknown_pane_focus_ends_the_automatic_burst_without_claiming_success() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    let first = runtime.pane_focus_in_flight.clone().unwrap();
    runtime.dispatch_json(&correlated_pane_focus_event("w1:p1", "latest"));
    let (changed, successor) = runtime.complete_lane_pane_focus(
        first,
        Err(live::ControlFailure::Ambiguous(
            "pane.focus result is unknown: response timed out".into(),
        )),
        5_000,
    );
    assert!(changed);
    assert!(
        successor.is_none(),
        "an unknown effect cannot release its queue"
    );
    assert!(runtime.pane_focus_in_flight.is_none());
    assert!(runtime.pending_pane_focus.is_none());
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    let request = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .unwrap();
    assert_eq!(request.request_id, "latest");
    assert_eq!(request.phase, "failed");
    assert!(request.retryable);
    assert!(request.message.as_ref().unwrap().contains("unknown"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.unknown"), 1);
    assert_eq!(diagnostic_count(&runtime, "pane.focus"), 0);
    assert_eq!(
        runtime.snapshot().status.last_error.as_ref().unwrap().kind,
        "pane.focus_unknown"
    );

    // Ending the wait cannot cancel the old mutation. A later external move
    // is followed, and an explicit new selection begins a new burst.
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p2")));
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    runtime.dispatch_json(&correlated_pane_focus_event("w1:p1", "retry"));
    assert_eq!(
        runtime.pane_focus_in_flight.as_ref().unwrap().target_id,
        "w1:p1"
    );
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "succeeded"
    );
}

#[test]
fn old_connection_focus_completion_runs_the_latest_intent_on_the_new_connection() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.dispatch_json(&correlated_pane_focus_event("w1:p2", "reconnected"));
    let old_generation = runtime
        .pane_focus_in_flight
        .as_ref()
        .unwrap()
        .live_generation;
    runtime.set_live(runtime.live.as_ref().unwrap().clone());
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_ne!(
        runtime
            .pane_focus_in_flight
            .as_ref()
            .unwrap()
            .live_generation,
        old_generation
    );
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "pending"
    );
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "succeeded"
    );
}

#[test]
fn latest_pane_focus_is_not_released_by_a_layout_while_an_older_request_is_running() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    runtime.dispatch_json(&operator_focus_event("w1:p2"));
    runtime.dispatch_json(&operator_focus_event("w1:p1"));
    // The newer response/event can arrive before the older socket request.
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p2")));

    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p1"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 0);
}

/// AC1, AC8, R1. With nothing in flight, a Herdr layout naming another
/// pane is a focus made outside Hide. Hide follows it and reports the
/// panes and where the change came from.
#[test]
fn view_authority_an_external_pane_focus_is_followed_and_reported() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert!(runtime.pending_pane_focus.is_none());

    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p2")));

    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p2"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.followed"), 1);
}

/// AC2, R1. A refused pane focus leaves the keyboard where the operator
/// put it and reports the refusal.
#[test]
fn view_authority_a_refused_pane_focus_keeps_the_pane_and_reports_it() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));
    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));

    finish_running_pane_focus(&mut runtime, Err("pane.focus rejected"));

    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    assert_eq!(diagnostic_count(&runtime, "pane.focus.refused"), 1);
    assert!(runtime.pending_pane_focus.is_none());
}

/// Rule 11. Reaching for the pane that already has the keyboard converges
/// on the state it is already in and sends Herdr nothing a second time.
/// The look itself still counts, because clicking the pane you are on is
/// still looking at it.
#[test]
fn view_authority_repeating_a_focus_sends_no_second_notification() {
    let mut runtime = live_runtime();
    let panes = [("w1:p1", 6018_u64), ("w1:p2", 6019), ("w1:p3", 6020)];
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p1")));

    assert!(runtime.dispatch_json(&operator_focus_event("w1:p3")));
    runtime.ingest_session(Ok(finished_tab_payload(&panes, "w1:p3")));
    finish_running_pane_focus(&mut runtime, Ok(()));
    assert!(runtime.pending_pane_focus.is_none());
    let notifications = diagnostic_count(&runtime, "pane.focus.requested");

    runtime.dispatch_json(&operator_focus_event("w1:p3"));

    assert_eq!(
        diagnostic_count(&runtime, "pane.focus.requested"),
        notifications,
        "no second request leaves for a pane Herdr has already focused"
    );
    assert_eq!(runtime.snapshot().focused.pane_id.as_deref(), Some("w1:p3"));
    assert_eq!(unread_panes(&runtime), vec!["w1:p1", "w1:p2"]);
}

#[test]
fn tab_order_follows_herdr_and_not_layout_arrival() {
    let checkout_path = "/private/tmp/hide-tab-order-arrival";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    // Herdr reports t1, t2, t3; the layouts arrive in the order the tabs
    // were first drawn, which is the order the navigator used to take.
    let payload = tab_order_payload(
        checkout_path,
        &["w-order:t1", "w-order:t2", "w-order:t3"],
        &["w-order:t3", "w-order:t1", "w-order:t2"],
        "w-order:t1",
    );
    assert!(runtime.ingest_session(Ok(payload)));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        vec![
            "w-order:t1".to_owned(),
            "w-order:t2".to_owned(),
            "w-order:t3".to_owned()
        ]
    );
}

#[test]
fn tab_order_follows_a_move_and_ignores_a_layout_redraw() {
    let checkout_path = "/private/tmp/hide-tab-order-moved";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));

    // Herdr moved the second tab in front of the first.
    let moved = ["w-order:t2", "w-order:t1", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &moved,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        moved.map(str::to_owned).to_vec()
    );

    // A layout redraw for an existing tab is detail, not order.
    assert!(!runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &moved,
        &["w-order:t3", "w-order:t2", "w-order:t1"],
        "w-order:t1"
    ))));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        moved.map(str::to_owned).to_vec()
    );
}

#[test]
fn herdr_active_tab_names_the_projected_tab() {
    let checkout_path = "/private/tmp/hide-tab-order-active";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2"
    ))));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(
        runtime.snapshot().tab.id.as_deref(),
        Some("w-order:t2"),
        "the active tab projection follows the id Herdr named"
    );
}

#[test]
fn herdr_active_tab_unresolved_is_reported_not_replaced() {
    let checkout_path = "/private/tmp/hide-tab-order-unplaceable";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t9"
    ))));

    // Hide owns the visible tab, so a checkout that has tabs shows one of
    // them. What Herdr named is still reported, because a name that
    // matches no tab in the session is worth knowing about.
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(
        runtime.snapshot().tab.id.as_deref(),
        Some("w-order:t1"),
        "a checkout with tabs never draws the empty-checkout state"
    );
    let unresolved = runtime
        .snapshot()
        .status
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "tab.active_unresolved")
        .count();
    assert_eq!(unresolved, 1);

    // Session sync reconciles once a second; the same unresolved state
    // must not append a diagnostic on every tick.
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t9",
    )));
    let unresolved = runtime
        .snapshot()
        .status
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "tab.active_unresolved")
        .count();
    assert_eq!(unresolved, 1);
}

#[test]
fn tab_strip_reorder_index_counts_positions_before_the_tab_leaves_the_list() {
    // Herdr inserts into the list it still holds and then takes the moved
    // tab out of its old place, so the index is where the tab that ends up
    // behind the moved one sits now. These three cases are the ones a live
    // 0.8.2 server was observed answering with exactly these orders.
    let current = ["t1", "t2", "t3"].map(str::to_owned);
    assert_eq!(
        herdr_insert_index(&current, &["t2", "t1", "t3"].map(str::to_owned), "t1"),
        Some(2)
    );
    assert_eq!(
        herdr_insert_index(&current, &["t2", "t3", "t1"].map(str::to_owned), "t1"),
        Some(3)
    );
    assert_eq!(
        herdr_insert_index(&current, &["t3", "t1", "t2"].map(str::to_owned), "t3"),
        Some(0)
    );
    // A file tab is not one of Herdr's, so there is no Herdr move to make.
    assert_eq!(
        herdr_insert_index(
            &current,
            &["t1", "t2", "t3"].map(str::to_owned),
            "file:notes"
        ),
        None
    );

    // A workspace split across two checkouts. The strip the operator drags
    // holds t1, t2 and t3; t5 is a sibling checkout's tab that Herdr still
    // counts. Dropping t1 at the end of that strip means "after t3", which
    // is one past the workspace's last position when t5 leads and t5's own
    // position when t5 trails. Reading the index off the strip alone gives
    // 3 in both cases, which puts the tab between t2 and t3.
    let leading = ["t5", "t1", "t2", "t3"].map(str::to_owned);
    assert_eq!(
        herdr_insert_index(&leading, &["t2", "t3", "t1"].map(str::to_owned), "t1"),
        Some(4)
    );
    let trailing = ["t1", "t2", "t3", "t5"].map(str::to_owned);
    assert_eq!(
        herdr_insert_index(&trailing, &["t2", "t3", "t1"].map(str::to_owned), "t1"),
        Some(3)
    );
    // A tab that keeps a successor in its own strip is placed in front of
    // it, wherever the workspace holds that successor.
    assert_eq!(
        herdr_insert_index(&leading, &["t2", "t1", "t3"].map(str::to_owned), "t1"),
        Some(3)
    );
}

#[test]
fn tab_strip_reorder_asks_herdr_and_lands_only_once_herdr_reports_the_order() {
    let (mut runtime, checkout_id, directory) = strip_checkout("herdr-move");
    let checkout_path = directory.to_string_lossy().into_owned();
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    open_file(&mut runtime, &checkout_id, &directory.join("notes.md"));
    let file_entry = strip_ids(&runtime, &checkout_id)[2].clone();
    let before = strip_ids(&runtime, &checkout_id);

    let herdr = FakeHerdr::start("tab-move", |_, _| tab_list(&["w-order:t2", "w-order:t1"]));
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });

    // Move the first Herdr tab behind the second. The file tab does not
    // move, so the arrangement differs from Herdr's only in Herdr's own
    // order, which is Herdr's to grant.
    assert!(reorder_tab(
        &mut runtime,
        &checkout_id,
        "herdr:w-order:t1",
        1
    ));
    herdr.wait_for_requests(1, Duration::from_secs(5));
    let request = &herdr.requests()[0];
    assert_eq!(request["method"], "tab.move");
    assert_eq!(
        request["params"],
        serde_json::json!({"tab_id": "w-order:t1", "insert_index": 2})
    );

    // The strip does not move on the operator's word alone.
    assert_eq!(strip_ids(&runtime, &checkout_id), before);
    assert_eq!(
        runtime.pending_tab_move[&checkout_id].herdr_order,
        vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()]
    );

    // Herdr reports the new order; the arrangement lands with it.
    let moved = ["w-order:t2", "w-order:t1"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &moved,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t2".to_owned(),
            "herdr:w-order:t1".to_owned(),
            file_entry
        ]
    );
    assert!(runtime.pending_tab_move.is_empty());

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_strip_reorder_indexes_a_move_in_the_whole_workspace_not_one_checkout() {
    let (mut runtime, checkout_id, repository, worktree) = split_workspace_checkouts("index-scope");
    let repository_path = repository.to_string_lossy().into_owned();
    let worktree_path = worktree.to_string_lossy().into_owned();
    // Herdr's list leads with the worktree's tab, so this checkout's tabs
    // do not start at the workspace's first position.
    let order = [
        ("w-order:t5", worktree_path.as_str()),
        ("w-order:t1", repository_path.as_str()),
        ("w-order:t2", repository_path.as_str()),
        ("w-order:t3", repository_path.as_str()),
    ];
    assert!(runtime.ingest_session(Ok(split_workspace_payload(&order, "w-order:t1"))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t1".to_owned(),
            "herdr:w-order:t2".to_owned(),
            "herdr:w-order:t3".to_owned()
        ],
        "the repository's checkout holds only its own three tabs"
    );

    let herdr = FakeHerdr::start("split-move", |_, _| {
        tab_list(&["w-order:t5", "w-order:t2", "w-order:t3", "w-order:t1"])
    });
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });

    // Drag the first tab to the end of this checkout's strip. In the
    // checkout's own coordinates that reads as index 3, which Herdr would
    // apply to its four-tab list and land the tab between t2 and t3.
    assert!(reorder_tab(
        &mut runtime,
        &checkout_id,
        "herdr:w-order:t1",
        2
    ));
    herdr.wait_for_requests(1, Duration::from_secs(5));
    let request = &herdr.requests()[0];
    assert_eq!(request["method"], "tab.move");
    assert_eq!(
        request["params"],
        serde_json::json!({"tab_id": "w-order:t1", "insert_index": 4})
    );
    assert!(
        runtime.snapshot().status.last_error.is_none(),
        "a move Herdr can make is not an error"
    );

    // Herdr reports the order the request asked for, so the arrangement
    // lands and the sibling checkout's tab is untouched.
    let moved = [
        ("w-order:t5", worktree_path.as_str()),
        ("w-order:t2", repository_path.as_str()),
        ("w-order:t3", repository_path.as_str()),
        ("w-order:t1", repository_path.as_str()),
    ];
    assert!(runtime.ingest_session(Ok(split_workspace_payload(&moved, "w-order:t1"))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t2".to_owned(),
            "herdr:w-order:t3".to_owned(),
            "herdr:w-order:t1".to_owned()
        ]
    );
    assert!(runtime.pending_tab_move.is_empty());

    std::fs::remove_dir_all(repository.parent().expect("fixture root")).ok();
}

#[test]
fn tab_strip_reorder_keeps_herdrs_order_and_reports_when_a_move_is_refused() {
    let (mut runtime, checkout_id, directory) = strip_checkout("herdr-refused");
    let checkout_path = directory.to_string_lossy().into_owned();
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    let before = strip_ids(&runtime, &checkout_id);
    runtime.pending_tab_move.insert(
        checkout_id.clone(),
        PendingTabMove {
            desired: vec!["herdr:w-order:t2".to_owned(), "herdr:w-order:t1".to_owned()],
            workspace_id: "w-order".to_owned(),
            herdr_order: vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()],
            target_id: "w-order:t1".to_owned(),
            generation: 7,
            connection_generation: 0,
            phase: "transmitting".to_owned(),
            stage: "request".to_owned(),
            started_at_unix_ms: 0,
            deadline_at_unix_ms: None,
            message: None,
            retryable: false,
        },
    );

    assert!(runtime.ingest_local_control_result(
        RemoteControlAction::MoveTab {
            checkout_id: checkout_id.clone(),
            tab_id: "w-order:t1".to_owned(),
            insert_index: 2,
            expected_order: vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()],
            generation: 7,
            connection_generation: 0,
        },
        Err("tab.move failed: tab_not_found: tab w-order:t9 not found".to_owned()),
        4,
    ));
    assert_eq!(strip_ids(&runtime, &checkout_id), before);
    assert!(runtime.pending_tab_move.is_empty());
    let error = runtime
        .snapshot()
        .status
        .last_error
        .clone()
        .expect("a refused move is reported");
    assert_eq!(error.kind, "tab.move_refused");
    assert!(error.message.contains("w-order:t1"), "{}", error.message);
    assert!(error.retryable);

    std::fs::remove_dir_all(&directory).ok();
}

/// AC12, SC6 recovery. A refused move leaves the strip where Herdr last
/// put it and says so, and the operator's answer to that is to drag again.
/// The retry has to be an ordinary drag: the same request goes out, and
/// the order lands when Herdr reports it, with nothing left over from the
/// refusal to hold it back.
#[test]
fn tab_strip_reorder_a_refused_drag_can_simply_be_dragged_again() {
    let (mut runtime, checkout_id, directory) = strip_checkout("herdr-retry");
    let checkout_path = directory.to_string_lossy().into_owned();
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    let before = strip_ids(&runtime, &checkout_id);

    // Both drags are answered; what the answer says does not matter here,
    // because the worker's callback is what carries it and this test
    // drives that by hand.
    let herdr = FakeHerdr::start("tab-retry", |_, _| tab_list(&["w-order:t2", "w-order:t1"]));
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });

    // The first drag is refused.
    assert!(reorder_tab(
        &mut runtime,
        &checkout_id,
        "herdr:w-order:t1",
        1
    ));
    let generation = runtime.pending_tab_move[&checkout_id].generation;
    assert!(
        runtime
            .complete_lane_tab(
                RemoteControlAction::MoveTab {
                    checkout_id: checkout_id.clone(),
                    tab_id: "w-order:t1".to_owned(),
                    insert_index: 2,
                    expected_order: vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()],
                    generation,
                    connection_generation: 0,
                },
                Err(live::ControlFailure::Definite(
                    "tab.move failed: tab_not_found: tab w-order:t1 not found".to_owned()
                )),
                4,
            )
            .0
    );
    assert_eq!(strip_ids(&runtime, &checkout_id), before);
    assert!(runtime.pending_tab_move.is_empty());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .expect("a refused move is reported")
            .kind,
        "tab.move_refused"
    );

    // The same drag again, with nothing else in between.
    assert!(reorder_tab(
        &mut runtime,
        &checkout_id,
        "herdr:w-order:t1",
        1
    ));
    herdr.wait_for_requests(2, Duration::from_secs(5));
    let requests = herdr.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["method"], "tab.move");
    assert_eq!(
        requests[1]["params"], requests[0]["params"],
        "the retry asks Herdr for exactly what the refused drag asked for"
    );
    assert_eq!(
        runtime.pending_tab_move[&checkout_id].herdr_order,
        vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()]
    );

    // Herdr grants it this time, and the strip lands.
    let moved = ["w-order:t2", "w-order:t1"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &moved,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec!["herdr:w-order:t2".to_owned(), "herdr:w-order:t1".to_owned()]
    );
    assert!(runtime.pending_tab_move.is_empty());

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_strip_reorder_ignores_a_result_a_later_drag_has_replaced() {
    let (mut runtime, checkout_id, directory) = strip_checkout("herdr-superseded");
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    let live = PendingTabMove {
        desired: vec!["herdr:w-order:t2".to_owned(), "herdr:w-order:t1".to_owned()],
        workspace_id: "w-order".to_owned(),
        herdr_order: vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()],
        target_id: "w-order:t1".to_owned(),
        generation: 9,
        connection_generation: 0,
        phase: "transmitting".to_owned(),
        stage: "request".to_owned(),
        started_at_unix_ms: 0,
        deadline_at_unix_ms: None,
        message: None,
        retryable: false,
    };
    runtime
        .pending_tab_move
        .insert(checkout_id.clone(), live.clone());

    // The first drag's refusal arrives after a second drag replaced it.
    assert!(runtime.ingest_local_control_result(
        RemoteControlAction::MoveTab {
            checkout_id: checkout_id.clone(),
            tab_id: "w-order:t1".to_owned(),
            insert_index: 2,
            expected_order: vec!["w-order:t2".to_owned(), "w-order:t1".to_owned()],
            generation: 8,
            connection_generation: 0,
        },
        Err("tab.move failed: transport".to_owned()),
        4,
    ));
    assert_eq!(runtime.pending_tab_move[&checkout_id], live);
    assert!(runtime.snapshot().status.last_error.is_none());

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_label_follows_focused_pane_unless_custom_named() {
    use crate::model::display_tab_label;
    assert_eq!(
        display_tab_label("notes", 2, Some("작업 제목"), Some("cargo")),
        "notes"
    );
    for raw in ["2", " Tab 2 ", ""] {
        assert_eq!(
            display_tab_label(raw, 2, Some("작업 제목"), Some("cargo")),
            "작업 제목"
        );
        assert_eq!(display_tab_label(raw, 2, None, Some("cargo")), "cargo");
        assert_eq!(display_tab_label(raw, 2, None, None), "Tab 2");
    }
    assert_eq!(display_tab_label("", 7, Some("  "), Some("zsh")), "zsh");
}

/// The shell used to name an unlabelled tab after its position, which
/// renamed every tab whenever one moved. Nothing may reintroduce that.
#[test]
fn tab_label_has_no_position_derived_path_left_in_the_shell() {
    let mut offenders = Vec::new();
    for (name, source) in web_sources() {
        if source.contains("fallbackIndex") || source.contains("displayLabel") {
            offenders.push(name.clone());
        }
        // The same class of defect, read the other way: the shell taking a
        // label the core formatted and parsing the number back out of it.
        // That writes the "Tab N" convention down a second time across the
        // wire, where a change to either half breaks the other in silence.
        // The core decides the next label; the shell draws it.
        if source.contains("startsWith(\"tab \")") || source.contains("startsWith(\"Tab \")") {
            offenders.push(name.clone());
        }
    }
    assert!(
        offenders.is_empty(),
        "a position-derived or reverse-parsed tab label path is back in {offenders:?}"
    );
}

#[test]
fn tab_label_names_the_next_tab_after_the_lowest_free_herdr_number() {
    let next = |labels: &[&str]| crate::model::next_tab_label(labels.iter().copied());
    assert_eq!(next(&[]), "Tab 1");
    assert_eq!(next(&["1", "2"]), "Tab 3");
    // The gap is taken before the end, so closing tab 1 and adding one
    // gives Tab 1 back rather than climbing forever.
    assert_eq!(next(&["2", "3"]), "Tab 1");
    // A named tab holds no number. A tab Herdr stores the display way
    // holds its number too: the shell hands `Tab N` to `tab.create`
    // verbatim, and counting only bare numbers made every new tab
    // `Tab 2` beside the last one.
    assert_eq!(next(&["1", "notes"]), "Tab 2");
    assert_eq!(next(&["Tab 1"]), "Tab 2");
    assert_eq!(next(&["1", "Tab 2", "Tab 2"]), "Tab 3");
    assert_eq!(next(&[" 2 ", "1"]), "Tab 3");
}

#[test]
fn a_plain_terminal_pane_cwd_is_reconciled_into_its_checkout() {
    let mut runtime = runtime();
    let checkout_path = "/private/tmp/hide-registered-checkout";
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:registered".to_owned(),
        label: "registered".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    let checkout_id =
        workspace::checkout_id_for_path("workspace:registered", Path::new(checkout_path));
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:registered".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.reset_terminal_projection(None);

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "tabs": [{
            "workspace_id": "herdr-workspace",
            "tab_id": "herdr-workspace:t1",
            "label": "2"
        }],
        "panes": [{"pane_id": "plain:p1", "cwd": checkout_path}],
        "layouts": [{
            "workspace_id": "herdr-workspace",
            "tab_id": "herdr-workspace:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "plain:p1",
            "panes": [{"pane_id": "plain:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("plain pane payload");

    // Before the session arrives the registration is the only row.
    assert_eq!(runtime.snapshot.navigator.workspaces.len(), 1);
    assert_eq!(runtime.snapshot.navigator.workspaces[0].checkouts.len(), 1);
    assert!(runtime.ingest_session(Ok(payload)));
    // Once Herdr has a workspace in that directory the registration's
    // row carries it, not a second entry beside it.
    assert_eq!(runtime.snapshot().navigator.workspaces.len(), 1);
    assert_eq!(
        runtime.snapshot().navigator.workspaces[0].session_workspace_ids,
        vec!["herdr-workspace".to_owned()]
    );
    let checkout = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == "workspace:registered")
        .and_then(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| checkout.id == checkout_id)
        })
        .expect("the checkout Herdr occupies");
    assert_eq!(
        checkout.tabs.len(),
        1,
        "plain pane layout should create one checkout tab"
    );
    assert_eq!(checkout.tabs[0].panes[0].id, "plain:p1");
    // Herdr's bare tab number reads as a label only after the display
    // rule turns it into a name.
    assert_eq!(checkout.tabs[0].label.as_deref(), Some("Tab 2"));
    assert_eq!(checkout.tabs[0].panes[0].cwd, checkout_path);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("plain:p1")
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.focused_pane_id.as_str()),
        Some("plain:p1")
    );
}

#[test]
fn an_exited_panes_root_directory_does_not_become_a_checkout() {
    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{"workspace_id": "w2W", "label": "modakbul"}],
        "panes": [
            {"pane_id": "w2W:p1", "cwd": "/private/tmp/hide-modakbul"},
            {"pane_id": "w2W:pM", "cwd": "/"}
        ],
        "tabs": [{"workspace_id": "w2W", "tab_id": "w2W:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w2W",
            "tab_id": "w2W:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w2W:p1",
            "panes": [
                {"pane_id": "w2W:p1", "rect": {"x": 0, "y": 0, "width": 40, "height": 24}},
                {"pane_id": "w2W:pM", "rect": {"x": 40, "y": 0, "width": 40, "height": 24}}
            ],
            "splits": []
        }]
    }))
    .expect("exited pane payload");

    let spaces = Runtime::session_spaces(
        &payload,
        &workspace::paths_here(Runtime::session_cwds(&payload)),
    );

    assert_eq!(spaces.len(), 1);
    assert_eq!(
        spaces[0].cwds,
        vec!["/private/tmp/hide-modakbul".to_owned()]
    );
}

#[test]
fn a_returned_pane_id_selects_its_layout_when_other_panes_share_the_cwd() {
    let mut runtime = runtime();
    let directory = scratch_dir("hide-selected-checkout-");
    let path = directory
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let checkout_path = path.as_str();
    runtime.test_dirs.push(directory);
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Selected".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let selected_workspace = workspace(
        &workspace_id,
        "Selected",
        checkout_path,
        vec![checkout(&workspace_id, &checkout_id, checkout_path, None)],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![selected_workspace.clone()];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id.clone());
    runtime.reset_terminal_projection(None);
    runtime.snapshot.terminal.pane_id = Some("w2X:pB".to_owned());
    runtime.snapshot.focused.pane_id = Some("w2X:pB".to_owned());
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w2X".to_owned(),
        tab_id: "w2X:t1".to_owned(),
        focused_pane_id: "w2X:pB".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w2X:pB".to_owned(),
        },
    }];
    runtime.snapshot.terminal.panes = vec![TerminalPaneSnapshot {
        pane_id: "w2X:pB".to_owned(),
        closed: false,
        exit_code: None,
        ..TerminalPaneSnapshot::default()
    }];

    // A pane id an operation returned (a created tab or worktree) holds the
    // keyboard before the session shows the pane's layout.
    let selected_pane = "w3V:p1";
    runtime.select_terminal_pane(Some(selected_pane.to_owned()));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some(selected_pane)
    );
    assert!(runtime.snapshot().active_pane_layout().is_none());

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [
            {"pane_id": "w2X:pB", "cwd": checkout_path},
            {"pane_id": selected_pane, "cwd": checkout_path}
        ],
        "tabs": [{"workspace_id": "w2X", "tab_id": "w2X:t1", "label": ""}, {"workspace_id": "w3V", "tab_id": "w3V:t1", "label": ""}],
        "layouts": [
            {
                "workspace_id": "w2X",
                "tab_id": "w2X:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w2X:pB",
                "panes": [{"pane_id": "w2X:pB", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            },
            {
                "workspace_id": "w3V",
                "tab_id": "w3V:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": selected_pane,
                "panes": [{"pane_id": selected_pane, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            }
        ]
    }))
    .expect("overlapping cwd session payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: vec![registration],
        workspaces: vec![selected_workspace],
        paths: Arc::new(workspace::paths_here([checkout_path])),
    };

    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(catalog)));
    let checkout = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .and_then(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| checkout.id == checkout_id)
        })
        .expect("selected checkout");
    assert!(checkout.tabs.iter().any(|tab| {
        tab.id.as_deref() == Some("w3V:t1") && tab.panes.iter().any(|pane| pane.id == selected_pane)
    }));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some(selected_pane)
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| (layout.workspace_id.as_str(), layout.tab_id.as_str())),
        Some(("w3V", "w3V:t1"))
    );
    assert!(runtime.snapshot().status.last_error.is_none());
}

#[test]
fn a_missing_selected_pane_reports_without_falling_back_to_a_same_cwd_pane() {
    let mut runtime = runtime();
    let checkout_path = "/tmp/hide-missing-selected-pane";
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Missing pane".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let selected_workspace = workspace(
        &workspace_id,
        "Missing pane",
        checkout_path,
        vec![checkout(&workspace_id, &checkout_id, checkout_path, None)],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![selected_workspace.clone()];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.snapshot.ui_state.selected_pane_id = Some("missing:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("missing:p1".to_owned());
    // The user chose this pane against a running session, so it is an
    // authoritative selection rather than a restore hint.
    runtime.restore_hint_pending = false;
    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "old:p1", "cwd": checkout_path}],
        "tabs": [{"workspace_id": "old-workspace", "tab_id": "old-workspace:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "old-workspace",
            "tab_id": "old-workspace:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "old:p1",
            "panes": [{"pane_id": "old:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("missing selected pane payload");
    let catalog = session_sync::PrecomputedCatalog {
        worktrees: runtime.worktree_catalog(),
        registrations: vec![registration],
        workspaces: vec![selected_workspace],
        paths: Default::default(),
    };

    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(catalog)));
    assert!(runtime.snapshot().active_pane_layout().is_none());
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("missing:p1")
    );
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable")
    );
}

/// A selection saved while another core ran this machine names that core's
/// device, and that core saved no pane of this machine at all when it held
/// this machine as a device; the window opens where this machine's Herdr has
/// the keyboard, which is where the last core left it, not on the catalog's
/// first row.
#[test]
fn a_restored_selection_of_another_machine_opens_where_herdr_has_the_keyboard() {
    let checkout = Some("remote:old-core:checkout:feature");
    for (saved_pane, saved_checkout, moved) in [
        (Some("remote:old-core:pane:w1:p1"), checkout, false),
        (None, checkout, false),
        // The other core saved nothing of this machine; its rekey says so.
        (None, None, true),
    ] {
        restored_selection_opens_on_herdrs_focus(saved_pane, saved_checkout, moved);
    }
}

#[test]
fn restored_checkout_waits_for_initial_path_facts() {
    let (mut runtime, checkout, repository, _) = split_workspace_checkouts("restore-pending");
    let repository = repository.to_string_lossy().into_owned();
    let mut payload = split_workspace_payload(&[("w-order:t1", &repository)], "w-order:t1");
    payload.focused_pane_id = Some("w-order:t1:p".to_owned());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout.clone());
    runtime.snapshot.ui_state.selected_pane_id = Some("w-order:t1:p".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout.clone());
    runtime.snapshot.navigator.workspaces.clear();
    runtime.restore_hint_pending = true;
    runtime.catalog_paths = Default::default();
    runtime.begin_catalog_coordinator();

    runtime.ingest_session(Ok(payload.clone()));
    assert!(
        runtime.restore_hint_pending,
        "unknown is not a missing checkout"
    );
    assert_eq!(
        runtime.snapshot.ui_state.focused_checkout_id.as_ref(),
        Some(&checkout)
    );
    assert_eq!(runtime.snapshot.status.last_error, None);

    let paths = runtime.ask_paths(BTreeSet::from([repository]));
    let spaces = Runtime::session_spaces(&payload, &paths);
    let catalog = session_sync::PrecomputedCatalog {
        registrations: runtime.snapshot.ui_state.workspace_registrations.clone(),
        worktrees: runtime.worktree_catalog(),
        workspaces: workspace::build_catalog(
            &runtime.node,
            &runtime.snapshot.ui_state.workspace_registrations,
            &spaces,
            &runtime.worktree_catalog(),
            &paths,
        ),
        paths,
    };
    runtime.ingest_session_with_catalog(Ok(payload), Some(catalog));
    assert!(!runtime.restore_hint_pending);
    assert_eq!(
        runtime.snapshot.ui_state.focused_checkout_id.as_ref(),
        Some(&checkout)
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
}

#[test]
fn a_known_saved_checkout_does_not_wait_for_an_unrelated_unknown_path() {
    let (mut runtime, checkout, repository, _) = split_workspace_checkouts("restore-known");
    let repository = repository.to_string_lossy().into_owned();
    let unrelated = format!("{repository}/not-yet-answered");
    let mut payload = split_workspace_payload(
        &[("w-order:t1", &repository), ("w-order:t2", &unrelated)],
        "w-order:t1",
    );
    payload.focused_pane_id = Some("w-order:t1:p".to_owned());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout.clone());
    runtime.snapshot.ui_state.selected_pane_id = Some("w-order:t1:p".to_owned());
    runtime.restore_hint_pending = true;
    runtime.catalog_paths = runtime.ask_paths(BTreeSet::from([repository]));
    runtime.begin_catalog_coordinator();

    runtime.ingest_session(Ok(payload));
    assert!(
        !runtime.restore_hint_pending,
        "the saved pane's facts are known"
    );
    assert_eq!(
        runtime.snapshot.ui_state.focused_checkout_id,
        Some(checkout)
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
    assert!(!runtime.catalog_paths.knows(&unrelated));
}

#[test]
fn a_removed_saved_checkout_restores_the_panes_known_current_checkout() {
    let (mut runtime, checkout, repository, worktree) =
        split_workspace_checkouts("restore-removed");
    let repository = repository.to_string_lossy().into_owned();
    let worktree_path = worktree.to_string_lossy().into_owned();
    let mut before = split_workspace_payload(
        &[("w-order:t1", &repository), ("w-order:t2", &worktree_path)],
        "w-order:t2",
    );
    before.focused_pane_id = Some("w-order:t2:p".to_owned());
    runtime.ingest_session(Ok(before));
    let saved_checkout = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .find(|row| row.path == worktree_path)
        .expect("linked checkout")
        .id
        .clone();
    runtime.snapshot.ui_state.focused_checkout_id = Some(saved_checkout.clone());
    runtime.snapshot.ui_state.selected_pane_id = Some("w-order:t2:p".to_owned());
    runtime.restore_hint_pending = true;
    std::fs::remove_dir_all(&worktree).unwrap();
    runtime.catalog_paths = runtime.ask_paths(BTreeSet::from([repository.clone(), worktree_path]));
    runtime.begin_catalog_coordinator();
    let unrelated = format!("{repository}/unanswered");
    let mut after = split_workspace_payload(
        &[("w-order:t2", &repository), ("w-order:t3", &unrelated)],
        "w-order:t2",
    );
    after.focused_pane_id = Some("w-order:t2:p".to_owned());

    runtime.ingest_session(Ok(after));
    assert!(
        !runtime.restore_hint_pending,
        "the moved pane has known facts"
    );
    assert_eq!(
        runtime.snapshot.ui_state.focused_checkout_id,
        Some(checkout)
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    assert_ne!(
        runtime.snapshot.ui_state.focused_checkout_id.as_ref(),
        Some(&saved_checkout)
    );
    assert!(!runtime.catalog_paths.knows(&unrelated));
}

fn restored_selection_opens_on_herdrs_focus(
    saved_pane: Option<&str>,
    saved_checkout: Option<&str>,
    moved: bool,
) {
    let (mut runtime, first_checkout, repository, worktree) =
        split_workspace_checkouts("restore-elsewhere");
    runtime.snapshot.ui_state.focused_checkout_id = saved_checkout.map(str::to_owned);
    runtime.snapshot.navigator.focused_checkout_id = saved_checkout.map(str::to_owned);
    runtime.snapshot.ui_state.selected_pane_id = saved_pane.map(str::to_owned);
    runtime.snapshot.ui_state.focus_from_herdr = moved;
    let repository = repository.to_string_lossy().into_owned();
    let worktree = worktree.to_string_lossy().into_owned();
    let mut payload = split_workspace_payload(
        &[
            ("w-order:t1", repository.as_str()),
            ("w-order:t5", worktree.as_str()),
        ],
        "w-order:t5",
    );
    payload.focused_pane_id = Some("w-order:t5:p".to_owned());
    runtime.ingest_session(Ok(payload));

    let focused = runtime.snapshot().navigator.focused_checkout_id.clone();
    assert_ne!(
        focused.as_deref(),
        Some(first_checkout.as_str()),
        "saved pane {saved_pane:?}, checkout {saved_checkout:?}, moved {moved}"
    );
    let worktree_checkout = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .find(|checkout| checkout.path == worktree)
        .map(|checkout| checkout.id.clone());
    assert_eq!(focused, worktree_checkout);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t5:p")
    );
    assert_eq!(runtime.snapshot().status.last_error, None);
    assert!(!runtime.snapshot.ui_state.focus_from_herdr, "taken once");
}

/// Herdr's keyboard on a pane no catalog checkout holds is no place to open:
/// a restore that missed starts on the catalog's first checkout, with no
/// selected pane outside it.
#[test]
fn a_missed_restore_with_herdrs_focus_outside_every_checkout_opens_catalog_first() {
    let (mut runtime, first_checkout, repository, worktree) =
        split_workspace_checkouts("restore-outside");
    runtime.snapshot.ui_state.focused_checkout_id =
        Some("remote:old-core:checkout:feature".to_owned());
    runtime.snapshot.navigator.focused_checkout_id =
        Some("remote:old-core:checkout:feature".to_owned());
    runtime.snapshot.ui_state.focus_from_herdr = true;
    let repository = repository.to_string_lossy().into_owned();
    let worktree = worktree.to_string_lossy().into_owned();
    let mut payload = split_workspace_payload(
        &[
            ("w-order:t1", repository.as_str()),
            ("w-order:t5", worktree.as_str()),
        ],
        "w-order:t1",
    );
    // A layout Herdr reports for a tab it lists in no workspace, so no
    // checkout holds its pane.
    let mut outside = payload.layouts[0].clone();
    outside.workspace_id = "w-loose".to_owned();
    outside.tab_id = "w-loose:t1".to_owned();
    outside.focused_pane_id = "w-loose:t1:p".to_owned();
    outside.panes[0].pane_id = "w-loose:t1:p".to_owned();
    payload.layouts.push(outside);
    payload.focused_pane_id = Some("w-loose:t1:p".to_owned());
    runtime.ingest_session(Ok(payload));

    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some(first_checkout.as_str())
    );
    assert_ne!(
        runtime.snapshot().ui_state.selected_pane_id.as_deref(),
        Some("w-loose:t1:p")
    );
    assert_ne!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-loose:t1:p")
    );
}

/// With nothing saved the start is the catalog's first checkout, but never
/// one whose folder the node found gone: a tab restored onto a folder moved
/// away while Hide was closed is a project of its own with no activity, so
/// whether it sorts first is a coin toss its id decides, and the window
/// opened on that empty project about half the time.
#[test]
fn a_start_with_nothing_saved_skips_a_checkout_whose_folder_is_gone() {
    // One of the two orders puts the gone folder first; both must start on
    // the folder that is there.
    for gone_first in [true, false] {
        let folder = scratch_dir("hide-start-gone-");
        let root = folder.path().canonicalize().expect("a real fixture path");
        let (kept, gone) = if gone_first {
            (root.join("kept"), root.join("gone"))
        } else {
            (root.join("gone"), root.join("kept"))
        };
        std::fs::create_dir_all(&kept).expect("kept folder");
        let kept = kept.to_string_lossy().into_owned();
        let gone = gone.to_string_lossy().into_owned();
        let mut runtime = runtime();
        runtime.restore_hint_pending = true;
        let mut payload = split_workspace_payload(
            &[("w-order:t1", &kept), ("w-order:t2", &gone)],
            "w-order:t1",
        );
        payload.focused_pane_id = Some("w-order:t1:p".to_owned());
        runtime.ingest_session(Ok(payload));

        let checkouts = || {
            runtime
                .snapshot()
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
        };
        assert!(
            checkouts().any(|checkout| checkout.path == gone),
            "the gone folder is still a row"
        );
        let kept_checkout = checkouts()
            .find(|checkout| checkout.path == kept)
            .map(|checkout| checkout.id.clone());
        assert!(kept_checkout.is_some());
        assert_eq!(
            runtime.snapshot().navigator.focused_checkout_id,
            kept_checkout,
            "{gone} is gone"
        );
    }
}

#[test]
fn a_restored_pane_the_session_no_longer_has_retargets_without_reporting() {
    let mut runtime = runtime();
    // What launch produces: ids read from disk that name the session that
    // ended, against a Herdr session that has since been restarted.
    runtime.snapshot.ui_state.selected_pane_id = Some("wW:p3".to_owned());
    runtime.snapshot.terminal.pane_id = Some("wW:p3".to_owned());
    runtime.snapshot.ui_state.focused_checkout_id = Some("checkout:gone".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout:gone".to_owned());

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "w19:p1", "cwd": "/tmp/hide-restored"}],
        "focused_pane_id": "w19:p1",
        "tabs": [{"workspace_id": "w19", "tab_id": "w19:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w19",
            "tab_id": "w19:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w19:p1",
            "panes": [{"pane_id": "w19:p1", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("restored pane payload");

    assert!(runtime.ingest_session_with_catalog(
        Ok(payload),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: Vec::new(),
            workspaces: Vec::new(),
            paths: Default::default(),
        }),
    ));
    assert_eq!(runtime.snapshot().status.last_error, None);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w19:p1")
    );
    assert!(runtime.snapshot().active_pane_layout().is_some());
}

#[test]
fn protocol_mismatch_exposes_typed_diagnostics_and_recovery_reports_the_connected_values() {
    let mut runtime = runtime();
    runtime.ingest_session(Err(SessionFetchError::Protocol {
        message: "incompatible runtime".to_owned(),
        expected_protocol: 23,
        received_protocol: 22,
        received_version: Some("0.9.0".to_owned()),
    }));

    let mismatch = &runtime.snapshot().status.herdr;
    assert_eq!(mismatch.state, "protocol_mismatch");
    assert_eq!(mismatch.expected_protocol, Some(23));
    assert_eq!(mismatch.received_protocol, Some(22));
    assert_eq!(mismatch.received_version.as_deref(), Some("0.9.0"));

    let mut payload = working_payload();
    payload.herdr_version = Some("0.9.1".to_owned());
    runtime.ingest_session(Ok(payload));
    let connected = &runtime.snapshot().status.herdr;
    assert_eq!(connected.state, "connected");
    // A healthy Herdr reports its real version and the revision that matched (B5).
    let revision = u64::from(crate::herdr_contract::HERDR_PROTOCOL_REVISION);
    assert_eq!(connected.expected_protocol, Some(revision));
    assert_eq!(connected.received_protocol, Some(revision));
    assert_eq!(connected.received_version.as_deref(), Some("0.9.1"));
}

/// AC11, R9, SC6. A checkout whose tabs come from two Herdr workspaces
/// refused every drag, because ownership was decided for the checkout
/// rather than for the drag. A drag that only steps over the other
/// workspace's tabs changes nothing Herdr can see, so it lands locally.
#[test]
fn tab_strip_reorder_a_drag_past_another_workspaces_tabs_needs_no_herdr_move() {
    let (mut runtime, checkout_id, directory) = strip_checkout("split-local");
    let checkout_path = directory.to_string_lossy().into_owned();
    assert!(runtime.ingest_session(Ok(split_checkout_payload(
        &checkout_path,
        &[
            ("w-left", &["w-left:t1"][..], "w-left:t1"),
            ("w-right", &["w-right:t1", "w-right:t2"][..], "w-right:t1"),
        ],
        "w-left"
    ))));
    let before = strip_ids(&runtime, &checkout_id);
    assert_eq!(
        before.len(),
        3,
        "the split checkout holds both workspaces: {before:?}"
    );

    // There is no live connection here, so any request to Herdr would fail
    // loudly. Landing silently is the assertion.
    let moved = before[0].clone();
    assert!(reorder_tab(&mut runtime, &checkout_id, &moved, 1));

    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        None
    );
    let after = strip_ids(&runtime, &checkout_id);
    assert_eq!(after[1], moved, "the drop did not land: {after:?}");
    assert!(runtime.pending_tab_move.is_empty());

    std::fs::remove_dir_all(&directory).ok();
}

/// AC11, R9. A drag that does change the moved tab's own workspace order
/// still asks Herdr, and asks with an index counted in that workspace, not
/// in the mixed strip the operator sees.
#[test]
fn tab_strip_reorder_a_drag_within_one_workspace_uses_that_workspaces_index() {
    let (mut runtime, checkout_id, directory) = strip_checkout("split-herdr");
    let checkout_path = directory.to_string_lossy().into_owned();
    assert!(runtime.ingest_session(Ok(split_checkout_payload(
        &checkout_path,
        &[
            ("w-left", &["w-left:t1"][..], "w-left:t1"),
            ("w-right", &["w-right:t1", "w-right:t2"][..], "w-right:t1"),
        ],
        "w-left"
    ))));
    let before = strip_ids(&runtime, &checkout_id);

    let herdr = FakeHerdr::start("split-move", |_, _| tab_list(&["w-right:t2", "w-right:t1"]));
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });

    // Move the right workspace's first tab behind its second.
    let moved = before
        .iter()
        .find(|id| id.ends_with("w-right:t1"))
        .expect("the right workspace's first tab")
        .clone();
    let target = before
        .iter()
        .position(|id| id.ends_with("w-right:t2"))
        .expect("the right workspace's second tab");
    assert!(reorder_tab(&mut runtime, &checkout_id, &moved, target));

    herdr.wait_for_requests(1, Duration::from_secs(5));
    let request = &herdr.requests()[0];
    assert_eq!(request["method"], "tab.move");
    assert_eq!(
        request["params"],
        serde_json::json!({"tab_id": "w-right:t1", "insert_index": 2}),
        "the index was not counted in the moved tab's own workspace"
    );
    // Nothing lands on the operator's word alone.
    assert_eq!(strip_ids(&runtime, &checkout_id), before);

    // Herdr reports the new order for the workspace it moved a tab in.
    // The held arrangement has to land here: the checkout holds two
    // workspaces' tabs, so what Herdr reports for one of them is never the
    // whole checkout's list, and a hold that waited for the whole list
    // would never be released.
    assert!(runtime.ingest_session(Ok(split_checkout_payload(
        &checkout_path,
        &[
            ("w-left", &["w-left:t1"][..], "w-left:t1"),
            ("w-right", &["w-right:t2", "w-right:t1"][..], "w-right:t1"),
        ],
        "w-left"
    ))));
    let after = strip_ids(&runtime, &checkout_id);
    let right = after
        .iter()
        .filter(|id| id.contains("w-right:"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        right,
        vec!["herdr:w-right:t2".to_owned(), "herdr:w-right:t1".to_owned()],
        "the move Herdr granted did not land: {after:?}"
    );
    assert!(
        runtime.pending_tab_move.is_empty(),
        "a granted move stayed pending in a checkout two workspaces share"
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// AC11, R9, SC6. A drag can both reorder the moved tab inside its own
/// workspace and carry it past another workspace's tabs. The operator's
/// arrangement is then an interleaving Herdr never reports, because Herdr
/// only ever states one workspace's order. Holding the checkout's mixed
/// order and waiting for it to come back left the drag pending forever and
/// the strip snapped back to where the tab started.
#[test]
fn tab_strip_reorder_a_drag_that_interleaves_two_workspaces_still_lands() {
    let (mut runtime, checkout_id, directory) = strip_checkout("split-interleaved");
    let checkout_path = directory.to_string_lossy().into_owned();
    let workspaces: [(&str, &[&str], &str); 2] = [
        ("w-left", &["w-left:t1"][..], "w-left:t1"),
        ("w-right", &["w-right:t1", "w-right:t2"][..], "w-right:t1"),
    ];
    assert!(runtime.ingest_session(Ok(split_checkout_payload(
        &checkout_path,
        &workspaces,
        "w-left"
    ))));
    let before = strip_ids(&runtime, &checkout_id);
    assert_eq!(
        before,
        vec![
            "herdr:w-left:t1".to_owned(),
            "herdr:w-right:t1".to_owned(),
            "herdr:w-right:t2".to_owned()
        ],
        "the fixture strip is not the order this test reasons about: {before:?}"
    );

    let herdr = FakeHerdr::start("split-interleave", |_, _| {
        tab_list(&["w-right:t2", "w-right:t1"])
    });
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });

    // The right workspace's second tab is dragged to the very front, over
    // the left workspace's tab as well as its own sibling.
    assert!(reorder_tab(
        &mut runtime,
        &checkout_id,
        "herdr:w-right:t2",
        0
    ));
    herdr.wait_for_requests(1, Duration::from_secs(5));
    let request = &herdr.requests()[0];
    assert_eq!(request["method"], "tab.move");
    assert_eq!(
        request["params"],
        serde_json::json!({"tab_id": "w-right:t2", "insert_index": 0}),
        "the index was not counted in the moved tab's own workspace"
    );

    // Herdr states the right workspace's new order. The checkout still
    // lists its tabs workspace by workspace, so what comes back is
    // `w-left:t1, w-right:t2, w-right:t1` - never the operator's
    // interleaving.
    let moved_workspaces: [(&str, &[&str], &str); 2] = [
        ("w-left", &["w-left:t1"][..], "w-left:t1"),
        ("w-right", &["w-right:t2", "w-right:t1"][..], "w-right:t1"),
    ];
    assert!(runtime.ingest_session(Ok(split_checkout_payload(
        &checkout_path,
        &moved_workspaces,
        "w-left"
    ))));

    assert!(
        runtime.pending_tab_move.is_empty(),
        "a granted move stayed pending because the operator interleaved two workspaces"
    );
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-right:t2".to_owned(),
            "herdr:w-left:t1".to_owned(),
            "herdr:w-right:t1".to_owned()
        ],
        "the drag did not stay where the operator dropped it"
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// The web Projects list's opened checkouts are the core's, changed by their
/// own toggle: a whole-state save, which an older or stale client sends with
/// whatever it last saw, keeps them.
#[test]
// Session-first-ui B7: new checkouts start open; only explicit collapses persist.
fn session_checkout_folds_survive_legacy_ui_state_updates() {
    let mut runtime = runtime();
    let dispatch = |runtime: &mut Runtime, kind: &str, payload: serde_json::Value| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": kind,
            "payload": payload
        }))
        .unwrap();
        runtime.dispatch_json(&event)
    };
    assert!(
        runtime
            .snapshot()
            .ui_state
            .session_collapsed_checkout_ids
            .is_empty()
    );
    assert!(dispatch(
        &mut runtime,
        "checkout_agents_toggle",
        serde_json::json!({"checkout_id": "checkout:web"})
    ));
    assert!(dispatch(
        &mut runtime,
        "checkout_agents_toggle",
        serde_json::json!({"checkout_id": "checkout:api"})
    ));
    assert!(dispatch(
        &mut runtime,
        "checkout_agents_toggle",
        serde_json::json!({"checkout_id": "checkout:web"})
    ));
    assert!(dispatch(
        &mut runtime,
        "checkout_agents_toggle",
        serde_json::json!({"checkout_id": "checkout:web"})
    ));
    assert_eq!(
        runtime.snapshot().ui_state.session_collapsed_checkout_ids,
        ["checkout:api", "checkout:web"]
    );

    assert!(dispatch(
        &mut runtime,
        "ui_state_update",
        serde_json::json!({
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "collapsed_checkout_ids": ["checkout:older"],
            "expanded_checkout_ids": [],
            "expanded_agent_pane_ids": ["pane:stale"]
        })
    ));
    assert!(dispatch(
        &mut runtime,
        "ui_state_update",
        serde_json::json!({
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "collapsed_checkout_ids": []
        })
    ));
    let state = &runtime.snapshot().ui_state;
    assert_eq!(
        state.session_collapsed_checkout_ids,
        ["checkout:api", "checkout:web"]
    );
    assert!(state.expanded_agent_pane_ids.is_empty());
    assert!(state.collapsed_checkout_ids.is_empty());
}

/// The web shell's chords are its own: a save from a shell that does not
/// know them (any older payload) keeps them.
#[test]
fn browser_shortcut_bindings_survive_a_ui_state_update_that_omits_them() {
    let mut runtime = runtime();
    let set = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "shortcut_bindings": {"split_right": "command+option+r"},
            "browser_shortcut_bindings": {"split_right": "alt+KeyR"}
        }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&set));
    let native_save = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "shortcut_bindings": {}
        }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&native_save));
    let state = &runtime.snapshot().ui_state;
    assert!(state.shortcut_bindings.is_empty());
    assert_eq!(
        state
            .browser_shortcut_bindings
            .get("split_right")
            .map(String::as_str),
        Some("alt+KeyR")
    );

    let oversized: serde_json::Map<String, serde_json::Value> = (0..40)
        .map(|index| (format!("command_{index}"), serde_json::json!("alt+KeyR")))
        .collect();
    let flood = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {
            "expanded_paths": [],
            "selected_path": null,
            "selected_pane_id": null,
            "browser_shortcut_bindings": oversized
        }
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&flood));
    assert_eq!(
        runtime.snapshot().ui_state.browser_shortcut_bindings.len(),
        1
    );
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("ui_state.browser_shortcuts_invalid")
    );
}

/// S6 B15. A chip or Return aimed at a pane that is closing is refused, and
/// the refusal answers that request, or the control would stay pending.
#[test]
fn pane_focus_request_on_a_closing_pane_is_answered_as_failed() {
    let checkout_path = "/private/tmp/hide-pane-focus-closing";
    let (mut runtime, _) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let close = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "close_pane",
        "payload": {"pane_id": "w-order:t2:p", "confirmed": true}
    }))
    .expect("close pane event");
    runtime.dispatch_json(&close);

    runtime.dispatch_json(&correlated_pane_focus_event("w-order:t2:p", "chip-1"));
    let request = runtime
        .snapshot()
        .status
        .pane_focus_request
        .as_ref()
        .expect("the refusal answers the request");
    assert_eq!(request.request_id, "chip-1");
    assert_eq!(request.phase, "failed");
    assert!(!request.retryable);
    assert!(request.message.as_deref().unwrap_or("").contains("closing"));
}

/// S6 B21: a Workspace on this machine chosen while a device is in front is
/// one event that brings this machine forward too, and a refused one moves
/// neither.
#[test]
fn a_checkout_chosen_with_its_device_moves_both_or_neither() {
    let checkout_path = "/private/tmp/hide-checkout-with-device";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let choose = |checkout_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "focus_checkout",
            "payload": {"workspace_id": "workspace:order", "checkout_id": checkout_id, "focus_device": true}
        }))
        .expect("focus checkout event")
    };
    runtime.snapshot.navigator.focused_device_id = Some("mini".to_owned());

    runtime.dispatch_json(&choose("checkout:missing"));
    assert!(runtime.snapshot.status.last_error.is_none());
    assert!(
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|entry| entry.kind == "checkout.row_stale")
    );
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some("mini")
    );

    runtime.dispatch_json(&choose(&checkout_id));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(crate::node::TEST_NODE)
    );
}

#[test]
fn checkout_row_admits_focus_and_disclosure_together_and_rejects_stale_targets() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/tmp/project",
        vec![
            checkout("project", "first", "/tmp/project", None),
            checkout("project", "second", "/tmp/second", None),
        ],
    )];
    let activate = |runtime: &mut Runtime, project: &str, checkout: &str, expanded: bool| {
        runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "focus_checkout",
            "payload": { "workspace_id": project, "checkout_id": checkout, "expanded": expanded }
        })).unwrap());
    };
    activate(&mut runtime, "project", "first", true);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("first")
    );
    assert_eq!(
        snapshot.ui_state.session_collapsed_checkout_ids,
        Vec::<String>::new()
    );
    activate(&mut runtime, "project", "second", true);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("second")
    );
    assert_eq!(
        snapshot.ui_state.session_collapsed_checkout_ids,
        Vec::<String>::new()
    );
    // Repeating an explicit target converges; closing only removes this row.
    activate(&mut runtime, "project", "second", true);
    activate(&mut runtime, "project", "second", false);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("second")
    );
    assert_eq!(snapshot.ui_state.session_collapsed_checkout_ids, ["second"]);
    for (project, checkout, expanded) in [
        ("missing", "first", Some(true)),
        ("project", "missing", Some(true)),
        ("missing", "first", None),
        ("project", "missing", None),
    ] {
        let mut payload = serde_json::json!({"workspace_id": project, "checkout_id": checkout});
        if let Some(expanded) = expanded {
            payload["expanded"] = serde_json::json!(expanded);
        }
        runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": "focus_checkout", "payload": payload})).unwrap());
        let snapshot = runtime.snapshot();
        assert_eq!(
            snapshot.navigator.focused_workspace_id.as_deref(),
            Some("project")
        );
        assert_eq!(
            snapshot.navigator.focused_checkout_id.as_deref(),
            Some("second")
        );
        assert_eq!(snapshot.ui_state.session_collapsed_checkout_ids, ["second"]);
        assert!(snapshot.status.last_error.is_none());
        assert!(
            snapshot
                .status
                .diagnostics
                .iter()
                .any(|entry| entry.kind == "checkout.row_stale")
        );
    }
}

#[test]
fn project_row_admits_focus_and_disclosure_together() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/tmp/project",
        vec![
            checkout("project", "first", "/tmp/project", None),
            checkout("project", "second", "/tmp/second", None),
        ],
    )];
    let choose = |runtime: &mut Runtime, id: &str, expanded: bool| {
        runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "focus_checkout",
            "payload": {"workspace_id": "project", "checkout_id": id, "project_expanded": expanded}
        })).unwrap());
    };
    choose(&mut runtime, "second", false);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("second")
    );
    assert_eq!(snapshot.ui_state.collapsed_workspace_ids, ["project"]);
    choose(&mut runtime, "second", false);
    assert_eq!(
        runtime.snapshot().ui_state.collapsed_workspace_ids,
        ["project"]
    );
    choose(&mut runtime, "missing", true);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("second")
    );
    assert_eq!(snapshot.ui_state.collapsed_workspace_ids, ["project"]);
    assert!(snapshot.status.last_error.is_none());
    assert!(
        snapshot
            .status
            .diagnostics
            .iter()
            .any(|entry| entry.kind == "checkout.row_stale")
    );
    choose(&mut runtime, "first", true);
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some("first")
    );
    assert!(snapshot.ui_state.collapsed_workspace_ids.is_empty());
}

#[test]
fn tab_rename_keeps_the_committed_name_on_failure_and_ignores_old_receipts() {
    let mut runtime = runtime();
    let mut checkout = checkout("w", "c", "/fixture", None);
    let mut tab = tab("w", "c", None);
    tab.naming = crate::model::TabNaming {
        focused_pane_id: String::new(),
        raw: "Saved".into(),
        automatic: "작업 제목".into(),
        ..Default::default()
    };
    tab.label = Some("Saved".into());
    checkout.tabs.push(tab.clone());
    checkout.strip = StripTabSnapshot::from_herdr_tabs(&checkout.tabs);
    runtime.snapshot.navigator.workspaces =
        vec![workspace("w", "Fixture", "/fixture", vec![checkout])];
    runtime.snapshot.tab = tab;
    let request = |id: &str, label: &str| RenameTabPayload {
        request_id: id.into(),
        tab_id: "c:tab".into(),
        label: label.into(),
    };
    // No connector is installed. The real event failure leaves the committed
    // name and records a correlated retry result without a global alert.
    runtime.rename_tab(request("first", "변경할 이름"));
    assert_eq!(
        runtime.snapshot.status.tab_rename.as_ref().unwrap().phase,
        "failed"
    );
    assert_eq!(
        runtime.snapshot.status.tab_rename.as_ref().unwrap().label,
        "변경할 이름"
    );
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("Saved"));
    assert!(runtime.snapshot.status.last_error.is_none());
    runtime.rename_tab(request("retry", ""));
    assert!(!runtime.ingest_tab_rename_result("first", Ok(())));
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("Saved"));
    runtime.ingest_tab_rename_result("retry", Ok(()));
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("작업 제목"));
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0].strip[0].label,
        "작업 제목"
    );
    // A stale projection cannot undo an acknowledged clear; once Herdr
    // confirms it, a subsequent external rename becomes authoritative.
    let mut incoming = runtime.snapshot.tab.clone();
    incoming.naming.raw = "Saved".into();
    incoming.label = Some("Saved".into());
    runtime.apply_tab_rename(&mut incoming);
    assert_eq!(incoming.label.as_deref(), Some("작업 제목"));
    incoming.naming.raw.clear();
    runtime.apply_tab_rename(&mut incoming);
    incoming.naming.raw = "External".into();
    incoming.label = Some("External".into());
    runtime.apply_tab_rename(&mut incoming);
    assert_eq!(incoming.label.as_deref(), Some("External"));

    let herdr =
        crate::fake_herdr::FakeHerdr::start("rename-busy", |_, _| serde_json::json!({"type":"ok"}));
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_owned(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });
    runtime.rename_tab(request("slow", "First"));
    runtime.rename_tab(request("newer", "Retry me"));
    runtime.ingest_tab_rename_result("slow", Ok(()));
    let receipt = runtime.snapshot.status.tab_rename.as_ref().unwrap();
    assert_eq!(
        (&*receipt.request_id, &*receipt.phase, &*receipt.label),
        ("newer", "failed", "Retry me")
    );
    runtime.live = None;
}

#[test]
fn tab_name_follows_core_focus_before_herdr_and_on_refusal() {
    let path = "/private/tmp/hide-tab-focus-name";
    let (mut runtime, _) = tab_order_runtime(path);
    let mut payload = tab_order_payload(path, &["w-order:t1"], &["w-order:t1"], "w-order:t1");
    let mut second = payload.panes[0].clone();
    second.pane_id = "w-order:p2".into();
    second.foreground_process = Some("cargo".into());
    payload.panes[0].foreground_process = Some("zsh".into());
    payload.panes.push(second);
    let mut second_rect = payload.layouts[0].panes[0].clone();
    second_rect.pane_id = "w-order:p2".into();
    second_rect.rect.x = 40;
    second_rect.rect.width = 40;
    payload.layouts[0].panes[0].rect.width = 40;
    payload.layouts[0].panes.push(second_rect);
    payload.layouts[0].splits = crate::sidebar::owned_label_fixture(serde_json::json!([{
        "direction":"right", "ratio":0.5,
        "rect":{"x":0,"y":0,"width":80,"height":24}
    }]))
    .unwrap();
    runtime.ingest_session(Ok(payload.clone()));
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("zsh"));
    let mut external = payload.clone();
    external.focused_pane_id = Some("w-order:p2".into());
    external.layouts[0].focused_pane_id = "w-order:p2".into();
    runtime.ingest_session(Ok(external));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:p2")
    );
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("cargo"));
    runtime.ingest_session(Ok(payload));
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("zsh"));
    runtime.dispatch_json(&correlated_pane_focus_event("w-order:p2", "label-focus"));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:p2")
    );
    assert_eq!(runtime.snapshot.tab.label.as_deref(), Some("cargo"));
    assert_eq!(runtime.snapshot.tab.naming.focused_pane_id, "w-order:p2");
    assert_eq!(
        runtime
            .snapshot
            .status
            .pane_focus_request
            .as_ref()
            .unwrap()
            .phase,
        "failed"
    );
}
