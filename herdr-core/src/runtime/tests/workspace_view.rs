use super::*;
use crate::workspace_views::Tool;

// PRD S6: a shell with separate Agent and View areas keeps each Workspace's
// document beside its terminals, its layout and tools, and brings them back
// after a restart; a shell without View areas keeps its
// document-in-place-of-terminal rule (Risks: older wire).

/// A new folder for a views file, and that file's path in it; whoever keeps
/// the folder keeps the file.
pub(super) fn views_path(name: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = scratch_dir(&format!("hide-workspace-views-{name}-"));
    let path = folder.path().join("workspace-views.json");
    (folder, path)
}

/// `with_views` on a new views file whose folder the runtime keeps.
pub(super) fn with_new_views(runtime: Runtime, name: &str) -> Runtime {
    let (folder, path) = views_path(name);
    let mut runtime = with_views(runtime, &path);
    runtime.test_dirs.push(folder);
    runtime
}

/// A daemon runtime whose local checkout roots are open.
pub(super) fn with_views(mut runtime: Runtime, path: &Path) -> Runtime {
    let roots = runtime
        .catalog_workspaces()
        .filter(|workspace| workspace.device_id == crate::node::TEST_NODE)
        .flat_map(|workspace| workspace.checkouts.iter())
        .filter_map(|checkout| {
            let path = PathBuf::from(&checkout.path);
            std::fs::File::open(&path).ok().map(|file| (path, file))
        })
        .collect();
    runtime.set_file_roots(crate::files::FileRoots::from_identities(
        hide_node::hold_roots(roots).1,
    ));
    with_views_only(runtime, path)
}

/// The daemon's own order: the views file is open and the first snapshot is
/// read before any root is pinned.
pub(super) fn with_views_only(mut runtime: Runtime, path: &Path) -> Runtime {
    let panel = (
        runtime.snapshot.ui_state.right_panel_visible,
        runtime.snapshot.ui_state.right_panel_section,
    );
    runtime.workspace_views = Some(WorkspaceViewStore::open(path.to_path_buf(), panel).0);
    runtime.sync_workspace_view();
    runtime
}

fn open(runtime: &mut Runtime, checkout_id: &str, path: &Path) {
    assert!(runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": path.to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "preview": false
        }),
    )));
}

pub(super) fn layout(runtime: &mut Runtime, payload: serde_json::Value) {
    runtime.dispatch_json(&explorer_event("workspace_view", payload));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// The front Workspace's lane: open, documents, tool and pin (PRD
/// lane-two-columns B1).
fn lane(runtime: &Runtime) -> (bool, bool, Option<Tool>, bool) {
    let view = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("a front Workspace");
    (view.open, view.documents, view.tool, view.pinned)
}

const CLOSED: (bool, bool, Option<Tool>, bool) = (false, false, None, false);
/// The documents alone, in a lane an open called.
const DOCUMENTS: (bool, bool, Option<Tool>, bool) = (true, true, None, false);

/// The front Workspace's last lane call number.
fn called(runtime: &Runtime) -> u64 {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .lane_called
}

fn only_display(runtime: &Runtime) -> (serde_json::Value, String) {
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    let workspace = serde_json::json!({"device_id": view.device_id, "path": view.path});
    let display = match &view.layout.root {
        crate::model::ViewNodeSnapshot::Area(area) => area.active.clone(),
        _ => None,
    }
    .expect("one display");
    (workspace, display)
}

fn close_only_display(runtime: &mut Runtime) {
    let (workspace, display) = only_display(runtime);
    runtime.dispatch_json(&explorer_event(
        "view_layout",
        serde_json::json!({"workspace": workspace, "action": "close", "display_id": display}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

pub(super) fn active_label(runtime: &Runtime) -> Option<String> {
    let active = runtime.snapshot.editor.active_tab_id.as_deref()?;
    runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.id == active)
        .map(|tab| tab.label.clone())
}

#[test]
fn browser_inventory_revokes_removed_and_disconnected_catalog_checkouts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_string_lossy().into_owned();
    let mut runtime = runtime();
    runtime.snapshot.status.herdr.state = "connected".into();
    let project = workspace(
        "inventory-project",
        "Inventory",
        &path,
        vec![checkout(
            "inventory-project",
            "inventory-checkout",
            &path,
            None,
        )],
    );
    runtime.snapshot.navigator.workspaces = vec![project.clone()];
    runtime = with_views_only(runtime, &directory.path().join("views.json"));
    let store = runtime.workspace_views.as_mut().unwrap();
    let layout = &mut store.views.entry(crate::node::TEST_NODE, &path).layout;
    let browser = layout.new_browser_display("https://example.test/inventory", 1);
    let id = browser.id.clone();
    layout.insert("a1", browser, 1).unwrap();
    store.generation += 1;
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.browser_views.len(), 1);
    assert_eq!(runtime.snapshot.browser_views[0].view_id, id);
    assert_eq!(runtime.snapshot.browser_views[0].area_id, "a1");
    assert_eq!(runtime.snapshot.browser_scopes.len(), 1);
    assert_eq!(
        runtime.snapshot.browser_scopes[0].device_id,
        crate::node::TEST_NODE
    );
    assert_eq!(runtime.snapshot.browser_scopes[0].path, path);
    assert_eq!(runtime.snapshot.browser_scopes[0].area_id, "a1");
    let original_incarnation = runtime.snapshot.browser_scopes[0].incarnation;
    let published = runtime.snapshot.browser_views_revision;
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.snapshot.browser_views_revision, published,
        "unchanged scope does not publish again"
    );
    assert_eq!(
        runtime.snapshot.browser_scopes[0].incarnation,
        original_incarnation
    );

    runtime.snapshot.navigator.workspaces.clear();
    runtime.sync_workspace_view();
    assert!(
        runtime.snapshot.browser_views.is_empty(),
        "removed catalog cannot retain a page"
    );
    assert_ne!(runtime.snapshot.browser_views_revision, published);
    assert!(runtime.snapshot.browser_scopes.is_empty());
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(crate::node::TEST_NODE, &path)
            .unwrap()
            .layout
            .display(&id)
            .is_some(),
        "saved layout remains restorable"
    );

    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.status.herdr.state = "disconnected".into();
    runtime.sync_workspace_view();
    assert!(
        runtime.snapshot.browser_views.is_empty(),
        "offline saved layout has no page authority"
    );
    assert!(runtime.snapshot.browser_scopes.is_empty());
    runtime.snapshot.status.herdr.state = "connected".into();
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.snapshot.browser_views.len(),
        1,
        "reconnected checkout gets fresh inventory"
    );
    assert_eq!(runtime.snapshot.browser_scopes.len(), 1);
    let regranted_incarnation = runtime.snapshot.browser_scopes[0].incarnation;
    assert_ne!(
        regranted_incarnation, original_incarnation,
        "revoke/regrant cannot preserve a capability even if no intervening frame was sent"
    );

    let store = runtime.workspace_views.as_mut().unwrap();
    store
        .views
        .entry(crate::node::TEST_NODE, &path)
        .layout
        .remove(&id)
        .unwrap();
    store.generation += 1;
    runtime.sync_workspace_view();
    assert!(runtime.snapshot.browser_views.is_empty());
    assert_eq!(
        runtime.snapshot.browser_scopes.len(),
        1,
        "closing the final page preserves an authorized empty area"
    );
    assert_eq!(
        runtime.snapshot.browser_scopes[0].incarnation, regranted_incarnation,
        "page closure does not retire the still-authorized area"
    );
}

