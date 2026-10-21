use super::*;
use crate::workspace_control::{Action, ActionPreparation, Context, Edge, Query};

fn action_id(suffix: &str) -> String {
    format!("{}-{suffix}", unix_milliseconds())
}

fn caller_fixture() -> (Runtime, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("fixture directory");
    let mut runtime = runtime();
    runtime.snapshot.status.herdr.state = "connected".to_owned();
    runtime.workspace_views = Some(
        WorkspaceViewStore::open(
            dir.path().join("views.json"),
            (
                runtime.snapshot.ui_state.right_panel_visible,
                runtime.snapshot.ui_state.right_panel_section,
            ),
        )
        .0,
    );
    runtime.snapshot.navigator.workspaces = vec![
        workspace(
            "workspace-a",
            "A",
            "/checkouts/a",
            vec![checkout(
                "workspace-a",
                "checkout-a",
                "/checkouts/a",
                Some(pane("pane-a", "/checkouts/a")),
            )],
        ),
        workspace(
            "workspace-b",
            "B",
            "/checkouts/b",
            vec![checkout(
                "workspace-b",
                "checkout-b",
                "/checkouts/b",
                Some(pane("pane-b", "/checkouts/b")),
            )],
        ),
    ];
    (runtime, dir)
}

fn seed_browser_areas(runtime: &mut Runtime, context: &Context) -> (String, String, String) {
    let layout = &mut runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(&context.device_id, &context.checkout_path)
        .layout;
    let first = layout.new_browser_display("https://example.test/page", 1);
    let first_id = first.id.clone();
    layout.insert("a1", first, 1).unwrap();
    let other = layout.new_browser_display("https://example.test/other", 2);
    let other_id = other.id.clone();
    layout.insert("a1", other, 2).unwrap();
    let other_area = layout.split(&other_id, "a1", Edge::Right, 3).unwrap();
    (first_id, other_id, other_area)
}

fn browser_control_state(
    runtime: &Runtime,
) -> (
    crate::workspace_views::WorkspaceViews,
    u64,
    u64,
    serde_json::Value,
) {
    let store = runtime.workspace_views.as_ref().unwrap();
    (
        store.views.clone(),
        store.generation,
        store.browser_load,
        serde_json::to_value((
            &runtime.snapshot.navigator,
            &runtime.snapshot.ui_state,
            &store.front,
            &store.live,
            &store.agent_live,
            &store.derived_active,
        ))
        .unwrap(),
    )
}

#[test]
fn caller_query_reads_only_its_live_checkout_and_untouched_views_are_empty() {
    let (mut runtime, _dir) = caller_fixture();
    let before = runtime.snapshot.navigator.focused_workspace_id.clone();
    let info = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap();
    assert_eq!(info.context.device_id, crate::node::TEST_NODE);
    assert_eq!(info.context.workspace_id, "workspace-b");
    assert_eq!(info.context.checkout_id, "checkout-b");
    assert_eq!(info.context.checkout_path, "/checkouts/b");
    assert!(info.views.is_none());
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
            .unwrap()
            .views,
        Some(Vec::new())
    );

    let layout = &mut runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, "/checkouts/b")
        .layout;
    let display = layout.new_display(
        "/checkouts/b/보고서.md",
        crate::view_layout::DisplayKind::File,
        None,
        false,
    );
    layout.insert("a1", display, 1).unwrap();
    let views = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
        .unwrap()
        .views
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].target, "/checkouts/b/보고서.md");
    assert_eq!(views[0].kind, "file");
    assert!(views[0].selected);
    assert!(views[0].active_area);
    assert_eq!(runtime.snapshot.navigator.focused_workspace_id, before);
}

#[test]
fn caller_query_refuses_stale_or_ambiguous_pane_without_using_the_front_workspace() {
    let (mut runtime, _dir) = caller_fixture();
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "closed-pane", Query::Info)
            .unwrap_err()
            .reason,
        "pane_not_connected"
    );
    runtime.snapshot.navigator.workspaces[1].checkouts[0].tabs[0].panes[0].id = "pane-a".to_owned();
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-a", Query::Info)
            .unwrap_err()
            .reason,
        "ambiguous_pane"
    );
    runtime.snapshot.navigator.workspaces[1].checkouts[0].tabs[0].panes[0].id = "pane-b".to_owned();
    runtime.workspace_views = None;
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap_err()
            .reason,
        "views_unavailable"
    );

    runtime.snapshot.status.herdr.state = "disconnected".to_owned();
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap_err()
            .reason,
        "pane_not_connected"
    );
}

#[test]
fn attested_device_resolves_colliding_pane_ids_without_crossing_workspaces() {
    let (mut runtime, _dir) = caller_fixture();
    let device = "remote:fixture";
    let mut remote_workspace = workspace(
        "remote-workspace",
        "Remote",
        "/remote/checkout",
        vec![checkout(
            "remote-workspace",
            "remote-checkout",
            "/remote/checkout",
            Some(pane("pane-b", "/remote/checkout")),
        )],
    );
    remote_workspace.device_id = device.to_owned();
    remote_workspace.remote_target_id = Some(device.to_owned());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: device.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_workspace],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });

    let local = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap();
    let remote = runtime
        .workspace_control_query(device, "pane-b", Query::Info)
        .unwrap();
    assert_eq!(local.context.workspace_id, "workspace-b");
    assert_eq!(remote.context.workspace_id, "remote-workspace");
    assert_eq!(remote.context.device_id, device);

    runtime.snapshot.status.remote[0].state = "disconnected".to_owned();
    assert_eq!(
        runtime
            .workspace_control_query(device, "pane-b", Query::Info)
            .unwrap_err()
            .reason,
        "pane_not_connected"
    );
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap()
            .context
            .workspace_id,
        "workspace-b"
    );
}

