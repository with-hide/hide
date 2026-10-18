use super::workspace_view::with_new_views;
use super::*;
use crate::model::InputRequestState;

fn setup() -> (Runtime, String) {
    let (runtime, checkout) = tab_order_runtime("/agent-groups");
    let mut runtime = with_new_views(runtime, "agent-groups");
    ingest(&mut runtime, &["w-order:t1", "w-order:t2", "w-order:t3"]);
    (runtime, checkout)
}
fn ingest(runtime: &mut Runtime, tabs: &[&str]) {
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        tabs,
        tabs,
        "w-order:t1",
    )));
    runtime.sync_workspace_view();
}
fn action(runtime: &mut Runtime, mut payload: serde_json::Value) {
    payload["workspace"] =
        serde_json::json!({"device_id":crate::node::TEST_NODE,"path":"/agent-groups"});
    runtime.dispatch_json(&explorer_event("agent_layout", payload));
}
fn layout(runtime: &Runtime) -> crate::agent_layout::Layout {
    runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .get(crate::node::TEST_NODE, "/agent-groups")
        .unwrap()
        .agent_layout
        .clone()
}
#[test]
fn agent_split_shows_both_tabs_retains_their_attaches_and_preserves_herdr_order() {
    let (mut runtime, checkout) = setup();
    let order = ordered_tab_ids(&runtime, &checkout);
    action(
        &mut runtime,
        serde_json::json!({"action":"split","tab_id":"w-order:t2","area_id":"a1","edge":"right","request_id":"split-1"}),
    );
    assert_eq!(layout(&runtime).shown(), ["w-order:t1", "w-order:t2"]);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), order);
    assert!(runtime.recent_visible_tabs.contains(&"w-order:t1".into()));
    assert!(runtime.recent_visible_tabs.contains(&"w-order:t2".into()));
    let before = layout(&runtime);
    action(
        &mut runtime,
        serde_json::json!({"action":"split","tab_id":"w-order:t2","area_id":"a1","edge":"right","request_id":"split-1"}),
    );
    assert_eq!(layout(&runtime), before);
    ingest(
        &mut runtime,
        &["w-order:t3", "w-order:t2", "w-order:t1", "w-order:t4"],
    );
    assert_eq!(layout(&runtime).active(), Some("w-order:t2"));
    assert_eq!(
        layout(&runtime).tree.areas()[0]
            .displays
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["w-order:t1", "w-order:t3"]
    );
}
#[test]
fn agent_last_member_departure_collapses_its_area_and_stale_workspace_does_nothing() {
    let (mut runtime, _) = setup();
    action(
        &mut runtime,
        serde_json::json!({"action":"split","tab_id":"w-order:t2","area_id":"a1","edge":"down","request_id":"split-1"}),
    );
    action(
        &mut runtime,
        serde_json::json!({"action":"move","tab_id":"w-order:t2","area_id":"a1","index":0}),
    );
    assert_eq!(layout(&runtime).tree.area_count(), 1);
    let before = layout(&runtime);
    runtime.dispatch_json(&explorer_event("agent_layout", serde_json::json!({"workspace":{"device_id":crate::node::TEST_NODE,"path":"/other"},"action":"split","tab_id":"w-order:t1","area_id":"a1","edge":"right","request_id":"stale"})));
    assert_eq!(layout(&runtime), before);
}

#[test]
fn requested_new_tab_area_survives_a_focus_change_before_topology_arrives() {
    let (mut runtime, _) = setup();
    action(
        &mut runtime,
        serde_json::json!({"action":"split","tab_id":"w-order:t2","area_id":"a1","edge":"right","request_id":"split-new"}),
    );
    runtime.ingest_local_control_result(
        RemoteControlAction::CreateTab {
            workspace_id: "w-order".into(),
            cwd: "/agent-groups".into(),
            label: "Tab".into(),
            area_id: Some("a1".into()),
            admission_id: None,
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w-order:t4".into()),
            created_pane_id: Some("w-order:p4".into()),
        }),
        1,
    );
    ingest(
        &mut runtime,
        &["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"],
    );
    let arranged = layout(&runtime);
    assert_eq!(
        arranged
            .tree
            .area("a1")
            .unwrap()
            .displays
            .last()
            .unwrap()
            .id,
        "w-order:t4"
    );
    assert_eq!(arranged.tree.active_area, "a1");
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_placements
            .is_empty()
    );
}

#[test]
fn replacement_predicate_requires_primary_provenance_a_linked_sibling_and_its_last_tab() {
    let (mut runtime, _) = setup();
    let primary = crate::domain::WorktreeProjection {
        repo_key: "repo".into(),
        repo_name: "repo".into(),
        repo_root: "/repo".into(),
        checkout_path: "/repo".into(),
        is_linked_worktree: false,
    };
    runtime
        .herdr_worktrees
        .insert("w-order".into(), primary.clone());
    runtime.herdr_worktrees.insert(
        "linked".into(),
        crate::domain::WorktreeProjection {
            is_linked_worktree: true,
            ..primary
        },
    );
    assert!(!runtime.primary_needs_shell("w-order", "w-order:t1"));
    runtime
        .herdr_workspace_tab_order
        .insert("w-order".into(), vec!["w-order:t1".into()]);
    assert!(runtime.primary_needs_shell("w-order", "w-order:t1"));
    assert!(!runtime.primary_needs_shell("linked", "w-order:t1"));
    runtime.herdr_worktrees.remove("linked");
    assert!(!runtime.primary_needs_shell("w-order", "w-order:t1"));
}

#[test]
fn external_focused_new_tab_preserves_canvas_then_existing_tab_focus_is_followed() {
    let (mut runtime, checkout) = setup();
    let before_pane = runtime.snapshot.terminal.pane_id.clone();
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &tabs,
        &tabs,
        "w-order:t4",
    )));
    assert_eq!(layout(&runtime).active(), Some("w-order:t1"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(runtime.snapshot.terminal.pane_id, before_pane);
    assert_eq!(layout(&runtime).tree.displays().count(), 4);
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &tabs,
        &tabs,
        "w-order:t4",
    )));
    runtime.sync_workspace_view();
    assert_eq!(layout(&runtime).active(), Some("w-order:t4"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t4")
    );
}

#[test]
fn creation_provenance_preserves_canvas_after_admission_and_same_tab_refocus_is_followed() {
    let (mut runtime, checkout) = setup();
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    // Membership may already be known before a delayed creation focus arrives.
    ingest(&mut runtime, &tabs);
    let mut payload = tab_order_payload("/agent-groups", &tabs, &tabs, "w-order:t4");
    payload.tab_focus = Some(crate::sidebar::SessionTabFocus {
        generation: 1,
        workspace_id: "w-order".into(),
        tab_id: "w-order:t4".into(),
        revision: 1,
        creation: true,
    });
    runtime.ingest_session(Ok(payload.clone()));
    runtime.ingest_session(Ok(payload.clone()));
    assert_eq!(layout(&runtime).active(), Some("w-order:t1"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    let focus = payload.tab_focus.as_mut().unwrap();
    focus.creation = false;
    focus.revision = 2;
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    assert_eq!(layout(&runtime).active(), Some("w-order:t4"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t4")
    );
}

#[test]
fn raw_snapshot_does_not_rearm_a_consumed_focus_event_after_local_timeout() {
    let (mut runtime, checkout) = setup();
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let mut event = tab_order_payload("/agent-groups", &tabs, &tabs, "w-order:t2");
    event.tab_focus = Some(crate::sidebar::SessionTabFocus {
        generation: 1,
        workspace_id: "w-order".into(),
        tab_id: "w-order:t2".into(),
        revision: 1,
        creation: false,
    });
    runtime.ingest_session(Ok(event.clone()));
    runtime.dispatch_json(&focus_tab_event(&checkout, "w-order:t1"));
    runtime.expire_pending_view_focus(u64::MAX);
    let mut raw = event.clone();
    raw.tab_focus = None;
    runtime.ingest_session(Ok(raw));
    runtime.ingest_session(Ok(event.clone()));
    runtime.sync_workspace_view();
    assert_eq!(layout(&runtime).active(), Some("w-order:t1"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    // A reconnect's new stream cannot collide with the previous revision.
    event.tab_focus.as_mut().unwrap().generation = 2;
    runtime.ingest_session(Ok(event));
    runtime.sync_workspace_view();
    assert_eq!(layout(&runtime).active(), Some("w-order:t2"));
}

#[test]
fn authoritative_overflow_waits_without_hidden_focus_and_admits_when_a_slot_opens() {
    let (mut runtime, checkout) = setup();
    let ids = (1..=65)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &tabs,
        &tabs,
        "w-order:t65",
    )));
    runtime.sync_workspace_view();
    assert_eq!(layout(&runtime).tree.display_count(), 64);
    assert_eq!(layout(&runtime).waiting, 1);
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .agent_layout
            .as_ref()
            .expect("this machine's Agent areas")
            .waiting,
        1
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout).len(), 65);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
    runtime.dispatch_json(&focus_tab_event(&checkout, "w-order:t65"));
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    runtime.focus_pane("w-order:t65:p".into(), PaneFocusOrigin::Operator, None);
    assert_eq!(
        runtime.snapshot.terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
    let workspace = runtime
        .snapshot
        .navigator
        .focused_workspace_id
        .clone()
        .unwrap();
    runtime.apply(Event::CreateTab(CreateTabPayload {
        workspace_id: workspace,
        checkout_id: Some(checkout.clone()),
        label: "Tab".into(),
        area_id: None,
        request_id: None,
    }));
    // There is no live control worker in this fixture. Admission must fail
    // before even trying to obtain one or changing the current selection.
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "agent_layout.display_limit"
    );
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_admissions
            .is_empty()
    );
    let remaining = tabs
        .iter()
        .copied()
        .filter(|id| *id != "w-order:t2")
        .collect::<Vec<_>>();
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &remaining,
        &remaining,
        "w-order:t65",
    )));
    runtime.sync_workspace_view();
    let arranged = layout(&runtime);
    assert_eq!(arranged.waiting, 0);
    assert_eq!(arranged.tree.display_count(), 64);
    assert_eq!(
        arranged.tree.active_area().displays.last().unwrap().id,
        "w-order:t65"
    );
    assert_eq!(arranged.active(), Some("w-order:t1"));
    let mut restored: crate::agent_layout::Layout =
        crate::sidebar::owned_label_fixture(serde_json::to_value(&arranged).unwrap()).unwrap();
    restored.repair();
    assert!(restored.tree.display("w-order:t65").is_some());
}