#[test]
fn browser_authority_regrant_survives_coalesced_session_updates() {
    let (runtime, _, directory) = strip_checkout("browser-authority-regrant");
    let path = directory.to_string_lossy().into_owned();
    let mut runtime = with_views(runtime, &directory.join("views.json"));
    with_tabs(&mut runtime, &directory);
    let store = runtime.workspace_views.as_mut().unwrap();
    let layout = &mut store.views.entry(crate::node::TEST_NODE, &path).layout;
    let browser = layout.new_browser_display("https://example.test/scoped", 1);
    layout.insert("a1", browser, 1).unwrap();
    store.generation += 1;
    let initial = runtime.snapshot_delta_payload(0);
    assert_eq!(runtime.snapshot.browser_views.len(), 1);
    let initial_incarnation = runtime.snapshot.browser_scopes[0].incarnation;

    assert!(runtime.ingest_session(Err(SessionFetchError::Unreachable(
        "fixture connection lost".into()
    ))));
    assert!(runtime.snapshot.browser_scopes.is_empty());
    // No snapshot is read between the failure and recovery, as with two
    // worker updates under one latched notification.
    with_tabs(&mut runtime, &directory);
    let recovered = runtime.snapshot_delta_payload(initial.revision);
    assert_ne!(
        runtime.snapshot.browser_scopes[0].incarnation, initial_incarnation,
        "a native client must observe a new grant even without an absent frame"
    );
    let recovered_incarnation = runtime.snapshot.browser_scopes[0].incarnation;
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(
        !runtime.ingest_session(Ok(tab_order_payload(
            &directory.to_string_lossy(),
            &tabs,
            &tabs,
            "w-order:t1"
        ))),
        "an unchanged session must not publish another change"
    );
    let unchanged = runtime.snapshot_delta_payload(recovered.revision);
    assert!(std::sync::Arc::ptr_eq(
        &recovered.current_rest.browser_views.value,
        &unchanged.current_rest.browser_views.value
    ));
    assert!(std::sync::Arc::ptr_eq(
        &recovered.current_rest.browser_scopes.value,
        &unchanged.current_rest.browser_scopes.value
    ));
    assert!(!std::sync::Arc::ptr_eq(
        &initial.current_rest.browser_scopes.value,
        &recovered.current_rest.browser_scopes.value
    ));
    assert_eq!(
        runtime.snapshot.browser_scopes[0].incarnation, recovered_incarnation,
        "unchanged session updates preserve authority"
    );

    assert!(runtime.ingest_session(Err(SessionFetchError::Unreachable(
        "fixture connection lost again".into()
    ))));
    let revoked = runtime.snapshot_delta_payload(recovered.revision);
    let wire: serde_json::Value =
        serde_json::from_slice(&serialize_snapshot_delta(&revoked).unwrap()).unwrap();
    assert_eq!(wire["rest"]["browser_scopes"], serde_json::json!([]));
    assert_eq!(wire["rest"]["browser_views"], serde_json::json!([]));
}

#[test]
fn browser_authority_records_remote_session_and_catalog_revocations_before_reads() {
    let directory = tempfile::tempdir().unwrap();
    let path = "/browser-authority-remote-fixture";
    let mut runtime = with_views_only(runtime(), &directory.path().join("views.json"));
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: "connecting".into(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut project = workspace(
        "remote-workspace",
        "Remote fixture",
        path,
        vec![checkout("remote-workspace", "remote-checkout", path, None)],
    );
    project.device_id = "mini".into();
    project.remote_target_id = Some("mini".into());
    let raw = RemoteSessionSnapshot {
        workspaces: vec![project],
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
    };
    let store = runtime.workspace_views.as_mut().unwrap();
    store.views.entry("mini", path);
    store.generation += 1;
    assert!(runtime.ingest_remote_session("mini", Ok(raw.clone())));
    let initial = runtime.snapshot_delta_payload(0);
    assert_eq!(runtime.snapshot.browser_scopes.len(), 1);
    let initial_incarnation = runtime.snapshot.browser_scopes[0].incarnation;

    assert!(runtime.ingest_remote_session(
        "mini",
        Err(SessionFetchError::Unreachable("fixture device lost".into()))
    ));
    assert!(runtime.snapshot.browser_scopes.is_empty());
    assert!(runtime.ingest_remote_session("mini", Ok(raw.clone())));
    let recovered = runtime.snapshot_delta_payload(initial.revision);
    let recovered_incarnation = runtime.snapshot.browser_scopes[0].incarnation;
    assert_ne!(recovered_incarnation, initial_incarnation);

    runtime
        .device_raw_sessions
        .get_mut("mini")
        .unwrap()
        .workspaces
        .clear();
    assert!(runtime.refresh_device_catalog("mini"));
    assert!(runtime.snapshot.browser_scopes.is_empty());
    runtime.device_raw_sessions.insert("mini".into(), raw);
    assert!(runtime.refresh_device_catalog("mini"));
    runtime.snapshot_delta_payload(recovered.revision);
    assert_ne!(
        runtime.snapshot.browser_scopes[0].incarnation, recovered_incarnation,
        "a catalog loss/regrant also retires the old grant before a read"
    );
}

/// The Overview says since when a device has not answered (PRD
/// overview-device-tasks B4): the clock starts at the first failure after
/// an answer, a failure that only changes its reason keeps it, and an
/// answer clears it from the wire.
#[test]
fn a_device_that_stops_answering_carries_since_when() {
    let mut runtime = runtime();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: "connecting".into(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let raw = RemoteSessionSnapshot {
        workspaces: Vec::new(),
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
    };
    let since = |runtime: &Runtime| runtime.snapshot.status.remote[0].unreachable_since_unix_ms;
    runtime.ingest_remote_session("mini", Ok(raw.clone()));
    assert_eq!(since(&runtime), None);

    let before = unix_milliseconds();
    assert!(runtime.ingest_remote_session(
        "mini",
        Err(SessionFetchError::Unreachable("connection lost".into()))
    ));
    let lost = since(&runtime).expect("the clock starts at the failure");
    assert!(lost >= before);
    runtime.ingest_remote_session(
        "mini",
        Err(SessionFetchError::SocketMissing("no server".into())),
    );
    assert_eq!(since(&runtime), Some(lost), "a new reason keeps the clock");
    let wire = serde_json::to_value(&runtime.snapshot.status.remote[0]).unwrap();
    assert_eq!(wire["unreachable_since_unix_ms"], serde_json::json!(lost));

    assert!(runtime.ingest_remote_session("mini", Ok(raw)));
    assert_eq!(since(&runtime), None);
    let wire = serde_json::to_value(&runtime.snapshot.status.remote[0]).unwrap();
    assert!(wire.get("unreachable_since_unix_ms").is_none());
}

fn with_tabs(runtime: &mut Runtime, directory: &Path) {
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
}

/// D-04, B8: choosing another Agent tab keeps the Workspace's document open
/// as the checkout's one list; without View
/// areas the terminal takes the canvas back.
#[test]
fn a_terminal_tab_choice_keeps_the_workspace_document_only_with_separate_areas() {
    for separate in [true, false] {
        let (runtime, checkout_id, directory) = strip_checkout("keeps-document");
        let mut runtime = if separate {
            with_new_views(runtime, "keeps-document")
        } else {
            runtime
        };
        with_tabs(&mut runtime, &directory);
        open(&mut runtime, &checkout_id, &directory.join("notes.md"));
        assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));

        runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2"));

        if separate {
            assert_eq!(
                active_label(&runtime).as_deref(),
                Some("notes.md"),
                "the checkout's list is the same for every tab"
            );
            assert_eq!(
                runtime.snapshot.editor.tabs.len(),
                1,
                "the document stays open"
            );
            runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1"));
            assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));
            let shown = runtime
                .snapshot_delta_payload(0)
                .documents
                .map(|documents| documents.visible)
                .unwrap_or_default();
            assert_eq!(shown, vec![runtime.snapshot.editor.tabs[0].id.clone()]);
        } else {
            assert_eq!(active_label(&runtime), None);
            assert!(runtime.snapshot.workspace_view.is_none());
        }
    }
}