#[test]
fn background_view_commands_preserve_front_focus_and_retried_split_converges() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let other = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-a", Query::Info)
        .unwrap()
        .context;
    let key = (crate::node::TEST_NODE, "/checkouts/b");
    let layout = &mut runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(key.0, key.1)
        .layout;
    for path in ["first.md", "second.md"] {
        let display = layout.new_display(
            &format!("/checkouts/b/{path}"),
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        layout.insert("a1", display, 1).unwrap();
    }
    let before = (
        runtime.snapshot.navigator.focused_workspace_id.clone(),
        runtime.snapshot.navigator.focused_checkout_id.clone(),
        runtime.snapshot.ui_state.selected_pane_id.clone(),
    );
    let view_id = runtime
        .view_layout_of(&(key.0.into(), key.1.into()))
        .unwrap()
        .areas()[0]
        .displays[0]
        .id
        .clone();
    let action = Action::Split {
        view_id: view_id.clone(),
        area_id: "a1".into(),
        edge: Edge::Right,
    };
    let retry_id = action_id("retry-split");
    let first = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &retry_id,
            action.clone(),
            Ok(None),
        )
        .unwrap();
    assert!(first.changed);
    let retry = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &retry_id,
            action,
            Ok(None),
        )
        .unwrap();
    assert_eq!(retry, first);
    assert_eq!(
        runtime
            .view_layout_of(&(key.0.into(), key.1.into()))
            .unwrap()
            .area_count(),
        2
    );
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &retry_id,
                Action::Close {
                    view_id: view_id.clone(),
                    expected_browser_area: None,
                },
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "request_id_reused"
    );
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-a",
                &other,
                &action_id("wrong-workspace"),
                Action::Select {
                    view_id: view_id.clone(),
                    expected_browser_area: None,
                },
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "view_layout.unknown_display"
    );
    assert_eq!(
        (
            runtime.snapshot.navigator.focused_workspace_id.clone(),
            runtime.snapshot.navigator.focused_checkout_id.clone(),
            runtime.snapshot.ui_state.selected_pane_id.clone(),
        ),
        before
    );

    runtime.snapshot.navigator.workspaces[1].checkouts[0].tabs[0].panes[0].id =
        "former-pane-b".to_owned();
    runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0].panes[0].id = "pane-b".to_owned();
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &retry_id,
                Action::Split {
                    view_id,
                    area_id: "a1".into(),
                    edge: Edge::Right,
                },
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "pane_changed"
    );
}

#[test]
fn close_of_an_already_closed_view_returns_a_no_change_result() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let key = (crate::node::TEST_NODE, "/checkouts/b");
    let layout = &mut runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(key.0, key.1)
        .layout;
    let display = layout.new_browser_display("https://example.org", 1);
    let view_id = display.id.clone();
    layout.insert("a1", display, 1).unwrap();
    let action = Action::Close {
        view_id: view_id.clone(),
        expected_browser_area: None,
    };
    assert!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &action_id("close-1"),
                action.clone(),
                Ok(None),
            )
            .unwrap()
            .changed
    );
    assert!(
        !runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &action_id("close-2"),
                action,
                Ok(None)
            )
            .unwrap()
            .changed
    );
    assert!(
        runtime
            .view_layout_of(&(key.0.into(), key.1.into()))
            .unwrap()
            .displays()
            .next()
            .is_none()
    );
}

/// Puts the operator in `pane_id` of `checkout_id`, the Workspace in front.
fn operator_in(runtime: &mut Runtime, workspace_id: &str, checkout_id: &str, pane_id: &str) {
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.to_owned());
    runtime.select_terminal_pane(Some(pane_id.to_owned()));
    runtime.sync_workspace_view();
}