#[test]
fn pending_agent_admissions_are_counted_before_the_next_effect() {
    let (mut runtime, _) = setup();
    let ids = (1..=63)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    ingest(&mut runtime, &tabs);
    assert!(runtime.admit_agent_tab("/agent-groups"));
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .agent_admissions
        .insert(
            (crate::node::TEST_NODE.into(), "/agent-groups".into()),
            HashSet::from(["create:1".into()]),
        );
    assert!(!runtime.admit_agent_tab("/agent-groups"));
    runtime.finish_agent_admission("/agent-groups", 1);
    assert!(runtime.admit_agent_tab("/agent-groups"));
}

#[test]
fn protected_close_and_reopen_at_capacity_refuse_before_any_worker_or_selection_change() {
    let (mut runtime, _) = setup();
    let ids = (1..=64)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    ingest(&mut runtime, &tabs);
    let primary = crate::domain::WorktreeProjection {
        repo_key: "repo".into(),
        repo_name: "repo".into(),
        repo_root: "/agent-groups".into(),
        checkout_path: "/agent-groups".into(),
        is_linked_worktree: false,
    };
    runtime
        .herdr_worktrees
        .insert("w-order".into(), primary.clone());
    runtime.herdr_worktrees.insert(
        "linked".into(),
        crate::domain::WorktreeProjection {
            is_linked_worktree: true,
            ..primary
        },
    );
    // The other checkout tabs may belong to other Herdr workspaces. Its
    // protected primary workspace has only the target remaining.
    runtime
        .herdr_workspace_tab_order
        .insert("w-order".into(), vec!["w-order:t1".into()]);
    let tab = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|w| &w.checkouts)
        .flat_map(|c| &c.tabs)
        .find(|tab| tab.id.as_deref() == Some("w-order:t1"))
        .unwrap()
        .clone();
    let before = runtime.snapshot.terminal.pane_id.clone();
    let context = runtime.close_context(&tab).unwrap();
    assert!(context.replacement_shell);
    runtime.start_close_capture(
        live::CloseCaptureTarget::Tab {
            tab_id: "w-order:t1".into(),
        },
        tab.clone(),
    );
    assert!(runtime.close_operations.is_empty());
    assert!(runtime.panes_closing.is_empty());
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "agent_layout.display_limit"
    );
    assert_eq!(runtime.snapshot.terminal.pane_id, before);
    runtime.push_recent_closed(ClosedItem::Tab {
        key: "capacity-reopen".into(),
        context,
        layout: crate::recent_closed::ClosedLayout {
            workspace_id: "w-order".into(),
            tab_id: "retired".into(),
            zoomed: false,
            focused_pane_id: "retired:p".into(),
            root: crate::recent_closed::ClosedLayoutNode::Pane {
                pane_id: Some("retired:p".into()),
                label: None,
                cwd: Some("/agent-groups".into()),
                command: None,
                env: BTreeMap::new(),
            },
        },
        panes: runtime.closed_panes(&tab),
    });
    runtime.reopen_closed();
    assert!(runtime.reopen_in_flight.is_none());
    assert_eq!(
        runtime.recent_closed.back().unwrap().key(),
        "capacity-reopen"
    );
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "agent_layout.display_limit"
    );
}

#[test]
fn admitted_tab_keeps_its_slot_when_external_topology_arrives_first() {
    let (mut runtime, _) = setup();
    let mut ids = (1..=63)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .agent_admissions
        .insert(
            (crate::node::TEST_NODE.into(), "/agent-groups".into()),
            HashSet::from(["create:71".into()]),
        );
    ids.push("w-order:external".into());
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(layout(&runtime).tree.display_count(), 63);
    assert_eq!(layout(&runtime).waiting, 1);
    runtime.ingest_local_control_result(
        RemoteControlAction::CreateTab {
            workspace_id: "w-order".into(),
            cwd: "/agent-groups".into(),
            label: "Tab".into(),
            area_id: Some("a1".into()),
            admission_id: Some(71),
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w-order:admitted".into()),
            created_pane_id: None,
        }),
        1,
    );
    // The known ID also retains its slot while still absent from topology.
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(layout(&runtime).tree.display_count(), 63);
    ids.push("w-order:admitted".into());
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert!(layout(&runtime).tree.display("w-order:admitted").is_some());
    assert!(layout(&runtime).tree.display("w-order:external").is_none());
    assert_eq!(layout(&runtime).waiting, 1);
}

#[test]
fn uncertain_create_retains_its_own_claim_until_a_definite_result() {
    let (mut runtime, _) = setup();
    let ids = (1..=62)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    ingest(&mut runtime, &tabs);
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .agent_admissions
        .insert(
            (crate::node::TEST_NODE.into(), "/agent-groups".into()),
            HashSet::from(["create:71".into(), "create:72".into()]),
        );
    let create = |id| RemoteControlAction::CreateTab {
        workspace_id: "w-order".into(),
        cwd: "/agent-groups".into(),
        label: "Tab".into(),
        area_id: Some("a1".into()),
        admission_id: Some(id),
    };
    runtime.ingest_local_control_failure(
        create(71),
        Err(live::ControlFailure::Ambiguous(
            "lost acknowledgement".into(),
        )),
        1,
    );
    ingest(&mut runtime, &tabs);
    assert!(!runtime.admit_agent_tab("/agent-groups"));
    runtime.ingest_local_control_failure(
        create(72),
        Err(live::ControlFailure::Definite("refused".into())),
        1,
    );
    let pending = &runtime.workspace_views.as_ref().unwrap().agent_admissions;
    assert_eq!(
        pending
            .get(&(crate::node::TEST_NODE.into(), "/agent-groups".into()))
            .unwrap(),
        &HashSet::from(["create:71".into()])
    );
    assert!(runtime.admit_agent_tab("/agent-groups"));
    runtime.ingest_local_control_result(
        create(71),
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w-order:recovered".into()),
            created_pane_id: None,
        }),
        1,
    );
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_admissions
            .is_empty()
    );
    let mut recovered = tabs.clone();
    recovered.push("w-order:recovered");
    ingest(&mut runtime, &recovered);
    assert_eq!(layout(&runtime).tree.display_count(), 63);
    assert!(runtime.admit_agent_tab("/agent-groups"));
}

#[test]
fn replacement_retry_with_missing_shell_at_capacity_sends_no_effect() {
    let (mut runtime, _) = setup();
    let ids = (1..=64)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let tab = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|w| &w.checkouts)
        .flat_map(|c| &c.tabs)
        .find(|tab| tab.id.as_deref() == Some("w-order:t1"))
        .unwrap()
        .clone();
    let mut context = runtime.close_context(&tab).unwrap();
    context.replacement_shell = true;
    let request = live::CloseCaptureRequest {
        failed_terminals: Vec::new(),
        retain_for_reopen: true,
        key: "retained-close".into(),
        connection_generation: runtime.live_generation,
        context,
        panes: runtime.closed_panes(&tab),
        target: live::CloseCaptureTarget::Tab {
            tab_id: "w-order:t1".into(),
        },
    };
    runtime.ensure_pending_close_from_request(&request);
    let operation = runtime.close_operations.get_mut("retained-close").unwrap();
    operation.phase = "refused".into();
    operation.stage = "close_request".into();
    operation.replacement_id = Some("w-order:removed-shell".into());
    assert!(runtime.retry_agent_close("retained-close"));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "agent_layout.display_limit"
    );
    assert_eq!(runtime.close_operations["retained-close"].phase, "refused");
    assert!(runtime.panes_closing.is_empty());
}