/// PRD right-lane B1, B15, B16; lane-two-columns B6: a new Workspace shows
/// Agent Views alone; an explicit file open shows the documents in the
/// viewing column, opens no tool, numbers that call and lands the file; a
/// session update, and an agent or tab chosen from anywhere, move no lane
/// state the core keeps; a tab not shown before sees the checkout's one list.
#[test]
fn a_file_open_shows_the_documents_and_an_agent_choice_moves_no_lane_state() {
    let (runtime, checkout_id, directory) = strip_checkout("area-intent");
    let mut runtime = with_new_views(runtime, "area-intent");
    with_tabs(&mut runtime, &directory);
    assert_eq!(lane(&runtime), CLOSED, "Agent Views alone");
    assert_eq!(called(&runtime), 0);

    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert_eq!(lane(&runtime), DOCUMENTS);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(
        view.lane_called > 0 && view.lane_called == view.lane_calls,
        "the open is this Workspace's newest lane call"
    );
    let opened = view.lane_called;
    assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));

    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    assert_eq!(
        (lane(&runtime), called(&runtime)),
        (DOCUMENTS, opened),
        "a session update is not a request"
    );
    runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        (lane(&runtime), called(&runtime)),
        (DOCUMENTS, opened),
        "an agent choice moves no lane state and calls nothing"
    );
    assert_eq!(
        active_label(&runtime).as_deref(),
        Some("notes.md"),
        "a tab not shown before sees the checkout's one list"
    );
    layout(&mut runtime, serde_json::json!({"pinned": true}));
    runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t1"));
    assert_eq!(lane(&runtime), (true, true, None, true));
}

/// PRD sidebar-task-lens B26: the PRs tool's checkout filter is the
/// Workspace's own, sent with the tool, and a tool this build does not know
/// (the removed Work tool's name included) is refused with the lane unchanged.
#[test]
fn the_prs_checkout_filter_is_stored_and_an_unknown_tool_is_refused() {
    let (runtime, _checkout_id, directory) = strip_checkout("lane-work");
    let (_views, state) = views_path("lane-work");
    let mut runtime = with_views(runtime, &state);
    with_tabs(&mut runtime, &directory);
    let prs = |runtime: &Runtime| {
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .expect("a front Workspace")
            .prs
    };
    assert!(!prs(&runtime).only_checkout);
    layout(
        &mut runtime,
        serde_json::json!({"tool": "prs", "prs_only_checkout": true}),
    );
    assert_eq!(lane(&runtime).2, Some(Tool::Prs));
    assert!(prs(&runtime).only_checkout);

    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"tool": "work"}),
    ));
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(
        lane(&runtime).2,
        Some(Tool::Prs),
        "a refused call changes nothing, the tool included"
    );
}