#[test]
fn a_select_calls_the_lane_only_from_the_pane_the_operator_is_in() {
    let (mut runtime, _dir) = caller_fixture();
    runtime.sync_agent_lineage_layouts();
    let key = (crate::node::TEST_NODE, "/checkouts/b");
    let layout = &mut runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(key.0, key.1)
        .layout;
    let first = layout.new_browser_display("https://example.org", 1);
    let first_id = first.id.clone();
    layout.insert("a1", first, 1).unwrap();
    let second = layout.new_browser_display("https://example.net", 2);
    let second_id = second.id.clone();
    layout.insert("a1", second, 2).unwrap();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let select = |view_id: &str| Action::Select {
        view_id: view_id.to_owned(),
        expected_browser_area: None,
    };
    let called = |runtime: &Runtime| runtime.workspace_views.as_ref().unwrap().lane_calls;
    let in_front = |runtime: &Runtime| {
        runtime
            .view_layout_of(&(key.0.to_owned(), key.1.to_owned()))
            .and_then(|layout| layout.active_area().active.clone())
    };
    operator_in(&mut runtime, "workspace-a", "checkout-a", "pane-a");
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("select-elsewhere"),
            select(&first_id),
            Ok(None),
        )
        .unwrap();
    assert_eq!(
        runtime.snapshot.navigator.focused_workspace_id.as_deref(),
        Some("workspace-a"),
        "the operator stays where they are"
    );
    assert_eq!(
        in_front(&runtime).as_deref(),
        Some(first_id.as_str()),
        "a Workspace not on screen shows what its agent selected, which it can then drive"
    );
    assert!(
        !views_of(&runtime, key),
        "right-lane B17: a select from another pane leaves the caller's lane as it was"
    );
    assert_eq!(
        called(&runtime),
        0,
        "a select from another pane calls no lane"
    );

    operator_in(&mut runtime, "workspace-b", "checkout-b", "pane-b");
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("select-operator"),
            select(&second_id),
            Ok(None),
        )
        .unwrap();
    assert_eq!(in_front(&runtime).as_deref(), Some(second_id.as_str()));
    assert!(
        views_of(&runtime, key),
        "a select from the operator's pane opens the files lane"
    );
    // The shell shows a called lane over the terminals; a call from the CLI
    // reaches it only as this count rising (PRD right-lane B1, B16).
    assert_eq!(called(&runtime), 1, "the operator's pane calls the lane");
    let numbered = runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .lane_called
        .get(&(key.0.to_owned(), key.1.to_owned()))
        .copied();
    assert_eq!(
        numbered,
        Some(1),
        "the call is numbered for the Workspace it called"
    );
}

fn views_of(runtime: &Runtime, key: (&str, &str)) -> bool {
    runtime
        .workspace_views
        .as_ref()
        .unwrap()
        .views
        .workspaces
        .iter()
        .find(|view| view.is(key.0, key.1))
        .map(|view| view.tool == Some(crate::workspace_views::Tool::Files))
        .unwrap()
}

#[test]
fn file_open_reads_outside_the_runtime_then_places_once_in_the_callers_workspace() {
    let (mut runtime, dir) = caller_fixture();
    let root = dir.path().to_string_lossy().into_owned();
    runtime.snapshot.navigator.workspaces[1].path = root.clone();
    runtime.snapshot.navigator.workspaces[1].checkouts[0].path = root.clone();
    runtime.sync_agent_lineage_layouts();
    let path = dir.path().join("보고서.md");
    std::fs::write(&path, "hello\n").unwrap();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let before = runtime.snapshot.navigator.focused_workspace_id.clone();
    let action = Action::OpenFile {
        path: path.to_string_lossy().into_owned(),
        beside: false,
    };
    let id = action_id("file-open");
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("file needs a host read")
    };
    let material = source.read().unwrap();
    let first = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &id,
            action.clone(),
            Ok(Some(material)),
        )
        .unwrap();
    assert!(first.changed);
    assert_eq!(first.context.checkout_path, root);
    assert_eq!(runtime.snapshot.navigator.focused_workspace_id, before);
    let views = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
        .unwrap()
        .views
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].view_id, first.view_id);
    assert_eq!(views[0].target, path.to_string_lossy());
    std::fs::remove_file(&path).unwrap();
    let ActionPreparation::Cached(Ok(retried)) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("retry should return its recorded result without rereading")
    };
    assert_eq!(retried, first);
}

#[test]
fn browser_load_status_tracks_the_current_load_and_retry_does_not_reload() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let before = runtime.snapshot.navigator.focused_workspace_id.clone();
    let action = Action::OpenBrowser {
        url: "https://example.test/page".into(),
        area_id: None,
        new_target: false,
    };
    let request_id = action_id("browser-open");
    let first = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &request_id,
            action.clone(),
            Ok(None),
        )
        .unwrap();
    assert_eq!(runtime.snapshot.navigator.focused_workspace_id, before);
    let key = (
        crate::node::TEST_NODE.to_owned(),
        expected.checkout_path.clone(),
    );
    let load = runtime
        .view_layout_of(&key)
        .unwrap()
        .display(&first.view_id)
        .unwrap()
        .load;
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &request_id,
                action.clone(),
                Ok(None)
            )
            .unwrap(),
        first
    );
    assert_eq!(
        runtime
            .view_layout_of(&key)
            .unwrap()
            .display(&first.view_id)
            .unwrap()
            .load,
        load
    );
    let page = |runtime: &Runtime| {
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
            .unwrap()
            .views
            .unwrap()
            .into_iter()
            .find(|view| view.view_id == first.view_id)
            .unwrap()
            .page
            .unwrap()
    };
    assert_eq!(page(&runtime).state, "pending");
    let report = |runtime: &mut Runtime, load: u64, loading: bool, failure: Option<&str>| {
        let payload = serde_json::from_value(serde_json::json!({
            "workspace":{"device_id":crate::node::TEST_NODE,"path":expected.checkout_path},
            "display_id":first.view_id,"url":"https://example.test/page","title":"Page",
            "load":load,"loading":loading,"failure":failure,
        }))
        .unwrap();
        runtime.record_browser_state(payload);
    };
    report(&mut runtime, load, true, None);
    assert_eq!(page(&runtime).state, "loading");
    report(&mut runtime, load, false, None);
    assert_eq!(page(&runtime).state, "loaded");
    let gone = serde_json::from_value(serde_json::json!({
        "workspace":{"device_id":crate::node::TEST_NODE,"path":expected.checkout_path},
        "display_id":first.view_id,"url":"https://example.test/page","title":"Page",
        "load":load,"present":false,
    }))
    .unwrap();
    runtime.record_browser_state(gone);
    assert_eq!(page(&runtime).state, "disconnected");
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("browser-reload"),
            action,
            Ok(None),
        )
        .unwrap();
    let newer = runtime
        .view_layout_of(&key)
        .unwrap()
        .display(&first.view_id)
        .unwrap()
        .load;
    assert!(newer > load);
    assert_eq!(page(&runtime).state, "pending");
    report(&mut runtime, load, false, None);
    assert_eq!(page(&runtime).state, "pending");
    report(&mut runtime, newer, false, Some("Connection refused"));
    assert_eq!(page(&runtime).state, "failed");
    assert_eq!(
        page(&runtime).failure.as_deref(),
        Some("Connection refused")
    );
}