fn capacity_close_request(runtime: &Runtime, key: &str) -> live::CloseCaptureRequest {
    let tab = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|w| &w.checkouts)
        .flat_map(|c| &c.tabs)
        .find(|tab| tab.id.as_deref() == Some("w-order:t1"))
        .unwrap();
    let mut context = runtime.close_context(tab).unwrap();
    context.replacement_shell = true;
    live::CloseCaptureRequest {
        failed_terminals: Vec::new(),
        retain_for_reopen: true,
        key: key.into(),
        connection_generation: runtime.live_generation,
        context,
        panes: runtime.closed_panes(tab),
        target: live::CloseCaptureTarget::Tab {
            tab_id: "w-order:t1".into(),
        },
    }
}

#[test]
fn replacement_unknown_claim_survives_refusal_and_dismissal() {
    let (mut runtime, _) = setup();
    let ids = (1..=63)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    ingest(&mut runtime, &tabs);
    let request = capacity_close_request(&runtime, "uncertain-replacement");
    assert!(runtime.reserve_agent_effect("/agent-groups", "close:uncertain-replacement"));
    runtime.ensure_pending_close_from_request(&request);
    let operation = runtime.close_operations.get_mut(&request.key).unwrap();
    operation.phase = "transmitting".into();
    operation.replacement_effect_started = true;
    runtime.ingest_close_effect_result(
        &live::CloseEffectRequest {
            safety: None,
            failed_terminals: Vec::new(),
            retain_for_reopen: true,
            key: request.key.clone(),
            connection_generation: runtime.live_generation,
            target: request.target.clone(),
            replacement: Some(request.context.clone()),
            allow_replacement_create: true,
        },
        Err(hide_herdr_client::ApiError::Remote {
            code: "replacement_failed".into(),
            message: "created but acknowledgement lost".into(),
        }),
    );
    ingest(&mut runtime, &tabs);
    assert!(!runtime.admit_agent_tab("/agent-groups"));
    assert!(
        runtime.reserve_agent_effect("/agent-groups", "close:uncertain-replacement"),
        "the same intent reuses its own claim"
    );
    runtime.dismiss_agent_close(&request.key);
    assert!(
        !runtime.admit_agent_tab("/agent-groups"),
        "dismiss is not proof that the effect did not happen"
    );
}

#[test]
fn reopen_unknown_claim_survives_error_and_retry_launch_failure_then_transfers_to_placement() {
    let (mut runtime, _) = setup();
    let ids = (1..=63)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    let tabs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    ingest(&mut runtime, &tabs);
    let close = capacity_close_request(&runtime, "uncertain-reopen");
    let item = ClosedItem::Tab {
        key: close.key.clone(),
        context: close.context,
        panes: close.panes,
        layout: crate::recent_closed::ClosedLayout {
            workspace_id: "w-order".into(),
            tab_id: "retired".into(),
            zoomed: false,
            focused_pane_id: "retired:p".into(),
            root: crate::recent_closed::ClosedLayoutNode::Pane {
                pane_id: Some("retired:p".into()),
                label: None,
                cwd: Some("/agent-groups".into()),
                command: None,
                env: BTreeMap::new(),
            },
        },
    };
    runtime.push_recent_closed(item.clone());
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item,
        workspace_exists: true,
        tab_exists: false,
        fallback_pane_id: None,
        owner: None,
    };
    assert!(runtime.reserve_agent_effect("/agent-groups", "reopen:uncertain-reopen"));
    runtime.reopen_in_flight = Some(close.key.clone());
    runtime.ingest_reopen_result(
        &request,
        Err("layout applied but acknowledgement lost".into()),
    );
    ingest(&mut runtime, &tabs);
    assert!(!runtime.admit_agent_tab("/agent-groups"));
    runtime.reopen_closed(); // No fixture live worker: this retry cannot start.
    assert!(!runtime.admit_agent_tab("/agent-groups"));
    runtime.reopen_in_flight = Some(close.key);
    runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: Some("w-order:restored".into()),
            consumed: true,
            focused_pane_id: None,
            notices: vec![],
            agents: Vec::new(),
        })),
    );
    assert_eq!(runtime.pending_agent_admissions("/agent-groups"), 0);
    assert!(
        !runtime.admit_agent_tab("/agent-groups"),
        "known placement keeps the slot until topology arrives"
    );
    let mut restored = tabs;
    restored.push("w-order:restored");
    ingest(&mut runtime, &restored);
    assert_eq!(layout(&runtime).waiting, 0);
    assert!(layout(&runtime).tree.display("w-order:restored").is_some());
}

#[test]
fn cross_machine_lineage_updates_agent_tab_placement_in_every_arrival_order() {
    // B16: a resolved child is a sidebar canvas, never a normal Agent tab.
    for capacity in [None, Some(0), Some(1)] {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let (mut runtime, _) = setup();
            let normal_count = capacity.map_or(1, |reserved| 64 - reserved);
            let mut tabs = (1..=normal_count + 1)
                .filter(|n| *n != 2)
                .map(|n| format!("w-order:t{n}"))
                .collect::<Vec<_>>();
            tabs.push("w-order:t2".into());
            let refs = tabs.iter().map(String::as_str).collect::<Vec<_>>();
            if capacity == Some(1) {
                assert!(runtime.reserve_agent_effect("/agent-groups", "pending-new"));
            }
            let mut local = tab_order_payload("/agent-groups", &refs, &refs, "w-order:t1");
            local.agents = serde_json::from_value(
                crate::sidebar::owned_label_fixture::<serde_json::Value>(
                    serde_json::json!({"agents": [{
                        "id":"child", "pane_id":"w-order:t2:p", "agent_status":"working",
                        "state_change_seq":1, "spawned_from_pane_id":"parent",
                        "spawned_from_machine_id":"machine-mini", "tokens":{"task":"Child"}
                    }]}),
                )
                .unwrap()["agents"]
                    .clone(),
            )
            .unwrap();
            let remote = RemoteSessionSnapshot {
            workspaces: vec![],
            agents: project_agents(
                crate::sidebar::owned_label_fixture(serde_json::json!({"agents":[{
                    "id":"parent", "pane_id":"remote:mini:pane:parent", "agent_status":"working",
                    "state_change_seq":1, "tokens":{"task":"Parent"}
                }]}))
                .unwrap(),
            )
            .agents,
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: vec![],
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        };
            runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
                target_id: "mini".into(),
                state: "connected".into(),
                message: None,
                herdr_version: Some("0.9.1".into()),
                session: None,
                files: RemoteFileListSnapshot::idle(),
                catalog: Default::default(),
                unreachable_since_unix_ms: None,
            });
            for step in order {
                match step {
                    0 => {
                        runtime.ingest_session(Ok(local.clone()));
                    }
                    1 => {
                        runtime.ingest_remote_session("mini", Ok(remote.clone()));
                    }
                    _ => {
                        runtime.ingest_device_machine_id("mini", Ok("machine-mini".into()));
                    }
                }
            }
            let assert_placement = |runtime: &Runtime, delegated: bool| {
                let child = runtime
                    .snapshot
                    .navigator
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == "w-order:t2:p")
                    .unwrap();
                assert_eq!(child.delegated, delegated, "arrival order {order:?}");
                let checkout = runtime
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|w| &w.checkouts)
                    .find(|c| c.path == "/agent-groups")
                    .unwrap();
                let tab = checkout
                    .tabs
                    .iter()
                    .find(|t| t.id.as_deref() == Some("w-order:t2"))
                    .unwrap();
                assert_eq!(tab.delegated, delegated, "tab ownership {order:?}");
                assert_eq!(
                    checkout.strip.iter().any(|t| t.source_id == "w-order:t2"),
                    !delegated
                );
                assert_eq!(
                    layout(runtime).tree.display("w-order:t2").is_some(),
                    !delegated && capacity.is_none()
                );
            };
            assert_placement(&runtime, true);
            runtime.dispatch_json(&operator_focus_event("w-order:t2:p"));
            runtime.sync_workspace_view();
            assert_eq!(layout(&runtime).active(), Some("w-order:t2"));
            assert!(layout(&runtime).tree.display("w-order:t2").is_none());
            runtime.ingest_session(Ok(local.clone()));
            runtime.sync_workspace_view();
            assert_eq!(
                runtime.snapshot.terminal.pane_id.as_deref(),
                Some("w-order:t2:p"),
                "stable delegated canvas at capacity {capacity:?}"
            );
            assert_eq!(layout(&runtime).active(), Some("w-order:t2"));
            runtime.ingest_device_machine_id("mini", Err("disconnected".into()));
            assert_placement(&runtime, false);
            if capacity.is_some() {
                assert_eq!(layout(&runtime).waiting, 1);
                assert_eq!(
                    runtime.snapshot.terminal.pane_id.as_deref(),
                    Some("w-order:t1:p")
                );
                assert_eq!(layout(&runtime).active(), Some("w-order:t1"));
                assert_eq!(
                    runtime.focused_visible_tab_id().as_deref(),
                    Some("w-order:t1")
                );
            }
            runtime.ingest_device_machine_id("mini", Ok("machine-mini".into()));
            assert_placement(&runtime, true);
            // A later local topology pass must not temporarily re-admit the child.
            runtime.ingest_session(Ok(local));
            assert_placement(&runtime, true);
        }
    }
}