/// PRD right-lane D-12, B2, B5, B7, B11, B20; lane-two-columns B14, B15,
/// B17: every lane field ends at the value sent, so a repeat changes
/// nothing and reports no change; a tool sent again is still a call, so a
/// page shows a lane it had hidden; a tool's close leaves the documents;
/// closing the lane keeps both columns, the pin and the View tree, and the
/// reopen brings them back; and the four widths are stored per Workspace
/// within the core's bound.
#[test]
fn each_lane_field_ends_at_its_sent_value_and_a_repeat_converges() {
    let (runtime, checkout_id, directory) = strip_checkout("lane-fields");
    let (_views, state) = views_path("lane-fields");
    let mut runtime = with_views(runtime, &state);
    with_tabs(&mut runtime, &directory);
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    layout(
        &mut runtime,
        serde_json::json!({"pinned": true, "tool": "explorer"}),
    );
    assert_eq!(lane(&runtime), (true, true, Some(Tool::Explorer), true));
    assert!(!runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"pinned": true})
    )));

    let before = called(&runtime);
    layout(&mut runtime, serde_json::json!({"tool": "prs"}));
    assert_eq!(lane(&runtime), (true, true, Some(Tool::Prs), true));
    assert_eq!(called(&runtime), before + 1);
    // The same tool again changes nothing stored but is a call the page
    // shows, so it reports a change.
    assert!(runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"tool": "prs"})
    )));
    assert_eq!(called(&runtime), before + 2);

    layout(&mut runtime, serde_json::json!({"tool": null}));
    assert_eq!(
        lane(&runtime),
        (true, true, None, true),
        "a tool's close leaves the documents (B14)"
    );
    layout(&mut runtime, serde_json::json!({"tool": "memory"}));
    layout(&mut runtime, serde_json::json!({"open": false}));
    assert_eq!(
        lane(&runtime),
        (false, true, Some(Tool::Memory), true),
        "closing the lane keeps both columns and the pin (B15)"
    );
    assert_eq!(called(&runtime), before + 3, "a close is no call");
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .layout
            .display_count,
        1,
        "closing the lane closes no view"
    );
    assert!(!runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"open": false})
    )));
    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(
        lane(&runtime),
        (true, true, Some(Tool::Memory), true),
        "the reopen brings both columns back"
    );
    assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));
    layout(&mut runtime, serde_json::json!({"documents": false}));
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Memory), true));

    layout(
        &mut runtime,
        serde_json::json!({"document_width": 700, "explorer_width": 300}),
    );
    layout(&mut runtime, serde_json::json!({"explorer_width": 99_999}));
    // Every column keeps a width of its own, so a wide document leaves the
    // others as they were.
    layout(
        &mut runtime,
        serde_json::json!({"panel_width": 440, "detail_width": 380}),
    );
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (
            view.document_width,
            view.detail_width,
            view.explorer_width,
            view.panel_width
        ),
        (
            Some(700),
            Some(380),
            Some(crate::workspace_views::MAX_WIDTH),
            Some(440)
        )
    );
    let (saved, _) = crate::workspace_views::load(&state, 0);
    let saved = saved.get(&view.device_id, &view.path).unwrap();
    assert_eq!(
        (
            saved.open,
            saved.documents,
            saved.tool,
            saved.pinned,
            saved.document_width,
            saved.panel_width
        ),
        (true, false, Some(Tool::Memory), true, Some(700), Some(440))
    );
    let written = std::fs::read_to_string(&state).unwrap();
    for retired in [
        "\"views\"",
        "\"tools\"",
        "views_width",
        "lane_called",
        "last_tool",
        "\"tree\"",
        "lane_width",
    ] {
        assert!(!written.contains(retired), "{retired} is not written");
    }
}

/// PRD lane-two-columns B15: a closed lane comes back in the shape it had,
/// however it was closed; a lane that never had a column opens on the
/// documents when the Workspace has some and on the Explorer when it has
/// none; the shape survives a restart.
#[test]
fn a_closed_lane_comes_back_in_the_shape_it_had_across_a_restart() {
    let (runtime, checkout_id, directory) = strip_checkout("lane-last-tool");
    let (_views, state) = views_path("lane-last-tool");
    let mut runtime = with_views(runtime, &state);
    with_tabs(&mut runtime, &directory);
    assert_eq!(lane(&runtime), CLOSED, "a lane that never opened");

    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(
        lane(&runtime),
        (true, false, Some(Tool::Explorer), false),
        "with no document the Explorer is what there is to show"
    );
    layout(&mut runtime, serde_json::json!({"tool": null}));
    assert_eq!(lane(&runtime), (true, false, None, false));
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    layout(&mut runtime, serde_json::json!({"documents": false}));
    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(
        lane(&runtime),
        DOCUMENTS,
        "with a document and no column, the documents come back"
    );

    layout(&mut runtime, serde_json::json!({"tool": "memory"}));
    layout(&mut runtime, serde_json::json!({"open": false}));
    assert_eq!(lane(&runtime), (false, true, Some(Tool::Memory), false));
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert_eq!(
        lane(&runtime),
        (true, true, Some(Tool::Changes), false),
        "a tool called while closed opens the lane with the documents back"
    );
    layout(&mut runtime, serde_json::json!({"open": false}));

    let (saved, _) = crate::workspace_views::load(&state, 0);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    let saved = saved.get(&view.device_id, &view.path).unwrap();
    assert_eq!(
        (saved.open, saved.documents, saved.tool),
        (false, true, Some(Tool::Changes))
    );
}

/// PRD lane-two-columns B5, B12: the Explorer opens with no view and opens
/// nothing (no document, no New tab page); a file picked there opens beside
/// it; closing the last view closes the viewing column and leaves the
/// Explorer; without a tool column the lane closes with the last view.
#[test]
fn the_explorer_opens_with_no_view_and_the_last_view_closes_the_viewing_column() {
    let (runtime, checkout_id, directory) = strip_checkout("views-empty");
    let mut runtime = with_new_views(runtime, "views-empty");
    layout(&mut runtime, serde_json::json!({"tool": "explorer"}));
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.tool, Some(Tool::Explorer));
    assert_eq!(view.layout.display_count, 0, "no New tab page is opened");
    assert!(!runtime.new_tab_visible());
    assert!(
        runtime.snapshot.ui_state.right_panel_visible
            && runtime.snapshot.ui_state.right_panel_section == RightPanelSection::Explorer,
        "the Explorer is read while its column shows"
    );

    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert_eq!(lane(&runtime), (true, true, Some(Tool::Explorer), false));
    close_only_display(&mut runtime);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (
            view.open,
            view.documents,
            view.tool,
            view.layout.display_count
        ),
        (true, false, Some(Tool::Explorer), 0),
        "the Explorer stays"
    );
    assert!(runtime.snapshot.ui_state.right_panel_visible);

    layout(&mut runtime, serde_json::json!({"tool": null}));
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert_eq!(lane(&runtime), DOCUMENTS);
    assert!(
        !runtime.snapshot.ui_state.right_panel_visible,
        "a document alone reads no tree"
    );
    close_only_display(&mut runtime);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (view.documents, view.tool, view.layout.display_count),
        (false, None, 0),
        "no column is left, so the lane closes"
    );
    assert!(!runtime.snapshot.ui_state.right_panel_visible);
}