#[test]
fn browser_new_target_uses_the_named_area_and_duplicate_intent_converges() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let (existing, other_view, other_area) = seed_browser_areas(&mut runtime, &expected);
    let key = (expected.device_id.clone(), expected.checkout_path.clone());
    assert_eq!(
        runtime.view_layout_of(&key).unwrap().active_area().id,
        other_area
    );
    let before =
        serde_json::to_value((&runtime.snapshot.navigator, &runtime.snapshot.ui_state)).unwrap();
    let action = Action::OpenBrowser {
        url: "https://example.test/page".into(),
        area_id: Some("a1".into()),
        new_target: true,
    };
    let id = action_id("new-browser-target");
    let first = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &id,
            action.clone(),
            Ok(None),
        )
        .unwrap();
    assert_ne!(first.view_id, existing);
    assert_eq!(first.area_id.as_deref(), Some("a1"));
    let layout = runtime.view_layout_of(&key).unwrap();
    assert_eq!(layout.area("a1").unwrap().displays.len(), 2);
    assert_eq!(
        layout.area(&other_area).unwrap().active.as_deref(),
        Some(other_view.as_str())
    );
    assert_eq!(layout.display(&existing).unwrap().load, 1);
    assert_eq!(
        serde_json::to_value((&runtime.snapshot.navigator, &runtime.snapshot.ui_state)).unwrap(),
        before
    );

    // A lost reply repeats the intent, even if the caller supplies no material.
    let after = browser_control_state(&runtime);
    let ActionPreparation::Cached(Ok(cached)) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("duplicate target creation must return its receipt")
    };
    assert_eq!(cached, first);
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &id,
                action.clone(),
                Ok(None)
            )
            .unwrap(),
        first
    );
    assert_eq!(browser_control_state(&runtime), after);

    let second = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("another-browser-target"),
            action,
            Ok(None),
        )
        .unwrap();
    assert_ne!(second.view_id, first.view_id);
    assert_eq!(second.area_id.as_deref(), Some("a1"));
    assert_eq!(
        runtime
            .view_layout_of(&key)
            .unwrap()
            .area("a1")
            .unwrap()
            .displays
            .len(),
        3
    );
}

#[test]
fn browser_explicit_area_dedupes_only_within_that_area() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let (_, other_view, other_area) = seed_browser_areas(&mut runtime, &expected);
    let key = (expected.device_id.clone(), expected.checkout_path.clone());
    let action = Action::OpenBrowser {
        url: "https://example.test/other".into(),
        area_id: Some("a1".into()),
        new_target: false,
    };
    let first = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("area-dedupe"),
            action.clone(),
            Ok(None),
        )
        .unwrap();
    assert_ne!(first.view_id, other_view);
    assert_eq!(first.area_id.as_deref(), Some("a1"));
    let reloaded = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("area-reload"),
            action,
            Ok(None),
        )
        .unwrap();
    assert_eq!(reloaded.view_id, first.view_id);
    assert_eq!(reloaded.area_id.as_deref(), Some("a1"));
    assert!(reloaded.load.unwrap() > first.load.unwrap());
    let layout = runtime.view_layout_of(&key).unwrap();
    assert_eq!(layout.displays().count(), 3);
    assert_eq!(layout.display(&other_view).unwrap().load, 2);
    assert_eq!(
        layout.area(&other_area).unwrap().active.as_deref(),
        Some(other_view.as_str())
    );
}

#[test]
fn legacy_browser_open_keeps_global_dedupe_and_active_area_insertion() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let (existing, _, other_area) = seed_browser_areas(&mut runtime, &expected);
    let key = (expected.device_id.clone(), expected.checkout_path.clone());
    let fresh: Action = serde_json::from_value(serde_json::json!({
        "action": "open_browser", "url": "https://example.test/fresh",
    }))
    .unwrap();
    let inserted = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("legacy-fresh"),
            fresh,
            Ok(None),
        )
        .unwrap();
    assert_eq!(inserted.area_id.as_deref(), Some(other_area.as_str()));
    let duplicate: Action = serde_json::from_value(serde_json::json!({
        "action": "open_browser", "url": "https://example.test/page",
    }))
    .unwrap();
    let reused = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("legacy-duplicate"),
            duplicate,
            Ok(None),
        )
        .unwrap();
    assert_eq!(reused.view_id, existing);
    assert_eq!(reused.area_id.as_deref(), Some("a1"));
    assert_eq!(runtime.view_layout_of(&key).unwrap().displays().count(), 3);
}