// A close takes its tab out of the Agent areas when it is approved and puts
// it back when it does not happen (docs/UI_BEHAVIOR.md, Agent areas).

/// `setup` with a Herdr connection whose workers cannot report back, so each
/// close result reaches the runtime only when the test hands it over.
fn live_setup() -> (Runtime, String) {
    let (runtime, checkout) = live_tab_order_runtime("/agent-groups");
    let mut runtime = with_new_views(runtime, "agent-groups-close");
    ingest(&mut runtime, &["w-order:t1", "w-order:t2", "w-order:t3"]);
    (runtime, checkout)
}

/// Each Agent area a snapshot read publishes, as the tab it shows and the
/// chips in its strip.
fn drawn(runtime: &mut Runtime) -> Vec<(Option<String>, Vec<String>)> {
    fn walk(
        node: &crate::split_tree::Node<crate::agent_layout::Tab>,
        out: &mut Vec<(Option<String>, Vec<String>)>,
    ) {
        match node {
            crate::split_tree::Node::Area(area) => out.push((
                area.active.clone(),
                area.displays.iter().map(|tab| tab.id.clone()).collect(),
            )),
            crate::split_tree::Node::Split(split) => {
                walk(&split.first, out);
                walk(&split.second, out);
            }
        }
    }
    runtime.sync_workspace_view();
    let mut areas = Vec::new();
    walk(
        &runtime
            .snapshot()
            .workspace_view
            .as_ref()
            .expect("the front Workspace's view")
            .agent_layout
            .as_ref()
            .expect("this machine's Agent areas")
            .root,
        &mut areas,
    );
    areas
}

fn area(shown: &str, tabs: &[&str]) -> (Option<String>, Vec<String>) {
    (
        Some(shown.to_owned()),
        tabs.iter().map(|tab| (*tab).to_owned()).collect(),
    )
}

/// The left area holds t1 and t3, the right one t2, and the operator is on
/// t1, having been on t3 before it.
fn two_areas_on_t1() -> (Runtime, String) {
    let (mut runtime, checkout) = live_setup();
    action(
        &mut runtime,
        serde_json::json!({"action":"split","tab_id":"w-order:t2","area_id":"a1","edge":"right","request_id":"split-1"}),
    );
    for tab in ["w-order:t3", "w-order:t1"] {
        action(
            &mut runtime,
            serde_json::json!({"action":"focus","tab_id":tab}),
        );
    }
    // Herdr never answers this fixture, so the focuses above wait until they
    // expire; a settled session has none, and a later Herdr move must not
    // read as their late answer.
    runtime.expire_pending_view_focus(unix_milliseconds() + 3_600_000);
    assert_eq!(
        drawn(&mut runtime),
        [
            area("w-order:t1", &["w-order:t1", "w-order:t3"]),
            area("w-order:t2", &["w-order:t2"])
        ]
    );
    (runtime, checkout)
}

fn close_tab(runtime: &mut Runtime, tab_id: &str) {
    runtime.dispatch_json(&explorer_event(
        "close_tab",
        serde_json::json!({"tab_id": tab_id, "confirmed": false}),
    ));
}

/// Herdr's answer to the close's `layout.export`, which captures the tab as
/// a reopen item, and then to its close.
fn answer_close(runtime: &mut Runtime, result: Result<(), hide_herdr_client::ApiError>) {
    let request = runtime
        .close_operations
        .values()
        .find(|operation| operation.phase == "preparing")
        .expect("a close in progress")
        .request
        .clone();
    let live::CloseCaptureTarget::Tab { tab_id } = &request.target else {
        panic!("a tab close");
    };
    let item = ClosedItem::Tab {
        key: request.key.clone(),
        context: request.context.clone(),
        panes: request.panes.clone(),
        layout: crate::recent_closed::ClosedLayout {
            workspace_id: "w-order".into(),
            tab_id: tab_id.clone(),
            zoomed: false,
            focused_pane_id: format!("{tab_id}:p"),
            root: crate::recent_closed::ClosedLayoutNode::Pane {
                pane_id: Some(format!("{tab_id}:p")),
                label: None,
                cwd: Some("/agent-groups".into()),
                command: None,
                env: BTreeMap::new(),
            },
        },
    };
    let (_, effects) = runtime
        .ingest_close_capture_result(&request, Ok(live::CloseCaptureOutcome { item: Some(item) }));
    let [effect] = effects.as_slice() else {
        panic!("one close effect, got {effects:?}");
    };
    runtime.ingest_close_effect_result(effect, result);
}

/// The keyboard follows the area, not Herdr's tab order: t2 is next in
/// Herdr's order but sits in the other area.
#[test]
fn a_closing_tab_leaves_its_area_at_once_and_a_refusal_puts_it_and_the_operator_back() {
    let (mut runtime, checkout) = two_areas_on_t1();
    let before = drawn(&mut runtime);

    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(
        drawn(&mut runtime),
        [
            area("w-order:t3", &["w-order:t3"]),
            area("w-order:t2", &["w-order:t2"])
        ]
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t3")
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t3:p")
    );

    answer_close(
        &mut runtime,
        Err(hide_herdr_client::ApiError::Remote {
            code: "tab_not_closable".into(),
            message: "refused".into(),
        }),
    );
    assert!(
        runtime.close_operations.is_empty(),
        "the refusal ended the close"
    );
    assert_eq!(drawn(&mut runtime), before);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
}

/// Herdr focuses the closed tab's neighbour on its own; that move arrives
/// while Hide's focus on the area's next tab is unanswered, so it is read as
/// the answer and the operator stays where the screen went.
#[test]
fn an_accepted_close_stays_out_of_its_area_and_herdrs_own_next_tab_does_not_move_the_operator() {
    let (mut runtime, checkout) = two_areas_on_t1();
    let during = [
        area("w-order:t3", &["w-order:t3"]),
        area("w-order:t2", &["w-order:t2"]),
    ];
    close_tab(&mut runtime, "w-order:t1");
    answer_close(&mut runtime, Ok(()));
    assert_eq!(
        runtime.close_operations.values().next().unwrap().phase,
        "awaiting_topology"
    );
    assert_eq!(drawn(&mut runtime), during);

    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &["w-order:t2", "w-order:t3"],
        &["w-order:t2", "w-order:t3"],
        "w-order:t2",
    )));
    assert!(runtime.close_operations.is_empty(), "the close completed");
    assert_eq!(drawn(&mut runtime), during);
    assert!(layout(&runtime).tree.display("w-order:t1").is_none());
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t3")
    );
}

/// A close of a tab the operator is not on, by its chip or by its only
/// pane, takes the chip and leaves the operator where they are.
#[test]
fn closing_a_tab_in_the_background_takes_its_chip_and_leaves_the_operator_in_place() {
    let (mut runtime, checkout) = live_setup();
    action(
        &mut runtime,
        serde_json::json!({"action":"focus","tab_id":"w-order:t2"}),
    );
    close_tab(&mut runtime, "w-order:t3");
    runtime.dispatch_json(&explorer_event(
        "close_pane",
        serde_json::json!({"pane_id": "w-order:t1:p", "confirmed": false}),
    ));
    assert_eq!(runtime.close_operations.len(), 2);
    assert_eq!(drawn(&mut runtime), [area("w-order:t2", &["w-order:t2"])]);
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
}

/// A close that must open a replacement shell first keeps its tab, so the
/// area never stands empty while that shell is made.
#[test]
fn a_close_that_needs_a_replacement_shell_keeps_its_tab_in_place() {
    let (runtime, _) = live_tab_order_runtime("/agent-groups");
    let mut runtime = with_new_views(runtime, "agent-groups-shell");
    ingest(&mut runtime, &["w-order:t1"]);
    let primary = crate::domain::WorktreeProjection {
        repo_key: "repo".into(),
        repo_name: "repo".into(),
        repo_root: "/repo".into(),
        checkout_path: "/repo".into(),
        is_linked_worktree: false,
    };
    runtime
        .herdr_worktrees
        .insert("w-order".into(), primary.clone());
    runtime.herdr_worktrees.insert(
        "linked".into(),
        crate::domain::WorktreeProjection {
            is_linked_worktree: true,
            ..primary
        },
    );
    close_tab(&mut runtime, "w-order:t1");
    let operation = runtime.close_operations.values().next().expect("a close");
    assert!(operation.request.context.replacement_shell);
    assert_eq!(drawn(&mut runtime), [area("w-order:t1", &["w-order:t1"])]);
}