/// PRD lane-two-columns B6, B7: a file, diff or page opened leaves the tool
/// column as it is, whatever it shows: from a closed lane the documents open
/// alone, and beside Changes the list stays.
#[test]
fn an_open_leaves_the_tool_column_as_it_is() {
    let (runtime, checkout_id, directory) = strip_checkout("tool-stays");
    let mut runtime = with_new_views(runtime, "tool-stays");
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert_eq!(lane(&runtime), DOCUMENTS, "from a closed lane");
    close_only_display(&mut runtime);
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert_eq!(
        lane(&runtime),
        (true, true, Some(Tool::Changes), false),
        "beside Changes, the file read before"
    );
    let page = |url: &str| {
        explorer_event(
            "browser_open",
            serde_json::json!({
                "url": url,
                "workspace": {"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()},
            }),
        )
    };
    layout(
        &mut runtime,
        serde_json::json!({"tool": "memory", "documents": false}),
    );
    runtime.dispatch_json(&page("not an address"));
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(
        lane(&runtime),
        (true, false, Some(Tool::Memory), false),
        "refused"
    );
    runtime.dispatch_json(&page("https://example.test/page"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(lane(&runtime), (true, true, Some(Tool::Memory), false));
}

/// PRD right-lane B16, D-13; lane-two-columns B6, B8: a reveal shows the
/// Explorer in the tool column, opens nothing and leaves the viewing column;
/// a terminal link's file shows the documents and leaves the tool column; a
/// linked folder shows the Explorer. Each is a lane call.
#[test]
fn a_reveal_or_a_linked_folder_shows_the_explorer_and_a_linked_file_the_documents() {
    let (runtime, _checkout_id, directory) = strip_checkout("tool-opens");
    let mut runtime = with_new_views(runtime, "tool-opens");
    assert!(!runtime.snapshot.ui_state.right_panel_visible);

    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Changes), false));
    let before = called(&runtime);
    layout(
        &mut runtime,
        serde_json::json!({"reveal": directory.join("notes.md").to_string_lossy()}),
    );
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Explorer), false));
    assert_eq!(called(&runtime), before + 1);
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .layout
            .display_count,
        0,
        "a reveal opens nothing"
    );

    layout(&mut runtime, serde_json::json!({"tool": "memory"}));
    let checkout_id = runtime
        .snapshot
        .navigator
        .focused_checkout_id
        .clone()
        .unwrap();
    runtime.dispatch_json(&explorer_event(
        "reveal_path",
        serde_json::json!({
            "path": directory.join("notes.md"), "workspace_id": runtime.snapshot.navigator.focused_workspace_id,
            "checkout_id": checkout_id, "is_directory": false,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        lane(&runtime),
        (true, true, Some(Tool::Memory), false),
        "a linked file shows the documents and leaves the tool"
    );
    assert_eq!(called(&runtime), before + 3);

    layout(&mut runtime, serde_json::json!({"open": false}));
    let folder = directory.join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    runtime.dispatch_json(&explorer_event(
        "reveal_path",
        serde_json::json!({
            "path": folder, "workspace_id": runtime.snapshot.navigator.focused_workspace_id,
            "checkout_id": checkout_id, "is_directory": true,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        lane(&runtime),
        (true, true, Some(Tool::Explorer), false),
        "a linked folder shows in the Explorer, the documents back beside it"
    );
    assert_eq!(called(&runtime), before + 4);
}

/// PRD right-lane D-01, A5; lane-two-columns B5: the lane is the
/// Workspace's, and the one global panel every existing reader gates on
/// follows its tool column: the Explorer is read only while its column
/// shows, the documents alone and a closed lane read nothing; a value no
/// column has, the retired `files` included, is refused.
#[test]
fn the_lane_drives_the_panel_the_readers_gate_on() {
    let (runtime, checkout_id, directory) = strip_checkout("tools");
    let mut runtime = with_new_views(runtime, "tools");
    let panel = |runtime: &Runtime| {
        (
            runtime.snapshot.ui_state.right_panel_visible,
            runtime.snapshot.ui_state.right_panel_section,
        )
    };
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert_eq!(panel(&runtime), (true, RightPanelSection::Changes));
    layout(&mut runtime, serde_json::json!({"tool": "prs"}));
    assert_eq!(panel(&runtime), (true, RightPanelSection::AgentSessions));
    layout(&mut runtime, serde_json::json!({"tool": "memory"}));
    assert_eq!(panel(&runtime), (true, RightPanelSection::Memory));
    layout(&mut runtime, serde_json::json!({"tool": "explorer"}));
    assert_eq!(panel(&runtime), (true, RightPanelSection::Explorer));

    layout(&mut runtime, serde_json::json!({"tool": null}));
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    assert!(!panel(&runtime).0, "a document alone reads no tree");
    layout(&mut runtime, serde_json::json!({"tool": "explorer"}));
    assert_eq!(panel(&runtime), (true, RightPanelSection::Explorer));

    layout(&mut runtime, serde_json::json!({"open": false}));
    assert!(!panel(&runtime).0);

    for refused in ["terminal", "files", "work"] {
        runtime.dispatch_json(&explorer_event(
            "workspace_view",
            serde_json::json!({"tool": refused}),
        ));
        assert_eq!(
            runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("workspace_view.unknown_tool")
        );
        assert_eq!(lane(&runtime), (false, true, Some(Tool::Explorer), false));
    }
}

/// B19: the daemon opens a checkout's root after the first snapshot names
/// it, so a restore waits for that root instead of reading too early, around
/// the pinned root, or marking every saved file unavailable; the tabs it has not restored yet
/// are not overwritten while it waits.
#[test]
fn a_restore_waits_for_the_daemon_to_open_the_checkout_root() {
    let (runtime, checkout_id, directory) = strip_checkout("waits-for-root");
    let (_views, state) = views_path("waits-for-root");
    let mut runtime = with_views(runtime, &state);
    // The test keeps the checkout folder: it restarts on that checkout after
    // the runtime is gone.
    let _checkout = hold_dirs(&mut runtime);
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    drop(runtime);

    let (restarted, checkout_id) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views_only(restarted, &state);
    let restored = |runtime: &Runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .filter(|tab| tab.checkout_id == checkout_id)
            .map(|tab| (tab.label.clone(), tab.unavailable_reason.is_some()))
            .collect::<Vec<_>>()
    };
    assert!(
        restored(&restarted).is_empty(),
        "nothing is read before the root is open"
    );
    restarted.sync_workspace_view();
    let waiting = |runtime: &Runtime| {
        let layout = &runtime.snapshot.workspace_view.as_ref().unwrap().layout;
        let crate::model::ViewNodeSnapshot::Area(area) = &layout.root else {
            panic!("one area");
        };
        area.displays
            .iter()
            .map(|display| (display.label.clone(), display.state, display.reason.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        waiting(&restarted),
        vec![(
            "notes.md".to_owned(),
            crate::model::ViewDisplayState::Waiting,
            Some("Waiting for this checkout to open".to_owned())
        )],
        "the saved display is kept, and says what it waits for"
    );
    assert!(
        !restarted.set_file_roots(crate::files::FileRoots::from_identities(Vec::new())),
        "a root set that does not hold the checkout still waits"
    );

    let root = std::fs::File::open(&directory).expect("open root");
    assert!(
        restarted.set_file_roots(crate::files::FileRoots::from_identities(
            hide_node::hold_roots(vec![(directory.clone(), root)]).1
        ))
    );
    assert_eq!(restored(&restarted), vec![("notes.md".to_owned(), false)]);
    restarted.sync_workspace_view();
    assert_eq!(
        waiting(&restarted),
        vec![(
            "notes.md".to_owned(),
            crate::model::ViewDisplayState::Open,
            None
        )]
    );
    assert!(
        !restarted.set_file_roots(crate::files::FileRoots::from_identities(Vec::new())),
        "a Workspace is restored once per process"
    );
}

/// D-10: a file this build cannot read is left for the operator and the
/// defaults load with a diagnostic, never a screen alert.
#[test]
fn an_unreadable_views_file_is_kept_and_reported_in_the_diagnostic_log() {
    let (_views, state) = views_path("unreadable");
    std::fs::write(&state, b"{broken").expect("fixture");
    let (store, diagnostics) = WorkspaceViewStore::open(state.clone(), Default::default());
    drop(store);
    assert_eq!(
        diagnostics
            .into_iter()
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>(),
        vec!["workspace_views.unreadable"]
    );
    assert!(!state.exists());
    let kept = std::fs::read_dir(state.parent().unwrap())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("workspace-views.json.unreadable-")
        });
    assert!(kept);
}

/// B8: coming back to a Workspace that has no Herdr tab shows the View tab
/// that was active there, not the newest one.
#[test]
fn returning_to_a_workspace_without_agent_tabs_keeps_its_active_view_tab() {
    let (runtime, checkout_id, directory) = strip_checkout("return-active");
    let mut runtime = with_new_views(runtime, "return-active");
    let other = directory.with_file_name(format!(
        "{}-other",
        directory.file_name().unwrap().to_string_lossy()
    ));
    std::fs::create_dir_all(&other).expect("second checkout");
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:other".to_owned(),
            label: "other".to_owned(),
            path: other.to_string_lossy().into_owned(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        });
    runtime.rebuild_catalog();
    let other_checkout = workspace::checkout_id_for_path("workspace:other", &other);
    std::fs::write(directory.join("later.md"), "later\n").expect("second file");
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    open(&mut runtime, &checkout_id, &directory.join("later.md"));
    let notes = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.label == "notes.md")
        .expect("notes tab")
        .id
        .clone();
    assert!(runtime.dispatch_json(&explorer_event(
        "file_focus",
        serde_json::json!({"tab_id": notes})
    )));
    assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));

    let focus = |workspace_id: &str, checkout_id: &str| {
        explorer_event(
            "focus_checkout",
            serde_json::json!({"workspace_id": workspace_id, "checkout_id": checkout_id}),
        )
    };
    runtime.dispatch_json(&focus("workspace:other", &other_checkout));
    runtime.dispatch_json(&focus("workspace:order", &checkout_id));

    assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));
}