#[test]
fn browser_unknown_area_refuses_without_mutating_empty_or_existing_views() {
    for seeded in [false, true] {
        let (mut runtime, _dir) = caller_fixture();
        let expected = runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap()
            .context;
        if seeded {
            seed_browser_areas(&mut runtime, &expected);
        }
        let action = Action::OpenBrowser {
            url: "https://example.test/page".into(),
            area_id: Some("missing-area".into()),
            new_target: true,
        };
        let id = action_id("unknown-browser-area");
        let before = browser_control_state(&runtime);
        let refusal = runtime
            .workspace_control_prepare_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &id,
                &action,
            )
            .err()
            .unwrap();
        assert_eq!(refusal.reason, "view_layout.unknown_area");
        assert_eq!(browser_control_state(&runtime), before);
        assert_eq!(
            runtime
                .workspace_control_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &id,
                    action.clone(),
                    Ok(None)
                )
                .unwrap_err(),
            refusal
        );
        assert_eq!(browser_control_state(&runtime), before);
        let ActionPreparation::Cached(Err(cached)) = runtime
            .workspace_control_prepare_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &id,
                &action,
            )
            .unwrap()
        else {
            panic!("a committed refusal is idempotent too")
        };
        assert_eq!(cached, refusal);
    }
}

#[test]
fn browser_area_removed_during_file_read_refuses_without_mutation() {
    let (mut runtime, dir) = caller_fixture();
    let root = dir.path().to_string_lossy().into_owned();
    runtime.snapshot.navigator.workspaces[1].path = root.clone();
    runtime.snapshot.navigator.workspaces[1].checkouts[0].path = root;
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let (_, other_view, other_area) = seed_browser_areas(&mut runtime, &expected);
    let path = dir.path().join("index.html");
    std::fs::write(&path, "<title>Inside</title>").unwrap();
    let action = Action::OpenBrowser {
        url: format!("file://{}", path.display()),
        area_id: Some(other_area.clone()),
        new_target: true,
    };
    let id = action_id("browser-area-disappeared");
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("the checked HTML source must still be read outside the lock")
    };
    let material = source.read().unwrap();
    let key = (expected.device_id.clone(), expected.checkout_path.clone());
    runtime
        .change_view_layout(&key, |layout, _| Ok((layout.remove(&other_view), true)))
        .unwrap();
    assert!(
        runtime
            .view_layout_of(&key)
            .unwrap()
            .area(&other_area)
            .is_none()
    );
    let before = browser_control_state(&runtime);
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &id,
                action,
                Ok(Some(material))
            )
            .unwrap_err()
            .reason,
        "view_layout.unknown_area"
    );
    assert_eq!(browser_control_state(&runtime), before);
}

#[test]
fn scoped_browser_close_and_select_refuse_a_page_moved_after_prepare() {
    for close in [false, true] {
        let (mut runtime, _dir) = caller_fixture();
        let expected = runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap()
            .context;
        let (view_id, _, other_area) = seed_browser_areas(&mut runtime, &expected);
        let copied_view = runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
            .unwrap()
            .views
            .unwrap()
            .into_iter()
            .find(|view| view.view_id == view_id)
            .unwrap();
        assert_eq!(copied_view.kind, "browser");
        assert_eq!(copied_view.area_id, "a1");
        let action = if close {
            Action::Close {
                view_id: view_id.clone(),
                expected_browser_area: Some(copied_view.area_id),
            }
        } else {
            Action::Select {
                view_id: view_id.clone(),
                expected_browser_area: Some(copied_view.area_id),
            }
        };
        let id = action_id("scoped-before-move");
        assert!(matches!(
            runtime
                .workspace_control_prepare_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &id,
                    &action
                )
                .unwrap(),
            ActionPreparation::Ready
        ));

        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &action_id("move-page-before-commit"),
                Action::Move {
                    view_id: view_id.clone(),
                    area_id: other_area.clone(),
                    index: 0,
                },
                Ok(None),
            )
            .unwrap();
        let key = (expected.device_id.clone(), expected.checkout_path.clone());
        assert_eq!(
            runtime
                .view_layout_of(&key)
                .unwrap()
                .area_of(&view_id)
                .unwrap()
                .id,
            other_area
        );
        let before = browser_control_state(&runtime);
        assert_eq!(
            runtime
                .workspace_control_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &id,
                    action,
                    Ok(None)
                )
                .unwrap_err()
                .reason,
            "browser_scope_changed"
        );
        assert_eq!(browser_control_state(&runtime), before);
    }
}

#[test]
fn scoped_browser_close_and_select_refuse_a_document_in_the_named_area() {
    for close in [false, true] {
        let (mut runtime, _dir) = caller_fixture();
        let expected = runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap()
            .context;
        let layout = &mut runtime
            .workspace_views
            .as_mut()
            .unwrap()
            .views
            .entry(&expected.device_id, &expected.checkout_path)
            .layout;
        let file = layout.new_display(
            "/checkouts/b/index.html",
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        let view_id = file.id.clone();
        layout.insert("a1", file, 1).unwrap();
        let action = if close {
            Action::Close {
                view_id,
                expected_browser_area: Some("a1".into()),
            }
        } else {
            Action::Select {
                view_id,
                expected_browser_area: Some("a1".into()),
            }
        };
        let id = action_id("scoped-document");
        let before = browser_control_state(&runtime);
        assert_eq!(
            runtime
                .workspace_control_prepare_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &id,
                    &action
                )
                .err()
                .unwrap()
                .reason,
            "browser_scope_changed"
        );
        assert_eq!(
            runtime
                .workspace_control_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &id,
                    action,
                    Ok(None)
                )
                .unwrap_err()
                .reason,
            "browser_scope_changed"
        );
        assert_eq!(browser_control_state(&runtime), before);
    }
}