/// Another registered folder of this machine; returns its checkout id.
pub(super) fn register_folder(runtime: &mut Runtime, name: &str) -> String {
    let id = format!("workspace:{name}");
    let path = format!("/{name}");
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(WorkspaceRegistration {
            primary_checkout_id: None,
            id: id.clone(),
            label: name.to_owned(),
            path: path.clone(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        });
    runtime.rebuild_catalog();
    workspace::checkout_id_for_path(&id, Path::new(&path))
}

/// Adds Herdr workspace `w-else`, holding the one tab `w-else:t1` in
/// /elsewhere, to a session.
pub(super) fn add_elsewhere_tab(payload: &mut SessionSnapshotPayload) {
    let elsewhere: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [],
            "workspaces": [{"workspace_id": "w-else", "label": "else", "active_tab_id": "w-else:t1"}],
            "tabs": [{"workspace_id": "w-else", "tab_id": "w-else:t1", "label": "", "number": 1}],
            "panes": [{"pane_id": "w-else:t1:p", "cwd": "/elsewhere"}],
            "layouts": [{
                "workspace_id": "w-else",
                "tab_id": "w-else:t1",
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w-else:t1:p",
                "panes": [{"pane_id": "w-else:t1:p", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            }]
        }))
        .expect("elsewhere session payload");
    payload.workspaces.extend(elsewhere.workspaces);
    payload.tabs.extend(elsewhere.tabs);
    payload.panes.extend(elsewhere.panes);
    payload.layouts.extend(elsewhere.layouts);
}

pub(super) fn choose_checkout(runtime: &mut Runtime, workspace_id: &str, checkout_id: &str) {
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": workspace_id, "checkout_id": checkout_id}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

fn front(runtime: &Runtime) -> Option<String> {
    runtime.snapshot().navigator.focused_checkout_id.clone()
}

/// The Workspace in front and whether a reload opens on it (D-11).
fn resumed(runtime: &mut Runtime) -> (String, bool) {
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.clone().expect("front");
    (view.path, view.resumed)
}

/// /agent-groups holding `tabs` in Herdr workspace w-order, beside
/// /elsewhere's one tab.
fn session_with_elsewhere(tabs: &[&str]) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload("/agent-groups", tabs, tabs, "w-order:t1");
    add_elsewhere_tab(&mut payload);
    payload
}

/// The operator was in /elsewhere, which holds a tab, and came to
/// /agent-groups, holding `tabs`, where they are on t1. /empty, a registered
/// folder with no tab, is there to visit too.
fn entered_from_elsewhere(tabs: &[&str]) -> (Runtime, String, String) {
    let (mut runtime, checkout) = live_tab_order_runtime("/agent-groups");
    let elsewhere = register_folder(&mut runtime, "elsewhere");
    register_folder(&mut runtime, "empty");
    let mut runtime = with_new_views(runtime, "agent-groups-leave");
    runtime.ingest_session(Ok(session_with_elsewhere(tabs)));
    choose_checkout(&mut runtime, "workspace:elsewhere", &elsewhere);
    choose_checkout(&mut runtime, "workspace:order", &checkout);
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
    (runtime, checkout, elsewhere)
}

fn refuse_close(runtime: &mut Runtime) {
    answer_close(
        runtime,
        Err(hide_herdr_client::ApiError::Remote {
            code: "tab_not_closable".into(),
            message: "refused".into(),
        }),
    );
    assert!(runtime.close_operations.is_empty(), "the refusal ended it");
}

/// Closing the last tab of the checkout in front is leaving it: the screen
/// goes back to the checkout the operator was in before, in the frame the
/// close is approved, and a reload opens there. A refusal, even after Herdr
/// has reported its session again meanwhile, brings the tab back, and the
/// operator back to it.
#[test]
fn closing_the_last_tab_goes_back_to_the_checkout_before_and_a_refusal_comes_back() {
    let (mut runtime, checkout, elsewhere) = entered_from_elsewhere(&["w-order:t1"]);

    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(front(&runtime).as_deref(), Some(elsewhere.as_str()));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-else:t1:p")
    );
    assert_eq!(
        resumed(&mut runtime),
        ("/elsewhere".to_owned(), true),
        "a reload opens where the close went"
    );

    runtime.ingest_session(Ok(session_with_elsewhere(&["w-order:t1"])));
    assert_eq!(front(&runtime).as_deref(), Some(elsewhere.as_str()));
    refuse_close(&mut runtime);
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
    assert_eq!(drawn(&mut runtime), [area("w-order:t1", &["w-order:t1"])]);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );
    assert_eq!(resumed(&mut runtime), ("/agent-groups".to_owned(), true));
}

/// Closing the only pane of the last tab is closing that tab, and goes back
/// the same way; a refusal that arrives after the operator chose somewhere
/// else leaves them there.
#[test]
fn closing_the_last_pane_goes_back_too_and_a_late_refusal_leaves_a_later_choice_alone() {
    let (mut runtime, _, elsewhere) = entered_from_elsewhere(&["w-order:t1"]);
    runtime.dispatch_json(&explorer_event(
        "close_pane",
        serde_json::json!({"pane_id": "w-order:t1:p", "confirmed": false}),
    ));
    assert_eq!(front(&runtime).as_deref(), Some(elsewhere.as_str()));

    let empty = workspace::checkout_id_for_path("workspace:empty", Path::new("/empty"));
    choose_checkout(&mut runtime, "workspace:empty", &empty);
    let request = runtime
        .close_operations
        .values()
        .next()
        .expect("the close")
        .request
        .clone();
    let _ = runtime
        .ingest_close_capture_result(&request, Err("the capture could not be read".to_owned()));
    assert!(
        runtime
            .close_operations
            .values()
            .all(|operation| operation.phase == "failed")
    );
    assert_eq!(front(&runtime).as_deref(), Some(empty.as_str()));
}

/// The checkout to go back to is the most recent one with a tab left: a
/// folder with no tab is passed over, and with no such checkout the screen
/// stays on the Agent area's empty state. A close that leaves another tab
/// moves only to that tab.
#[test]
fn closing_the_last_tab_passes_over_a_checkout_with_no_tab_and_stays_with_none() {
    let (mut runtime, checkout, elsewhere) = entered_from_elsewhere(&["w-order:t1"]);
    let empty = workspace::checkout_id_for_path("workspace:empty", Path::new("/empty"));
    choose_checkout(&mut runtime, "workspace:empty", &empty);
    choose_checkout(&mut runtime, "workspace:order", &checkout);
    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(front(&runtime).as_deref(), Some(elsewhere.as_str()));

    let (runtime, checkout) = live_tab_order_runtime("/agent-groups");
    let mut runtime = with_new_views(runtime, "agent-groups-stay");
    let empty = register_folder(&mut runtime, "empty");
    ingest(&mut runtime, &["w-order:t1"]);
    choose_checkout(&mut runtime, "workspace:empty", &empty);
    choose_checkout(&mut runtime, "workspace:order", &checkout);
    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(runtime.close_operations.len(), 1);
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));

    let (mut runtime, checkout, _) =
        entered_from_elsewhere(&["w-order:t1", "w-order:t2", "w-order:t3"]);
    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
    let pane = runtime.snapshot().terminal.pane_id.clone();
    assert!(
        matches!(pane.as_deref(), Some("w-order:t2:p" | "w-order:t3:p")),
        "the keyboard went to a tab left, got {pane:?}"
    );
}

/// A checkout whose project is being removed is no place to go back to,
/// even while its tabs are still open.
#[test]
fn closing_the_last_tab_passes_over_a_checkout_being_removed() {
    let (mut runtime, checkout, _) = entered_from_elsewhere(&["w-order:t1"]);
    runtime
        .workspace_removals_in_flight
        .insert("workspace:elsewhere".to_owned());
    close_tab(&mut runtime, "w-order:t1");
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
}

/// A close that first opens a replacement shell keeps its checkout busy, so
/// it leaves the screen where it is.
#[test]
fn a_close_that_opens_a_replacement_shell_stays_on_its_checkout() {
    let (mut runtime, checkout, _) = entered_from_elsewhere(&["w-order:t1"]);
    let primary = crate::domain::WorktreeProjection {
        repo_key: "repo".into(),
        repo_name: "repo".into(),
        repo_root: "/repo".into(),
        checkout_path: "/repo".into(),
        is_linked_worktree: false,
    };
    runtime
        .herdr_worktrees
        .insert("w-order".into(), primary.clone());
    runtime.herdr_worktrees.insert(
        "linked".into(),
        crate::domain::WorktreeProjection {
            is_linked_worktree: true,
            ..primary
        },
    );
    close_tab(&mut runtime, "w-order:t1");
    let operation = runtime.close_operations.values().next().expect("a close");
    assert!(operation.request.context.replacement_shell);
    assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
}

fn reopen(runtime: &mut Runtime) {
    runtime.dispatch_json(&explorer_event("reopen_closed", serde_json::json!({})));
}

/// The chip is gone before Herdr confirms the close, so a reopen pressed at
/// once is offered, waits for that close, and starts when it is confirmed.
#[test]
fn a_reopen_pressed_while_the_close_settles_starts_once_herdr_confirms_the_close() {
    let (mut runtime, _) = two_areas_on_t1();
    close_tab(&mut runtime, "w-order:t1");
    answer_close(&mut runtime, Ok(()));
    assert!(runtime.snapshot().recent_closed.can_reopen);

    reopen(&mut runtime);
    assert!(
        !runtime.snapshot().recent_closed.restoring,
        "nothing reopens before the close is confirmed"
    );
    assert!(runtime.snapshot().recent_closed.notices.is_empty());
    assert!(
        !runtime.snapshot().recent_closed.can_reopen,
        "one reopen waits"
    );

    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &["w-order:t2", "w-order:t3"],
        &["w-order:t2", "w-order:t3"],
        "w-order:t3",
    )));
    assert!(
        runtime.snapshot().recent_closed.restoring,
        "the reopen started"
    );
}