/// PRD right-lane B16, AGENTS.md one event per action: revealing a file
/// opens the files tool with its tree and unfolds its folders in the same
/// `workspace_view`, and a path outside the Workspace is refused without
/// showing anything.
#[test]
fn a_reveal_shows_the_explorer_and_unfolds_the_folders_in_one_event() {
    let (runtime, _checkout_id, directory) = strip_checkout("reveal");
    let mut runtime = with_new_views(runtime, "reveal");
    let root = directory.to_string_lossy().into_owned();

    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"reveal": "/elsewhere/a.txt"}),
    ));
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(lane(&runtime), CLOSED);
    runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"reveal": format!("{root}/../elsewhere/a.txt")}),
    ));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("workspace_view.reveal_outside"),
        "a path that climbs out of the Workspace is refused, not unfolded"
    );
    assert_eq!(lane(&runtime), CLOSED);
    assert_eq!(called(&runtime), 0);

    layout(
        &mut runtime,
        serde_json::json!({"reveal": format!("{root}/src/deep/a.rs")}),
    );
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Explorer), false));
    let expanded = &runtime.snapshot.ui_state.expanded_paths;
    assert!(expanded.contains(&format!("{root}/src")));
    assert!(expanded.contains(&format!("{root}/src/deep")));
}

/// The daemon's second checkout, registered beside the strip checkout.
pub(super) fn second_checkout(runtime: &mut Runtime, directory: &Path) -> (PathBuf, String) {
    let other = directory.with_file_name(format!(
        "{}-second",
        directory.file_name().unwrap().to_string_lossy()
    ));
    std::fs::create_dir_all(&other).expect("second checkout");
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(WorkspaceRegistration {
            primary_checkout_id: None,
            id: "workspace:other".to_owned(),
            label: "other".to_owned(),
            path: other.to_string_lossy().into_owned(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        });
    runtime.rebuild_catalog();
    let checkout = workspace::checkout_id_for_path("workspace:other", &other);
    (other, checkout)
}

fn saved_displays(runtime: &Runtime, path: &Path) -> Vec<String> {
    runtime
        .workspace_views
        .as_ref()
        .and_then(|store| {
            store
                .views
                .get(crate::node::TEST_NODE, &path.to_string_lossy())
        })
        .map(|entry| {
            entry
                .layout
                .displays()
                .map(|display| display.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// B19: a restore opens into the Workspace in front, so when the front moved
/// after the last sync saw it, nothing is opened until the sync catches up;
/// otherwise one Workspace's saved documents would land in another.
#[test]
fn a_restore_waits_while_the_front_moved_since_the_last_sync() {
    let (runtime, checkout_id, directory) = strip_checkout("front-moved");
    let (_views, state) = views_path("front-moved");
    let mut runtime = with_views(runtime, &state);
    // The test keeps the checkout folder: it restarts on that checkout after
    // the runtime is gone.
    let _checkout = hold_dirs(&mut runtime);
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    drop(runtime);

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views_only(restarted, &state);
    let (other, other_checkout) = second_checkout(&mut restarted, &directory);
    restarted.snapshot.navigator.focused_workspace_id = Some("workspace:other".to_owned());
    restarted.snapshot.navigator.focused_checkout_id = Some(other_checkout.clone());

    let roots = [directory.clone(), other.clone()]
        .into_iter()
        .map(|path| {
            let file = std::fs::File::open(&path).expect("open root");
            (path, file)
        })
        .collect();
    assert!(
        !restarted.set_file_roots(crate::files::FileRoots::from_identities(
            hide_node::hold_roots(roots).1
        )),
        "the Workspace the sync saw is no longer in front"
    );
    assert!(
        restarted
            .snapshot
            .editor
            .tabs
            .iter()
            .all(|tab| tab.checkout_id != other_checkout),
        "no saved tab lands in the other Workspace"
    );
    restarted.sync_workspace_view();
    assert_eq!(
        saved_displays(&restarted, &directory),
        vec![directory.join("notes.md").to_string_lossy().into_owned()],
        "the Workspace that did not come back keeps its saved displays"
    );
}

/// B20: a restored tab whose file was missing reads it again when the file
/// is opened after it came back, rather than saying it is unavailable.
#[test]
fn opening_a_file_that_came_back_clears_its_unavailable_tab() {
    let (runtime, checkout_id, directory) = strip_checkout("came-back");
    let (_views, state) = views_path("came-back");
    let mut runtime = with_views(runtime, &state);
    // The test keeps the checkout folder: it restarts on that checkout after
    // the runtime is gone.
    let _checkout = hold_dirs(&mut runtime);
    std::fs::write(directory.join("gone.md"), "gone\n").expect("fixture");
    open(&mut runtime, &checkout_id, &directory.join("gone.md"));
    drop(runtime);
    std::fs::remove_file(directory.join("gone.md")).expect("remove fixture");
    let (restarted, checkout_id) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views(restarted, &state);
    let gone = |runtime: &Runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .filter(|tab| tab.label == "gone.md")
            .map(|tab| tab.unavailable_reason.is_some())
            .collect::<Vec<_>>()
    };
    assert_eq!(gone(&restarted), vec![true]);

    std::fs::write(directory.join("gone.md"), "back\n").expect("fixture");
    open(&mut restarted, &checkout_id, &directory.join("gone.md"));

    assert_eq!(gone(&restarted), vec![false]);
    assert_eq!(active_label(&restarted).as_deref(), Some("gone.md"));
    let shown = restarted
        .snapshot_delta_payload(0)
        .documents
        .expect("a fresh reader gets the documents on screen");
    assert_eq!(
        shown
            .changed
            .iter()
            .map(|(_, document)| document.contents_utf8.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("back\n")]
    );
}

/// D-11: the app opens on the Workspace the operator last chose, after a
/// reload or a restart; a first run, or a front only Herdr's own focus put
/// there, starts on Main.
#[test]
fn only_the_workspace_the_operator_chose_is_resumed() {
    let (runtime, checkout_id, directory) = strip_checkout("resumed");
    let (_views, state) = views_path("resumed");
    let mut runtime = with_views(runtime, &state);
    let resumed = |runtime: &Runtime| runtime.snapshot.workspace_view.as_ref().map(|v| v.resumed);
    assert_eq!(resumed(&runtime), Some(false), "a first run starts on Main");
    drop(runtime);

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    runtime = with_views(restarted, &state);
    assert_eq!(
        resumed(&runtime),
        Some(false),
        "a front the operator never chose is not resumed"
    );
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:order", "checkout_id": checkout_id}),
    ));
    assert_eq!(resumed(&runtime), Some(true), "a reload opens it");
    drop(runtime);

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    let restarted = with_views(restarted, &state);
    assert_eq!(resumed(&restarted), Some(true), "so does a restart");
}

/// D-10, B20: the tools each Workspace shows drive the one right panel the
/// readers gate on, but the older settings file keeps the panel it had.
#[test]
fn workspace_tools_never_reach_the_older_settings_file() {
    let (runtime, _checkout_id, _directory) = strip_checkout("settings");
    let saved = (
        runtime.snapshot.ui_state.right_panel_visible,
        runtime.snapshot.ui_state.right_panel_section,
    );
    let mut runtime = with_new_views(runtime, "settings");
    layout(
        &mut runtime,
        serde_json::json!({"tool": if saved.0 { serde_json::Value::Null } else { "changes".into() }}),
    );
    assert_eq!(runtime.snapshot.ui_state.right_panel_visible, !saved.0);
    let written = runtime.ui_state_to_save();
    assert_eq!(
        (written.right_panel_visible, written.right_panel_section),
        saved
    );
}

/// Removing a device forgets its Workspaces' layouts and View tabs with the
/// rest of Hide's record of it.
#[test]
fn removing_a_device_forgets_its_workspace_views() {
    let (runtime, _checkout_id, directory) = strip_checkout("device-views");
    let mut runtime = with_new_views(runtime, "device-views");
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: "studio".to_owned(),
            label: "studio".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: "studio".to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent: None,
        });
    let store = runtime.workspace_views.as_mut().unwrap();
    store.views.entry("studio", "/srv/app").tool = Some(Tool::Explorer);
    store
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy());
    runtime.apply_area_intent_to(
        &("studio".to_owned(), "/srv/app".to_owned()),
        AreaIntent::Documents,
    );

    assert!(runtime.dispatch_json(&explorer_event(
        "remove_device",
        serde_json::json!({"device_id": "studio"}),
    )));

    let views = &runtime.workspace_views.as_ref().unwrap().views;
    assert!(views.get("studio", "/srv/app").is_none());
    assert!(
        views
            .get(crate::node::TEST_NODE, &directory.to_string_lossy())
            .is_some()
    );
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .lane_called
            .keys()
            .all(|(device, _)| device != "studio")
    );
}