#[test]
fn browser_scope_is_part_of_retry_identity_and_closed_target_replays_its_receipt() {
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let (view_id, _, other_area) = seed_browser_areas(&mut runtime, &expected);
    let selected_action = Action::Select {
        view_id: view_id.clone(),
        expected_browser_area: Some("a1".into()),
    };
    let select_id = action_id("scoped-select");
    let selected = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &select_id,
            selected_action.clone(),
            Ok(None),
        )
        .unwrap();
    assert_eq!(selected.view_id, view_id);
    assert_eq!(selected.area_id.as_deref(), Some("a1"));
    let after_select = browser_control_state(&runtime);
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &select_id,
                selected_action,
                Ok(None)
            )
            .unwrap(),
        selected
    );
    for expected_browser_area in [None, Some(other_area)] {
        assert_eq!(
            runtime
                .workspace_control_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &select_id,
                    Action::Select {
                        view_id: view_id.clone(),
                        expected_browser_area
                    },
                    Ok(None),
                )
                .unwrap_err()
                .reason,
            "request_id_reused"
        );
    }
    assert_eq!(browser_control_state(&runtime), after_select);

    let close_action = Action::Close {
        view_id: view_id.clone(),
        expected_browser_area: Some("a1".into()),
    };
    let close_id = action_id("scoped-close");
    let closed = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &close_id,
            close_action.clone(),
            Ok(None),
        )
        .unwrap();
    assert!(closed.changed);
    let key = (expected.device_id.clone(), expected.checkout_path.clone());
    assert!(
        runtime
            .view_layout_of(&key)
            .unwrap()
            .display(&view_id)
            .is_none()
    );
    let after_close = browser_control_state(&runtime);
    let ActionPreparation::Cached(Ok(cached)) = runtime
        .workspace_control_prepare_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &close_id,
            &close_action,
        )
        .unwrap()
    else {
        panic!("a closed browser target must replay its successful receipt")
    };
    assert_eq!(cached, closed);
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &close_id,
                close_action.clone(),
                Ok(None)
            )
            .unwrap(),
        closed
    );
    assert_eq!(browser_control_state(&runtime), after_close);
    for expected_browser_area in [None, Some("different-area".into())] {
        assert_eq!(
            runtime
                .workspace_control_action(
                    crate::node::TEST_NODE,
                    "pane-b",
                    &expected,
                    &close_id,
                    Action::Close {
                        view_id: view_id.clone(),
                        expected_browser_area
                    },
                    Ok(None),
                )
                .unwrap_err()
                .reason,
            "request_id_reused"
        );
    }
    assert_eq!(browser_control_state(&runtime), after_close);
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &action_id("fresh-scoped-close"),
                close_action,
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "browser_scope_changed"
    );
    assert!(
        !runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &action_id("legacy-close-missing"),
                Action::Close {
                    view_id,
                    expected_browser_area: None
                },
                Ok(None),
            )
            .unwrap()
            .changed
    );
}

#[test]
fn browser_file_source_is_confined_to_the_calling_checkout() {
    let (mut runtime, dir) = caller_fixture();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    runtime.snapshot.navigator.workspaces[1].path = checkout.to_string_lossy().into_owned();
    runtime.snapshot.navigator.workspaces[1].checkouts[0].path =
        checkout.to_string_lossy().into_owned();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let outside = dir.path().join("outside.html");
    std::fs::write(&outside, "<title>Outside</title>").unwrap();
    let action = Action::OpenBrowser {
        url: format!("file://{}", outside.display()),
        area_id: None,
        new_target: false,
    };
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &action_id("outside-html"),
            &action,
        )
        .unwrap()
    else {
        panic!("file URL must read outside the lock")
    };
    assert_eq!(source.read().err().unwrap().reason, "path_outside_checkout");
    let inside = checkout.join("index.html");
    std::fs::write(&inside, "<title>Inside</title>").unwrap();
    let action = Action::OpenBrowser {
        url: format!("file://{}", inside.display()),
        area_id: None,
        new_target: false,
    };
    let id = action_id("inside-html");
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("file URL must read outside the lock")
    };
    let material = source.read().unwrap();
    assert!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &id,
                action,
                Ok(Some(material))
            )
            .is_ok()
    );
}