/// A refused close brings its tab back, so the reopen waiting for it is
/// dropped rather than reaching the older close still on the stack.
#[test]
fn a_reopen_waiting_for_a_refused_close_is_dropped_and_never_reopens_an_older_close() {
    let (mut runtime, _) = two_areas_on_t1();
    close_tab(&mut runtime, "w-order:t3");
    answer_close(&mut runtime, Ok(()));
    runtime.ingest_session(Ok(tab_order_payload(
        "/agent-groups",
        &["w-order:t1", "w-order:t2"],
        &["w-order:t1", "w-order:t2"],
        "w-order:t1",
    )));
    assert_eq!(
        runtime.snapshot().recent_closed.count,
        1,
        "t3 can be reopened"
    );

    close_tab(&mut runtime, "w-order:t1");
    reopen(&mut runtime);
    answer_close(
        &mut runtime,
        Err(hide_herdr_client::ApiError::Remote {
            code: "tab_not_closable".into(),
            message: "refused".into(),
        }),
    );
    assert!(
        runtime.close_operations.is_empty(),
        "the refusal ended the close"
    );
    assert!(!runtime.snapshot().recent_closed.restoring);
    assert_eq!(runtime.snapshot().recent_closed.count, 1);
}

/// Herdr makes a reopened tab inside a seed tab it then replaces
/// (`workspace.create`, then `layout.apply`), and the seed's focus can reach
/// the core after the reopen's own answer, when it is followed like any move
/// of Herdr's. When the seed goes, the keyboard stays on the restored tab the
/// checkout shows, never on the other area's tab Herdr's keyboard names
/// (#425, `agent-tab-groups.spec.ts` "New tab and Reopen use the
/// requested area").
#[test]
fn a_followed_reopen_seed_that_goes_leaves_the_operator_on_the_restored_tab() {
    let (mut runtime, checkout) = two_areas_on_t1();
    close_tab(&mut runtime, "w-order:t3");
    answer_close(&mut runtime, Ok(()));
    let after_close = ["w-order:t1", "w-order:t2"];
    let mut herdr = HerdrMoves::new();
    runtime.ingest_session(Ok(herdr.after(
        &[],
        tab_order_payload("/agent-groups", &after_close, &after_close, "w-order:t1"),
    )));
    action(
        &mut runtime,
        serde_json::json!({"action":"focus","tab_id":"w-order:t2"}),
    );
    // This fixture's Herdr never answers, so its lane never moves on by
    // itself; it is moved on here as Herdr's answer would, so the click
    // leaves, and only a request that has left is confirmed.
    let _ = runtime.advance_lane();
    // Herdr confirms the click on the right area's tab.
    runtime.ingest_session(Ok(herdr.after(
        &["w-order:t2"],
        tab_order_payload("/agent-groups", &after_close, &after_close, "w-order:t2"),
    )));
    let item = runtime.recent_closed.back().cloned().expect("t3 to reopen");
    reopen(&mut runtime);
    assert!(runtime.reopen_in_flight.is_some(), "the reopen started");
    runtime.ingest_reopen_result(
        &live::ReopenRequest {
            codex_daemon: Default::default(),
            item,
            workspace_exists: true,
            tab_exists: false,
            fallback_pane_id: None,
            owner: None,
        },
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: Some("w-order:t4".into()),
            consumed: true,
            focused_pane_id: Some("w-order:t4:p".into()),
            notices: vec![],
            agents: Vec::new(),
        })),
    );

    // The seed's creation and its focus reach the core after that answer.
    let seeded = ["w-order:t1", "w-order:t2", "w-order:t8"];
    let mut seed = tab_order_payload("/agent-groups", &seeded, &seeded, "w-order:t8");
    seed.tab_focus = Some(crate::sidebar::SessionTabFocus {
        generation: 1,
        workspace_id: "w-order".into(),
        tab_id: "w-order:t8".into(),
        revision: 1,
        creation: true,
    });
    runtime.ingest_session(Ok(herdr.after(&[], seed.clone())));
    let focus = seed.tab_focus.as_mut().unwrap();
    focus.creation = false;
    focus.revision = 2;
    runtime.ingest_session(Ok(herdr.after(&["w-order:t8"], seed)));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t8:p"),
        "the seed's focus was followed"
    );

    // The seed goes and the restored tab arrives, with Herdr's keyboard back
    // on the right area's tab.
    let restored = ["w-order:t1", "w-order:t2", "w-order:t4"];
    runtime.ingest_session(Ok(herdr.after(
        &["w-order:t2"],
        tab_order_payload("/agent-groups", &restored, &restored, "w-order:t2"),
    )));
    assert_eq!(
        drawn(&mut runtime),
        [
            area("w-order:t4", &["w-order:t1", "w-order:t4"]),
            area("w-order:t2", &["w-order:t2"])
        ]
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some("w-order:t4")
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t4:p")
    );
}

fn order(runtime: &Runtime) -> Vec<String> {
    layout(runtime).tree.areas()[0]
        .displays
        .iter()
        .map(|tab| tab.id.clone())
        .collect()
}

fn pending_fork(runtime: &mut Runtime, source: &str, request_id: Option<&str>) {
    runtime.forks_in_flight.insert(
        source.into(),
        super::agents::PendingFork {
            press: runtime.forks_in_flight.len() as u64 + 1,
            generation: runtime.live_generation,
            issued_serial: runtime.control_lane.issued_serial(),
            origin: super::events::ForkOrigin::Operator,
            tab_id: None,
            pane_id: None,
            tab_label: Some("claude".into()),
            focused: None,
            kind: "claude".into(),
            name: "fork-fixture".into(),
            launch: Default::default(),
        },
    );
    if let Some(request_id) = request_id {
        runtime.input_requests.open(
            request_id,
            super::terminal_input::InputOrigin::Fork(source.into()),
            std::time::Instant::now(),
        );
        runtime.sync_input_requests();
    }
}

fn input_state(runtime: &Runtime, request_id: &str) -> Option<InputRequestState> {
    runtime
        .snapshot
        .terminal
        .input_requests
        .iter()
        .find(|request| request.request_id == request_id)
        .map(|request| request.state)
}

// PRD fork-operator-root B2: `Fork agent` puts the fork's tab right of the
// source's tab, whether Herdr's session or the worker's answer comes first.
#[test]
fn a_fork_tab_lands_right_of_its_source_whichever_answer_comes_first() {
    let all = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    // The worker's answer first: the tab waits for its place.
    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, "w-order:t1:p", None);
    runtime.ingest_fork_pane("w-order:t1:p", Some("w-order:t4"), "w-order:t4:p");
    ingest(&mut runtime, &all);
    assert_eq!(
        order(&runtime),
        ["w-order:t1", "w-order:t4", "w-order:t2", "w-order:t3"]
    );
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_placements
            .is_empty()
    );

    // Herdr's session first: the appended tab moves next to its source.
    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, "w-order:t2:p", None);
    ingest(&mut runtime, &all);
    assert_eq!(order(&runtime), all);
    runtime.ingest_fork_pane("w-order:t2:p", Some("w-order:t4"), "w-order:t4:p");
    assert_eq!(
        order(&runtime),
        ["w-order:t1", "w-order:t2", "w-order:t4", "w-order:t3"]
    );

    // A source whose tab closed meanwhile leaves the tab at the end.
    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, "w-order:t9:p", None);
    runtime.ingest_fork_pane("w-order:t9:p", Some("w-order:t4"), "w-order:t4:p");
    ingest(&mut runtime, &all);
    assert_eq!(order(&runtime), all);
}

// PRD fork-operator-root B1, B4, B5, B7: the fork takes the keyboard when
// Herdr makes its pane, and what was typed meanwhile reaches its agent once
// it has started; a view chosen after the press keeps the keyboard, and a
// failed fork drops the keys, its placement, and gives the source back.
#[test]
fn a_fork_takes_the_keyboard_when_its_pane_exists_and_its_keys_when_it_starts() {
    let source = "w-order:t1:p";
    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, source, Some("fork-keys"));
    runtime.ingest_fork_pane(source, None, "w-order:t3:p");
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t3:p")
    );
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Pending)
    );
    assert_eq!(
        runtime.snapshot.terminal.input_requests[0]
            .held_for
            .as_deref(),
        Some("w-order:t3:p")
    );
    runtime.ingest_fork_result(source, Ok("w-order:t3:p".into()), 1);
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Ready)
    );
    assert!(runtime.forks_in_flight.is_empty());

    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, source, Some("fork-keys"));
    runtime.last_view_intent_serial = runtime.control_lane.issued_serial() + 1;
    let before = runtime.snapshot.focused.pane_id.clone();
    runtime.ingest_fork_pane(source, None, "w-order:t3:p");
    assert_eq!(runtime.snapshot.focused.pane_id, before);
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Discarded)
    );
    runtime.ingest_fork_result(source, Ok("w-order:t3:p".into()), 1);
    assert_eq!(runtime.snapshot.focused.pane_id, before);

    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, "w-order:t2:p", Some("fork-keys"));
    runtime.ingest_fork_pane("w-order:t2:p", Some("w-order:t4"), "w-order:t4:p");
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t4:p")
    );
    runtime.ingest_fork_result("w-order:t2:p", Err("start refused".into()), 1);
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Discarded)
    );
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.fork_failed")
    );
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_placements
            .is_empty()
    );
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
}