/// Lane-call history follows the existing bounded Workspace retention.
#[test]
fn workspace_lane_calls_forget_evicted_workspaces() {
    let (runtime, _, _) = strip_checkout("lane-calls-cap");
    let mut runtime = with_new_views(runtime, "lane-calls-cap");
    for index in 0..=crate::workspace_views::MAX_WORKSPACES {
        let key = ("studio".to_owned(), format!("/srv/workspace-{index}"));
        runtime.apply_area_intent_to(&key, AreaIntent::Documents);
    }
    let store = runtime.workspace_views.as_ref().unwrap();
    assert!(store.lane_called.len() <= crate::workspace_views::MAX_WORKSPACES);
    assert!(
        store
            .lane_called
            .keys()
            .all(|(device, path)| store.views.get(device, path).is_some())
    );
    let last = (
        "studio".to_owned(),
        format!("/srv/workspace-{}", crate::workspace_views::MAX_WORKSPACES),
    );
    assert_eq!(
        store.lane_called.get(&last),
        Some(&(store.lane_calls, crate::model::LaneCallSide::View))
    );
}

/// PRD lane-two-columns B12: closing the last view, a page or a file,
/// pinned or not, closes the viewing column; with a tool column the lane
/// stays on it, without one the lane has nothing left and closes; either
/// way the pin stays, with that state saved.
#[test]
fn closing_the_last_view_closes_the_viewing_column_and_the_lane_without_a_tool() {
    for pinned in [true, false] {
        for browser in [true, false] {
            for tool in [Some(Tool::Explorer), None] {
                let (runtime, checkout_id, directory) = strip_checkout("last-view-lane");
                let (_views, state) = views_path("last-view-lane");
                let mut runtime = with_views(runtime, &state);
                with_tabs(&mut runtime, &directory);
                if browser {
                    runtime.dispatch_json(&explorer_event(
                        "browser_open",
                        serde_json::json!({"url": "about:blank"}),
                    ));
                } else {
                    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
                }
                layout(
                    &mut runtime,
                    serde_json::json!({"pinned": pinned, "tool": tool.map(|_| "explorer")}),
                );
                assert_eq!(lane(&runtime), (true, true, tool, pinned));
                close_only_display(&mut runtime);
                let view = runtime.snapshot.workspace_view.as_ref().unwrap();
                assert_eq!(
                    (view.layout.display_count, view.documents, view.tool),
                    (0, false, tool),
                    "pinned {pinned}, page {browser}, tool {tool:?}"
                );
                assert_eq!(lane(&runtime), (true, false, tool, pinned));
                let (saved, _) = crate::workspace_views::load(&state, 0);
                let saved = saved.get(&view.device_id, &view.path).unwrap();
                assert_eq!(
                    (saved.documents, saved.tool, saved.pinned),
                    (false, tool, pinned)
                );
            }
        }
    }
}