#[test]
fn diff_open_requires_a_real_working_tree_change() {
    let (mut runtime, dir) = caller_fixture();
    let root = dir.path().to_string_lossy().into_owned();
    runtime.snapshot.navigator.workspaces[1].path = root.clone();
    runtime.snapshot.navigator.workspaces[1].checkouts[0].path = root;
    let path = dir.path().join("memo.md");
    std::fs::write(&path, "baseline\n").unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["add", "memo.md"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "baseline",
    ]);
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let action = Action::OpenDiff {
        path: path.to_string_lossy().into_owned(),
        beside: false,
    };
    let clean_id = action_id("clean");
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &clean_id,
            &action,
        )
        .unwrap()
    else {
        panic!("diff needs a Git read")
    };
    let Err(refusal) = source.read() else {
        panic!("unchanged file must have no diff")
    };
    assert_eq!(refusal.reason, "diff_unchanged");
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                "pane-b",
                &expected,
                &clean_id,
                action.clone(),
                Err(refusal),
            )
            .unwrap_err()
            .reason,
        "diff_unchanged"
    );
    std::fs::write(&path, "changed\n").unwrap();
    let ActionPreparation::Cached(Err(retried)) = runtime
        .workspace_control_prepare_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &clean_id,
            &action,
        )
        .unwrap()
    else {
        panic!("the failed intent must keep its refusal")
    };
    assert_eq!(retried.reason, "diff_unchanged");
    let id = action_id("changed");
    let ActionPreparation::Read(source) = runtime
        .workspace_control_prepare_action(crate::node::TEST_NODE, "pane-b", &expected, &id, &action)
        .unwrap()
    else {
        panic!("diff needs a Git read")
    };
    let material = source.read().unwrap();
    let result = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            "pane-b",
            &expected,
            &id,
            action,
            Ok(Some(material)),
        )
        .unwrap();
    let views = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::ViewList)
        .unwrap()
        .views
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].view_id, result.view_id);
    assert_eq!(views[0].kind, "diff");
}

#[test]
fn checkout_caller_resolves_the_longest_registered_checkout_containing_its_cwd() {
    let (mut runtime, _dir) = caller_fixture();
    runtime.snapshot.navigator.workspaces[0]
        .checkouts
        .push(checkout(
            "workspace-a",
            "checkout-a-wt",
            "/checkouts/a/worktrees/wt",
            None,
        ));
    let caller = |key: &str, cwd: &str| crate::workspace_control::checkout_caller_id(key, cwd);

    let nested = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            &caller("k1", "/checkouts/a/worktrees/wt/src"),
            Query::Info,
        )
        .unwrap();
    assert_eq!(nested.context.checkout_id, "checkout-a-wt");
    assert_eq!(nested.context.checkout_path, "/checkouts/a/worktrees/wt");
    assert_eq!(nested.context.workspace_id, "workspace-a");
    assert!(nested.views.is_none());

    let parent = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            &caller("k1", "/checkouts/a/src"),
            Query::ViewList,
        )
        .unwrap();
    assert_eq!(parent.context.checkout_id, "checkout-a");
    assert_eq!(parent.views, Some(Vec::new()));

    let exact = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            &caller("k1", "/checkouts/b"),
            Query::Info,
        )
        .unwrap();
    assert_eq!(exact.context.checkout_id, "checkout-b");
    // A checkout caller is offered the Workspace, never delivery, which acts
    // as one agent and so needs a pane-bound caller.
    let offered = |caller: &str| {
        runtime
            .workspace_control_query(crate::node::TEST_NODE, caller, Query::Info)
            .unwrap()
            .capabilities
    };
    let checkout_offer = offered(&caller("k1", "/checkouts/b"));
    assert!(checkout_offer.contains(&"view.list"));
    for delivery in ["request.send", "inbox", "inbox.hook", "watch.assign"] {
        assert!(!checkout_offer.contains(&delivery), "{delivery}");
        assert!(offered("pane-b").contains(&delivery), "{delivery}");
    }

    for outside in ["/checkouts/ab", "/checkouts", "/elsewhere/checkouts/a"] {
        assert_eq!(
            runtime
                .workspace_control_query(
                    crate::node::TEST_NODE,
                    &caller("k1", outside),
                    Query::Info
                )
                .unwrap_err()
                .reason,
            "checkout_not_registered",
            "{outside}"
        );
    }

    runtime.snapshot.status.herdr.state = "disconnected".to_owned();
    assert_eq!(
        runtime
            .workspace_control_query(
                crate::node::TEST_NODE,
                &caller("k1", "/checkouts/a"),
                Query::Info
            )
            .unwrap_err()
            .reason,
        "checkout_not_registered"
    );
}

#[test]
fn two_registrations_of_one_path_refuse_the_checkout_caller_instead_of_choosing() {
    let (mut runtime, _dir) = caller_fixture();
    runtime.snapshot.navigator.workspaces[0]
        .checkouts
        .push(checkout(
            "workspace-a",
            "checkout-b-again",
            "/checkouts/b",
            None,
        ));
    assert_eq!(
        runtime
            .workspace_control_query(
                crate::node::TEST_NODE,
                &crate::workspace_control::checkout_caller_id("k1", "/checkouts/b/src"),
                Query::Info,
            )
            .unwrap_err()
            .reason,
        "ambiguous_checkout"
    );
    // The pane in that checkout is unaffected.
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
            .unwrap()
            .context
            .checkout_id,
        "checkout-b"
    );
}