// PRD fork-operator-root B6: a second press while the pane's fork is starting,
// in either placement, is refused by name, and its keys go nowhere.
#[test]
fn a_second_fork_of_a_pane_still_forking_is_refused_whichever_item_asked() {
    let (mut runtime, _) = setup();
    let mut agents = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
            {"pane_id":"w-order:t1:p","agent":"claude","agent_status":"idle","state_change_seq":1}
        ]}))
        .unwrap(),
    )
    .agents;
    agents[0].session_id = Some("3f2b1c00-0000-4000-8000-000000000001".into());
    runtime.snapshot.navigator.agents = agents;
    pending_fork(&mut runtime, "w-order:t1:p", None);
    runtime.dispatch_json(&explorer_event(
        "fork_pane",
        serde_json::json!({"pane_id":"w-order:t1:p","placement":"side","origin":"operator","request_id":"second"}),
    ));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.fork_already_running")
    );
    assert_eq!(
        input_state(&runtime, "second"),
        Some(InputRequestState::Discarded)
    );
    assert_eq!(runtime.forks_in_flight.len(), 1);
}

// PRD fork-operator-root D-10: one fork's keyboard move is not the operator
// choosing a view, so of two forks starting at once the later press keeps the
// keyboard, whichever pane Herdr makes first.
#[test]
fn of_two_forks_starting_at_once_the_later_press_keeps_the_keyboard() {
    for later_first in [false, true] {
        let (mut runtime, _) = setup();
        pending_fork(&mut runtime, "w-order:t1:p", Some("first"));
        pending_fork(&mut runtime, "w-order:t2:p", Some("second"));
        let earlier =
            |runtime: &mut Runtime| runtime.ingest_fork_pane("w-order:t1:p", None, "w-order:t9:p");
        let later =
            |runtime: &mut Runtime| runtime.ingest_fork_pane("w-order:t2:p", None, "w-order:t8:p");
        if later_first {
            later(&mut runtime);
            earlier(&mut runtime);
        } else {
            earlier(&mut runtime);
            later(&mut runtime);
        }
        assert_eq!(
            runtime.snapshot.focused.pane_id.as_deref(),
            Some("w-order:t8:p"),
            "later first: {later_first}"
        );
        assert_eq!(
            input_state(&runtime, "second"),
            Some(InputRequestState::Pending)
        );
        assert_eq!(
            input_state(&runtime, "first"),
            Some(if later_first {
                InputRequestState::Discarded
            } else {
                InputRequestState::Pending
            })
        );
    }
}

// PRD fork-operator-root B2, D-10: a fork's tab the full Agent area cannot
// show waits in the strip as a new tab does, and does not take the keys.
#[test]
fn a_fork_tab_the_full_agent_area_cannot_show_does_not_take_the_keyboard() {
    let (mut runtime, _) = setup();
    let ids = (1..=64)
        .map(|n| format!("w-order:t{n}"))
        .collect::<Vec<_>>();
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    assert_eq!(layout(&runtime).tree.display_count(), 64);
    pending_fork(&mut runtime, "w-order:t1:p", Some("fork-keys"));
    let before = runtime.snapshot.focused.pane_id.clone();
    runtime.ingest_fork_pane("w-order:t1:p", Some("w-order:t65"), "w-order:t65:p");
    assert_eq!(runtime.snapshot.focused.pane_id, before);
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Discarded)
    );
}

// A pane the replaced Herdr made is neither placed nor focused on the
// connection that replaced it.
#[test]
fn a_fork_pane_from_a_replaced_herdr_is_not_placed_or_focused() {
    let (mut runtime, _) = setup();
    pending_fork(&mut runtime, "w-order:t1:p", Some("fork-keys"));
    runtime.live_generation += 1;
    let before = runtime.snapshot.focused.pane_id.clone();
    assert!(!runtime.ingest_fork_pane("w-order:t1:p", Some("w-order:t4"), "w-order:t4:p"));
    assert_eq!(runtime.snapshot.focused.pane_id, before);
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .agent_placements
            .is_empty()
    );
    runtime.ingest_fork_result(
        "w-order:t1:p",
        Err("session_fork_execution_changed".into()),
        1,
    );
    assert_eq!(runtime.snapshot.focused.pane_id, before);
    assert_eq!(
        input_state(&runtime, "fork-keys"),
        Some(InputRequestState::Discarded)
    );
}

/// /agent-groups holding `tabs` with Herdr's keyboard on `focused`'s tab,
/// beside /elsewhere's one tab, `w-else:t1`, which can hold it too; `split`
/// puts a second pane right of t1's.
fn fork_session(tabs: &[&str], focused: &str, split: Option<&str>) -> SessionSnapshotPayload {
    let active = if focused == "w-else:t1" {
        tabs[0]
    } else {
        focused
    };
    let mut payload = tab_order_payload("/agent-groups", tabs, tabs, active);
    if let Some(split) = split {
        let layout = payload
            .layouts
            .iter_mut()
            .find(|layout| layout.tab_id == "w-order:t1")
            .expect("t1 is laid out");
        let half = crate::sidebar::SessionLayoutRect {
            x: 0,
            y: 0,
            width: 40,
            height: 24,
        };
        layout.panes[0].rect = half;
        layout.panes.push(crate::sidebar::SessionLayoutPanePayload {
            pane_id: split.to_owned(),
            rect: crate::sidebar::SessionLayoutRect { x: 40, ..half },
        });
        layout.focused_pane_id = split.to_owned();
        layout.splits = serde_json::from_value(serde_json::json!([
            {"direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}
        ]))
        .expect("t1's split");
        payload.focused_pane_id = Some(split.to_owned());
        payload.panes.push(
            serde_json::from_value(serde_json::json!({"pane_id": split, "cwd": "/agent-groups"}))
                .expect("the split's pane"),
        );
    }
    add_elsewhere_tab(&mut payload);
    if focused == "w-else:t1" {
        payload.focused_workspace_id = Some("w-else".to_owned());
        payload.focused_pane_id = Some("w-else:t1:p".to_owned());
    }
    payload
}

/// The checkout in front, the pane the terminal projection is on, the tab it
/// draws, and whether it says it is waiting for that pane.
fn fork_on_screen(runtime: &Runtime) -> (Option<String>, Option<String>, Option<String>, bool) {
    let snapshot = runtime.snapshot();
    (
        snapshot.navigator.focused_checkout_id.clone(),
        snapshot.terminal.pane_id.clone(),
        snapshot
            .active_pane_layout()
            .map(|layout| layout.tab_id.clone()),
        snapshot
            .status
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind == "pane.projection_unavailable"),
    )
}

/// The operator in /elsewhere, or in /agent-groups when `from_elsewhere` is
/// false, presses Fork agent (`tab`) or Fork agent to the side (`side`) on
/// t1's pane and Herdr answers with the fork's pane, before its layout.
fn forked(from_elsewhere: bool, tab: bool) -> (Runtime, String, &'static str) {
    let (mut runtime, checkout, elsewhere) = entered_from_elsewhere(&["w-order:t1", "w-order:t2"]);
    if from_elsewhere {
        choose_checkout(&mut runtime, "workspace:elsewhere", &elsewhere);
        runtime.ingest_session(Ok(fork_session(
            &["w-order:t1", "w-order:t2"],
            "w-else:t1",
            None,
        )));
    }
    pending_fork(&mut runtime, "w-order:t1:p", Some("fork-keys"));
    let pane = if tab {
        runtime.ingest_fork_pane("w-order:t1:p", Some("w-order:t4"), "w-order:t4:p");
        "w-order:t4:p"
    } else {
        runtime.ingest_fork_pane("w-order:t1:p", None, "w-order:t1:p2");
        "w-order:t1:p2"
    };
    (runtime, checkout, pane)
}

/// The fork's keys wait for its agent, held for its pane.
fn keys_held_for(runtime: &Runtime, pane: &str) -> bool {
    runtime
        .snapshot
        .terminal
        .input_requests
        .iter()
        .any(|request| {
            request.request_id == "fork-keys"
                && request.state == InputRequestState::Pending
                && request.held_for.as_deref() == Some(pane)
        })
}