/// PRD right-lane B12: only the last view closing closes the viewing
/// column; a close that leaves another view keeps the documents.
#[test]
fn a_close_that_leaves_a_view_keeps_the_documents() {
    let (runtime, checkout_id, directory) = strip_checkout("one-of-two-views");
    let mut runtime = with_new_views(runtime, "one-of-two-views");
    std::fs::write(directory.join("other.md"), "# Other\n").expect("fixture");
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    open(&mut runtime, &checkout_id, &directory.join("other.md"));
    assert_eq!(lane(&runtime), DOCUMENTS);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.layout.display_count, 2);
    let workspace = serde_json::json!({"device_id": view.device_id, "path": view.path});
    let crate::model::ViewNodeSnapshot::Area(area) = &view.layout.root else {
        panic!("both files open in the one area");
    };
    let first = area.displays[0].id.clone();
    runtime.dispatch_json(&explorer_event(
        "view_layout",
        serde_json::json!({"workspace": workspace, "action": "close", "display_id": first}),
    ));
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!((view.documents, view.layout.display_count), (true, 1));
    close_only_display(&mut runtime);
    assert_eq!(lane(&runtime), (true, false, None, false));
}

/// B20: width acknowledgements identify the accepted intent, including a
/// no-op width, without adding UI bookkeeping to the saved Workspace file.
#[test]
fn workspace_width_requests_are_bounded_ephemeral_and_confirm_exact_intents() {
    let (runtime, _checkout_id, directory) = strip_checkout("width-requests");
    let (_views, state) = views_path("width-requests");
    let mut runtime = with_views(runtime, &state);
    // The test keeps the checkout folder: it restarts on that checkout after
    // the runtime is gone.
    let _checkout = hold_dirs(&mut runtime);
    with_tabs(&mut runtime, &directory);
    layout(
        &mut runtime,
        serde_json::json!({"document_width": 672, "width_request_id": "divider:first"}),
    );
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .width_request_id
            .as_deref(),
        Some("divider:first")
    );
    layout(
        &mut runtime,
        serde_json::json!({"document_width": 704, "width_request_id": "divider:second"}),
    );
    let published = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(published.document_width, Some(704));
    assert_eq!(
        published.width_request_id.as_deref(),
        Some("divider:second")
    );
    assert!(!runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"document_width": 704, "width_request_id": "divider:second"})
    )));
    let saved = std::fs::read_to_string(&state).unwrap();
    assert!(runtime.dispatch_json(&explorer_event(
        "workspace_view",
        serde_json::json!({"document_width": 704, "width_request_id": "divider:third"})
    )));
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .width_request_id
            .as_deref(),
        Some("divider:third")
    );
    assert_eq!(std::fs::read_to_string(&state).unwrap(), saved);
    assert!(!saved.contains("width_request_id"));
    layout(&mut runtime, serde_json::json!({"pinned": true}));
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .width_request_id
            .as_deref(),
        Some("divider:third")
    );
    // The other tools' divider confirms its intent the same way.
    layout(
        &mut runtime,
        serde_json::json!({"panel_width": 448, "width_request_id": "divider:panel"}),
    );
    let published = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(
        (published.panel_width, published.width_request_id.as_deref()),
        (Some(448), Some("divider:panel"))
    );
    layout(&mut runtime, serde_json::json!({"document_width": 650}));
    assert_eq!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .width_request_id,
        None
    );
    for invalid in [String::new(), "x".repeat(129)] {
        assert!(runtime.dispatch_json(&explorer_event(
            "workspace_view",
            serde_json::json!({"document_width": 720, "width_request_id": invalid})
        )));
        assert_eq!(
            runtime.snapshot.status.last_error.as_ref().unwrap().kind,
            "workspace_view.invalid_width_request"
        );
        assert_eq!(
            runtime
                .snapshot
                .workspace_view
                .as_ref()
                .unwrap()
                .document_width,
            Some(650)
        );
    }
    let published = runtime.snapshot.workspace_view.as_ref().unwrap();
    let key = (published.device_id.clone(), published.path.clone());
    drop(runtime);
    let (reloaded, _) = crate::workspace_views::load(&state, 0);
    assert_eq!(
        reloaded.get(&key.0, &key.1).unwrap().document_width,
        Some(650)
    );
    std::fs::remove_dir_all(state.parent().unwrap()).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_explicit_file_visit_supersedes_a_delayed_memory_visit() {
    let (runtime, checkout_id, directory) = strip_checkout("file-supersedes-memory");
    let mut runtime = with_new_views(runtime, "file-supersedes-memory");
    open(&mut runtime, &checkout_id, &directory.join("notes.md"));
    let tab = runtime.snapshot.editor.tabs[0].id.clone();
    let key = (
        crate::node::TEST_NODE.to_owned(),
        directory.to_string_lossy().into_owned(),
    );
    runtime.choose_when_in_front(crate::node::TEST_NODE, "old-memory", key);
    runtime.attach_memory_navigation(
        crate::node::TEST_NODE,
        "old-memory",
        "workspace:order".into(),
    );
    runtime.dispatch_json(&explorer_event(
        "file_focus",
        serde_json::json!({"tab_id": tab}),
    ));
    runtime.acknowledge_choice(crate::node::TEST_NODE, "old-memory");
    runtime.sync_workspace_view();
    assert_eq!(active_label(&runtime).as_deref(), Some("notes.md"));
    assert_ne!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tool,
        Some(Tool::Memory)
    );
    assert!(
        runtime
            .snapshot
            .memory
            .as_ref()
            .is_none_or(|memory| memory.project.is_none())
    );
}