#[test]
fn checkout_callers_in_one_checkout_keep_separate_retry_records() {
    let (mut runtime, _dir) = caller_fixture();
    let first = crate::workspace_control::checkout_caller_id("k1", "/checkouts/b/src");
    let second = crate::workspace_control::checkout_caller_id("k2", "/checkouts/b");
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, &first, Query::Info)
        .unwrap()
        .context;
    assert_eq!(
        runtime
            .workspace_control_query(crate::node::TEST_NODE, &second, Query::Info)
            .unwrap()
            .context,
        expected
    );
    let request_id = action_id("shared");
    let opened = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            &first,
            &expected,
            &request_id,
            Action::OpenBrowser {
                url: "http://localhost:3000".into(),
                area_id: None,
                new_target: false,
            },
            Ok(None),
        )
        .unwrap();
    assert!(opened.changed);
    assert_eq!(opened.context, expected);
    // The second caller reusing the same request id for another action is not
    // told the id was reused: the record belongs to the first capability.
    let selected = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            &second,
            &expected,
            &request_id,
            Action::Select {
                view_id: opened.view_id.clone(),
                expected_browser_area: None,
            },
            Ok(None),
        )
        .unwrap();
    assert_eq!(selected.view_id, opened.view_id);
    let views = runtime
        .workspace_control_query(crate::node::TEST_NODE, &second, Query::ViewList)
        .unwrap()
        .views
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].target, "http://localhost:3000");

    // Unregistering the checkout refuses the checkout caller by name, not as a
    // moved pane.
    runtime.snapshot.navigator.workspaces[1].checkouts[0].path = "/checkouts/moved".to_owned();
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                &second,
                &expected,
                &action_id("after-move"),
                Action::Close {
                    view_id: opened.view_id,
                    expected_browser_area: None,
                },
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "checkout_not_registered"
    );
}

#[test]
fn hide_memory_refuses_a_disconnected_or_unknown_caller_and_a_changed_checkout() {
    // Memory requires a ready reader and the caller's attested checkout.
    let (mut runtime, _dir) = caller_fixture();
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: "studio".into(),
            label: "studio".into(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: "studio-host".into(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    let command = || crate::memory_command::MemoryCommand::Show { number: 1 };
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;

    let prepared = runtime
        .prepare_memory(crate::node::TEST_NODE, "pane-b", &expected, command())
        .unwrap();
    assert_eq!(prepared.checkout_path, "/checkouts/b");

    let refused = |device: &str, pane: &str| {
        runtime
            .prepare_memory(device, pane, &expected, command())
            .err()
    };
    assert_eq!(refused("studio", "pane-b").as_deref(), Some("link_down"));
    assert_eq!(
        refused("remote:fixture", "pane-b").as_deref(),
        Some("unregistered")
    );
    assert_eq!(
        refused(crate::node::TEST_NODE, "pane-a").as_deref(),
        Some("caller_context_changed")
    );
}

#[test]
fn hide_memory_reads_a_person_only_in_a_pane_no_agent_runs_in() {
    // PRD memory-stale-cards D-10 as amended: a release gives a card's report
    // back only for a person, so neither Herdr nor the ledger may name an
    // agent in the caller's pane, and a caller outside every pane is no one
    // Hide can name.
    use crate::memory_command::{MemoryCaller, MemoryCommand};
    let (mut runtime, _dir) = caller_fixture();
    let expected = runtime
        .workspace_control_query(crate::node::TEST_NODE, "pane-b", Query::Info)
        .unwrap()
        .context;
    let caller = |runtime: &Runtime, id: &str| {
        runtime
            .prepare_memory(
                crate::node::TEST_NODE,
                id,
                &expected,
                MemoryCommand::Release { number: 1 },
            )
            .unwrap()
            .caller
    };
    assert_eq!(caller(&runtime, "pane-b"), MemoryCaller::Person);
    assert_eq!(
        caller(
            &runtime,
            &crate::workspace_control::checkout_caller_id("k1", "/checkouts/b")
        ),
        MemoryCaller::Checkout,
        "a terminal outside every pane may be Codex's shared daemon"
    );

    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger.agents.push(crate::coordination::AgentRecord {
        id: "agent-1".into(),
        name: None,
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: "native".into(),
        instance: "pane-b".into(),
        pane: "pane-b".into(),
        parent: None,
        origin: None,
        project: None,
        actor: crate::delivery::Actor {
            pane_id: "pane-b".into(),
            name: "agent-1".into(),
            kind: "claude".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: None,
        },
        ended: false,
        terminal_id: None,
        forked: false,
    });
    runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
    assert_eq!(
        caller(&runtime, "pane-b"),
        MemoryCaller::Agent(None),
        "an agent the ledger names before Herdr lists it"
    );
    ledger.agents[0].ended = true;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    assert_eq!(caller(&runtime, "pane-b"), MemoryCaller::Person);

    let mut row = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [{"id": "a", "pane_id": "pane-b", "agent": "claude", "agent_status": "idle",
                "state_change_seq": 1, "tokens": {"task": "Work"}}],
        }))
        .unwrap(),
    )
    .agents
    .remove(0);
    runtime.snapshot.navigator.agents = vec![row.clone()];
    assert_eq!(
        caller(&runtime, "pane-b"),
        MemoryCaller::Agent(None),
        "an agent whose session Herdr has not said"
    );
    row.session_id = Some("s1".to_owned());
    runtime.snapshot.navigator.agents = vec![row];
    assert_eq!(
        caller(&runtime, "pane-b"),
        MemoryCaller::Agent(Some(("claude".to_owned(), "s1".to_owned())))
    );
}