// A fork's pane is the checkout's it was made in from Herdr's answer, before
// Herdr's layout lists it, so the keyboard brings that checkout forward even
// when a focus on the pane, the fork's own or a second one (the journey's),
// comes before the catalog does. Its own tab is drawn as a created tab is and
// a split as a split Hide asked for is, so the projection never waits on it.
// With another checkout in front the journey's Grok fork waited 30 s on
// `pane.projection_unavailable` on macOS.
#[test]
fn a_fork_brings_its_checkout_forward_before_herdr_lists_its_pane() {
    let before = ["w-order:t1", "w-order:t2"];
    for from_elsewhere in [false, true] {
        let (mut runtime, checkout, pane) = forked(from_elsewhere, true);
        assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
        focus_pane(&mut runtime, pane);
        let shown = (
            Some(checkout.clone()),
            Some(pane.to_owned()),
            Some("w-order:t4".to_owned()),
            false,
        );
        runtime.ingest_session(Ok(fork_session(&before, "w-order:t1", None)));
        assert_eq!(
            fork_on_screen(&runtime),
            shown,
            "from elsewhere: {from_elsewhere}"
        );
        runtime.ingest_session(Ok(fork_session(
            &["w-order:t1", "w-order:t2", "w-order:t4"],
            "w-order:t4",
            None,
        )));
        assert_eq!(
            fork_on_screen(&runtime),
            shown,
            "from elsewhere: {from_elsewhere}"
        );
        assert!(keys_held_for(&runtime, pane));

        let (mut runtime, checkout, pane) = forked(from_elsewhere, false);
        assert_eq!(front(&runtime).as_deref(), Some(checkout.as_str()));
        focus_pane(&mut runtime, pane);
        let shown = (
            Some(checkout.clone()),
            Some(pane.to_owned()),
            Some("w-order:t1".to_owned()),
            false,
        );
        runtime.ingest_session(Ok(fork_session(&before, "w-order:t1", None)));
        assert_eq!(
            fork_on_screen(&runtime),
            shown,
            "from elsewhere: {from_elsewhere}"
        );
        runtime.ingest_session(Ok(fork_session(&before, "w-order:t1", Some(pane))));
        assert_eq!(
            fork_on_screen(&runtime),
            shown,
            "from elsewhere: {from_elsewhere}"
        );
        assert!(keys_held_for(&runtime, pane));
    }
}

// Herdr's layout of a fork made with another checkout in front, its keyboard
// on the fork's tab, is not a focus Herdr moved: Hide stays on the fork's
// pane rather than following the front checkout's own pane (the journey's
// Grok fork on Windows: `Herdr focused pane w1:p1; Hide was on w8:p3`).
#[test]
fn herdrs_layout_of_a_fork_is_not_read_as_herdr_moving_the_keyboard() {
    for tab in [true, false] {
        let (mut runtime, checkout, pane) = forked(true, tab);
        // Herdr confirms the fork's focus in milliseconds, before its layout
        // reaches Hide, so nothing Hide asked for is pending any more.
        let control = runtime
            .pane_focus_in_flight
            .clone()
            .expect("the fork's focus went to Herdr");
        let confirmed = PaneLayoutSnapshot {
            workspace_id: "w-order".to_owned(),
            tab_id: if tab { "w-order:t4" } else { "w-order:t1" }.to_owned(),
            focused_pane_id: pane.to_owned(),
            zoomed: false,
            root: PaneLayoutNodeSnapshot::Pane {
                pane_id: pane.to_owned(),
            },
        };
        runtime.complete_lane_pane_focus(control, Ok(confirmed), 4);
        let (tabs, focused, split): (&[&str], _, _) = if tab {
            (
                &["w-order:t1", "w-order:t2", "w-order:t4"],
                "w-order:t4",
                None,
            )
        } else {
            (&["w-order:t1", "w-order:t2"], "w-order:t1", Some(pane))
        };
        // A session read before Herdr moved still has it on /elsewhere; Herdr
        // then reaches the fork's tab through its workspace's active tab, the
        // source's, and a session can carry that step. Both are on the way to
        // Hide's own request, not a tab Herdr chose: a local journey run
        // showed the step taking the keyboard to the source for one frame.
        let steps: [(&[&str], &str); 3] = [
            (&["w-order:t1", "w-order:t2"], "w-else:t1"),
            (tabs, "w-order:t1"),
            (tabs, focused),
        ];
        for (tabs, focused) in steps {
            let split = split.filter(|_| focused != "w-else:t1");
            runtime.ingest_session(Ok(fork_session(tabs, focused, split)));
            let (front, selected, _, waiting) = fork_on_screen(&runtime);
            assert_eq!(
                (front, selected, waiting),
                (Some(checkout.clone()), Some(pane.to_owned()), false),
                "tab: {tab}, Herdr on {focused}"
            );
        }
        assert_eq!(
            diagnostic_count(&runtime, "pane.focus.followed"),
            0,
            "tab: {tab}"
        );
        assert_eq!(
            diagnostic_count(&runtime, "tab.focus.followed"),
            0,
            "tab: {tab}"
        );
    }
}

// What is drawn for a fork's pane lasts as long as any drawing ahead of
// Herdr: a split Herdr has not laid out by then is the unknown result a split
// Hide asked for is, after which its pane is waited on as any missing pane
// is, and a fork whose agent never started takes back what was drawn for the
// pane its worker closed.
#[test]
fn a_fork_pane_herdr_never_lays_out_is_released_at_its_bound() {
    let before = ["w-order:t1", "w-order:t2"];
    let (mut runtime, _, pane) = forked(false, false);
    runtime.tick_async_operations(unix_milliseconds() + CLOSE_STAGE_TIMEOUT_MS + 1_000);
    assert_eq!(diagnostic_count(&runtime, "pane.operation.unconfirmed"), 1);
    runtime.ingest_session(Ok(fork_session(&before, "w-order:t1", None)));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.projection_unavailable"),
        "{pane} is waited on, and says so"
    );

    for (tab, taken_back) in [
        (true, "tab.create.taken_back"),
        (false, "pane.split.taken_back"),
    ] {
        let (mut runtime, checkout, _) = forked(false, tab);
        let (_, records) = crate::diagnostics::capture(|| {
            runtime.ingest_fork_result("w-order:t1:p", Err("start refused".into()), 1)
        });
        assert!(
            records.iter().any(|record| record["kind"] == taken_back),
            "tab: {tab}"
        );
        runtime.ingest_session(Ok(fork_session(&before, "w-order:t1", None)));
        assert_eq!(ordered_tab_ids(&runtime, &checkout), before, "tab: {tab}");
        assert_eq!(
            runtime
                .snapshot()
                .active_pane_layout()
                .map(|layout| layout.pane_ids()),
            Some(vec!["w-order:t1:p"]),
            "tab: {tab}"
        );
        assert_eq!(
            input_state(&runtime, "fork-keys"),
            Some(InputRequestState::Discarded)
        );
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t1:p")
        );
    }
}

// Only the operator's own press of Fork agent takes the screen and the
// keyboard (#1271): a fork another caller asks for is made and drawn where
// it goes, and waits there.
#[test]
fn a_fork_the_operator_did_not_press_stays_behind() {
    let before = ["w-order:t1", "w-order:t2"];
    for tab in [true, false] {
        let (mut runtime, checkout, elsewhere) = entered_from_elsewhere(&before);
        choose_checkout(&mut runtime, "workspace:elsewhere", &elsewhere);
        runtime.ingest_session(Ok(fork_session(&before, "w-else:t1", None)));
        let screen = fork_on_screen(&runtime);
        pending_fork(&mut runtime, "w-order:t1:p", Some("fork-keys"));
        runtime
            .forks_in_flight
            .get_mut("w-order:t1:p")
            .expect("the fork is starting")
            .origin = super::events::ForkOrigin::Agent;
        let (tab_id, pane) = if tab {
            (Some("w-order:t4"), "w-order:t4:p")
        } else {
            (None, "w-order:t1:p2")
        };
        runtime.ingest_fork_pane("w-order:t1:p", tab_id, pane);
        runtime.ingest_session(Ok(fork_session(&before, "w-else:t1", None)));
        assert_eq!(fork_on_screen(&runtime), screen, "tab: {tab}");
        assert_eq!(
            input_state(&runtime, "fork-keys"),
            Some(InputRequestState::Discarded),
            "tab: {tab}"
        );
        if tab {
            assert!(
                ordered_tab_ids(&runtime, &checkout)
                    .iter()
                    .any(|tab| tab == "w-order:t4")
            );
        }
    }
}

// A device's pane is drawn by the device that holds it, never ahead of Herdr
// on this machine.
#[test]
fn a_fork_of_a_device_pane_draws_nothing_here() {
    let (mut runtime, _) = setup();
    let source = "remote:device-1:pane:w1:p1";
    pending_fork(&mut runtime, source, None);
    runtime.ingest_fork_pane(source, Some("w1:t2"), "remote:device-1:pane:w1:p2");
    runtime.ingest_fork_pane(source, None, "remote:device-1:pane:w1:p3");
    assert!(runtime.provisional_tabs.is_empty());
    assert!(runtime.pane_operations.is_empty());
}

fn focus_pane(runtime: &mut Runtime, pane: &str) {
    runtime.dispatch_json(&explorer_event(
        "focus_pane",
        serde_json::json!({"pane_id": pane, "origin": "operator", "focus_device": false}),
    ));
}
