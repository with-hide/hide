use super::*;

/// A layout update for one tab changes that tab's entry and no other, so
/// redrawing one tab cannot disturb what another tab draws.
#[test]
fn tab_layouts_update_only_the_tab_whose_layout_changed() {
    let checkout_path = "/private/tmp/hide-tab-layouts-one";
    let (mut runtime, _checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    let before = runtime.snapshot().pane_layouts.clone();

    let mut next = tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1");
    next.layouts
        .iter_mut()
        .find(|layout| layout.tab_id == "w-order:t2")
        .expect("the second tab's layout")
        .zoomed = true;
    assert!(runtime.ingest_session(Ok(next)));

    let after = runtime.snapshot().pane_layouts.clone();
    assert_eq!(after.len(), before.len());
    for (was, now) in before.iter().zip(after.iter()) {
        if now.tab_id == "w-order:t2" {
            assert!(!was.zoomed);
            assert!(now.zoomed);
        } else {
            assert_eq!(was, now);
        }
    }
}

/// A layout Herdr has already sent still moves the canvas when it belongs
/// to another tab. The return value is what fires the change notifier, so
/// it has to report the projection that was rebuilt and not only the
/// geometry that was compared. Before layouts survived a tab switch the
/// two could not disagree, because a switch emptied the stored layout and
/// every following layout counted as new.
#[test]
fn tab_layouts_report_a_projection_move_under_an_unchanged_layout() {
    let checkout_path = "/private/tmp/hide-tab-layouts-notify";
    let (mut runtime, _checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));

    let second = runtime
        .snapshot()
        .pane_layouts
        .iter()
        .find(|layout| layout.tab_id == "w-order:t2")
        .expect("the second tab's layout")
        .clone();

    // The geometry is the one already stored, so only the projection
    // moves. That move still has to be reported.
    assert!(runtime.apply_pane_layout(second.clone(), false));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );

    // The same layout over the same projection changes nothing, and
    // reports nothing, so the canvas is not redrawn for a repeat.
    assert!(!runtime.apply_pane_layout(second, false));
}

#[test]
fn tab_order_is_unchanged_by_a_tab_switch() {
    let checkout_path = "/private/tmp/hide-tab-order-focus";
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));

    let focus = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_tab",
        "payload": {
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "tab_id": "w-order:t3"
        }
    }))
    .expect("focus event");
    assert!(runtime.dispatch_json(&focus));

    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        tabs.map(str::to_owned).to_vec(),
        "a tab switch must not move the tab it switched to"
    );
    // The active mark moves on the dispatch that asked for it, because
    // Hide owns the visible tab. It is still never inferred from
    // position: the strip order above did not change.
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout_id).as_deref(),
        Some("w-order:t3")
    );
    assert_eq!(runtime.snapshot().tab.id.as_deref(), Some("w-order:t3"));
}

#[test]
fn file_view_mode_is_per_open_tab_and_reopen_starts_live_and_wrapped() {
    let (mut runtime, checkout_id, directory) = strip_checkout("file-view-mode");
    let first = directory.join("notes.md");
    let second = directory.join("second.md");
    std::fs::write(&first, "# Original").unwrap();
    std::fs::write(&second, "# Second").unwrap();
    open_file(&mut runtime, &checkout_id, &first);
    let first_id = runtime.snapshot.editor.active_tab_id.clone().unwrap();
    assert!(runtime.snapshot.editor.tabs.last().unwrap().wrap);
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "file_view",
        "payload": {"tab_id": first_id, "markdown_live": false, "wrap": false}
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&event));
    assert!(
        !runtime.dispatch_json(&event),
        "repeated selection publishes no change"
    );
    open_file(&mut runtime, &checkout_id, &second);
    let second_tab = runtime.snapshot.editor.tabs.last().unwrap();
    assert!(second_tab.markdown_live);
    assert!(second_tab.wrap);
    open_file(&mut runtime, &checkout_id, &first);
    let tab = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.id == first_id)
        .unwrap();
    assert!(!tab.markdown_live);
    assert!(!tab.wrap);
    let close = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "file_close", "payload": {"tab_id": first_id}
    }))
    .unwrap();
    runtime.dispatch_json(&close);
    open_file(&mut runtime, &checkout_id, &first);
    let reopened = runtime.snapshot.editor.tabs.last().unwrap();
    assert!(reopened.markdown_live);
    assert!(reopened.wrap);
}

#[test]
fn diff_tab_opens_wrapped_and_keeps_its_wrap_choice() {
    let (mut runtime, checkout_id, directory) = strip_checkout("diff-wrap");
    let file = directory.join("notes.md");
    runtime.show_diff_tab(
        "workspace:order",
        &checkout_id,
        &file.to_string_lossy(),
        false,
        false,
    );
    let tab_id = runtime.snapshot.editor.active_tab_id.clone().unwrap();
    let wrap = |runtime: &Runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .unwrap()
            .wrap
    };
    assert!(wrap(&runtime));
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "file_view",
        "payload": {"tab_id": tab_id, "markdown_live": true, "wrap": false}
    }))
    .unwrap();
    assert!(runtime.dispatch_json(&event));
    assert!(!wrap(&runtime));
}

#[test]
fn recent_navigation_restores_file_and_diff_across_projects_atomically() {
    for diff in [false, true] {
        let (mut runtime, checkout_id, directory) = strip_checkout("recent-surface");
        let file = directory.join("notes.md");
        if diff {
            runtime.show_diff_tab(
                "workspace:order",
                &checkout_id,
                &file.to_string_lossy(),
                false,
                false,
            );
        } else {
            open_file(&mut runtime, &checkout_id, &file);
        }
        let tab_id = runtime.snapshot.editor.active_tab_id.clone().unwrap();
        runtime.snapshot.navigator.focused_workspace_id = Some("other-project".into());
        runtime.snapshot.navigator.focused_checkout_id = Some("other-checkout".into());
        runtime.deactivate_editor_tab();
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "file_focus", "payload": {"tab_id": tab_id}
        }))
        .unwrap();
        assert!(runtime.dispatch_json(&event));
        assert_eq!(
            runtime.snapshot.navigator.focused_workspace_id.as_deref(),
            Some("workspace:order")
        );
        assert_eq!(
            runtime.snapshot.navigator.focused_checkout_id.as_deref(),
            Some(checkout_id.as_str())
        );
        assert_eq!(
            runtime.snapshot.editor.active_tab_id.as_deref(),
            Some(tab_id.as_str())
        );
        assert_eq!(runtime.snapshot.editor.document.is_some(), !diff);
        if diff {
            assert_eq!(
                runtime.snapshot.changes.selected_path.as_deref(),
                Some(file.to_str().unwrap())
            );
        }
        let previous = runtime.snapshot.editor.active_tab_id.clone();
        let invalid = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "file_focus", "payload": {"tab_id": "deleted-tab"}
        })).unwrap();
        assert!(runtime.dispatch_json(&invalid));
        assert_eq!(runtime.snapshot.editor.active_tab_id, previous);
        assert_eq!(
            runtime.snapshot.status.last_error.as_ref().unwrap().kind,
            "file.focus_failed"
        );
        runtime.snapshot.navigator.workspaces.clear();
        assert!(runtime.dispatch_json(&event));
        assert_eq!(runtime.snapshot.editor.active_tab_id, previous);
        assert_eq!(
            runtime.snapshot.status.last_error.as_ref().unwrap().kind,
            "editor.invalid_context"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn tab_strip_lists_herdr_tabs_and_then_the_file_that_was_opened() {
    let directory_folder = scratch_dir("hide-strip-open-");
    let directory = directory_folder.path().to_path_buf();
    std::fs::create_dir_all(&directory).expect("checkout directory");
    // The temp root is a symlink on macOS; the catalog keys checkouts by
    // the real path, so the fixture has to use it too. The fixture is also
    // made its own repository, because a directory inside another
    // repository is catalogued under that repository's root instead.
    let directory = directory.canonicalize().expect("a real checkout path");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&directory)
            .status()
            .expect("git init runs")
            .success()
    );
    let file = directory.join("notes.md");
    std::fs::write(&file, "notes\n").expect("fixture file");
    let checkout_path = directory.to_string_lossy().into_owned();
    let (mut runtime, checkout_id) = tab_order_runtime(&checkout_path);

    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec!["herdr:w-order:t1".to_owned(), "herdr:w-order:t2".to_owned()]
    );

    open_file(&mut runtime, &checkout_id, &file);
    let with_file = strip_ids(&runtime, &checkout_id);
    assert_eq!(with_file.len(), 3);
    assert_eq!(
        &with_file[..2],
        &["herdr:w-order:t1".to_owned(), "herdr:w-order:t2".to_owned()]
    );
    assert!(with_file[2].starts_with("file:"));
    assert_eq!(strip_labels(&runtime, &checkout_id)[2], "notes.md");

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_strip_appends_a_reopened_file_nowhere_and_a_new_herdr_tab_at_the_end() {
    let directory_folder = scratch_dir("hide-strip-append-");
    let directory = directory_folder.path().to_path_buf();
    std::fs::create_dir_all(&directory).expect("checkout directory");
    // The temp root is a symlink on macOS; the catalog keys checkouts by
    // the real path, so the fixture has to use it too. The fixture is also
    // made its own repository, because a directory inside another
    // repository is catalogued under that repository's root instead.
    let directory = directory.canonicalize().expect("a real checkout path");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&directory)
            .status()
            .expect("git init runs")
            .success()
    );
    let file = directory.join("notes.md");
    std::fs::write(&file, "notes\n").expect("fixture file");
    let checkout_path = directory.to_string_lossy().into_owned();
    let (mut runtime, checkout_id) = tab_order_runtime(&checkout_path);

    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    open_file(&mut runtime, &checkout_id, &file);
    let opened = strip_ids(&runtime, &checkout_id);

    // Opening a file that is already open activates its tab; it does not
    // add a second one.
    open_file(&mut runtime, &checkout_id, &file);
    assert_eq!(strip_ids(&runtime, &checkout_id), opened);

    // A Herdr tab created next to an open file goes to the end of the
    // strip, not in front of the file that was there first.
    let grown = ["w-order:t1", "w-order:t2", "w-order:t3"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &grown,
        &grown,
        "w-order:t1"
    ))));
    let after = strip_ids(&runtime, &checkout_id);
    assert_eq!(after.len(), 4);
    assert_eq!(&after[..3], &opened[..]);
    assert_eq!(after[3], "herdr:w-order:t3");

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_strip_keeps_a_file_in_its_slot_while_herdr_reorders_around_it() {
    let directory_folder = scratch_dir("hide-strip-slot-");
    let directory = directory_folder.path().to_path_buf();
    std::fs::create_dir_all(&directory).expect("checkout directory");
    // The temp root is a symlink on macOS; the catalog keys checkouts by
    // the real path, so the fixture has to use it too. The fixture is also
    // made its own repository, because a directory inside another
    // repository is catalogued under that repository's root instead.
    let directory = directory.canonicalize().expect("a real checkout path");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&directory)
            .status()
            .expect("git init runs")
            .success()
    );
    let file = directory.join("notes.md");
    std::fs::write(&file, "notes\n").expect("fixture file");
    let checkout_path = directory.to_string_lossy().into_owned();
    let (mut runtime, checkout_id) = tab_order_runtime(&checkout_path);

    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    open_file(&mut runtime, &checkout_id, &file);
    let file_entry = strip_ids(&runtime, &checkout_id)[2].clone();

    // The operator dropped the file tab between the two Herdr tabs. The
    // move itself is the reorder event's job; what matters here is that
    // the slot the file took is the slot it keeps.
    runtime.checkout_tab_order.insert(
        checkout_id.clone(),
        vec![
            "herdr:w-order:t1".to_owned(),
            file_entry.clone(),
            "herdr:w-order:t2".to_owned(),
        ],
    );
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &checkout_path,
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t1".to_owned(),
            file_entry.clone(),
            "herdr:w-order:t2".to_owned()
        ]
    );

    // Herdr swapped its two tabs. They swap slots; the file does not move.
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
            file_entry,
            "herdr:w-order:t1".to_owned()
        ]
    );

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn tab_strip_reorder_moves_a_file_tab_without_asking_herdr() {
    let (mut runtime, checkout_id, directory) = strip_checkout("file-move");
    let tabs = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    open_file(&mut runtime, &checkout_id, &directory.join("notes.md"));
    let file_entry = strip_ids(&runtime, &checkout_id)[2].clone();

    // There is no live connection in this fixture, so a move that reached
    // Herdr would fail loudly. Landing silently is the assertion.
    assert!(reorder_tab(&mut runtime, &checkout_id, &file_entry, 1));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t1".to_owned(),
            file_entry.clone(),
            "herdr:w-order:t2".to_owned()
        ]
    );
    assert!(runtime.snapshot().status.last_error.is_none());
    assert!(runtime.pending_tab_move.is_empty());

    // The slot survives the next catalog rebuild, which is what makes the
    // move a move rather than a repaint.
    runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(
        strip_ids(&runtime, &checkout_id),
        vec![
            "herdr:w-order:t1".to_owned(),
            file_entry,
            "herdr:w-order:t2".to_owned()
        ]
    );

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn a_closed_projected_pane_retargets_to_the_remaining_pane_in_its_checkout() {
    let mut runtime = runtime();
    let checkout_path = "/tmp/hide-closed-selected-pane";
    let paths = workspace::paths_here([checkout_path]);
    let checkout_path = paths.comparison(checkout_path);
    let checkout_path = checkout_path.as_str();
    let workspace_id = workspace::workspace_id_for_path(Path::new(checkout_path));
    let checkout_id = workspace::checkout_id_for_path(&workspace_id, Path::new(checkout_path));
    let registration = WorkspaceRegistration {
        primary_checkout_id: None,
        id: workspace_id.clone(),
        label: "Closed pane".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    };
    let previous_workspace = workspace(
        &workspace_id,
        "Closed pane",
        checkout_path,
        vec![checkout(
            &workspace_id,
            &checkout_id,
            checkout_path,
            Some(pane("w-close:p1", checkout_path)),
        )],
    );
    let current_workspace = workspace(
        &workspace_id,
        "Closed pane",
        checkout_path,
        vec![checkout(&workspace_id, &checkout_id, checkout_path, None)],
    );
    runtime.snapshot.ui_state.workspace_registrations = vec![registration.clone()];
    runtime.snapshot.navigator.workspaces = vec![previous_workspace];
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.ui_state.selected_pane_id = Some("w-close:p1".to_owned());
    runtime.snapshot.terminal.pane_id = Some("w-close:p1".to_owned());
    runtime.snapshot.focused.pane_id = Some("w-close:p1".to_owned());
    runtime.snapshot.pane_layouts = vec![PaneLayoutSnapshot {
        workspace_id: "w-close".to_owned(),
        tab_id: "w-close:t1".to_owned(),
        focused_pane_id: "w-close:p1".to_owned(),
        zoomed: false,
        root: PaneLayoutNodeSnapshot::Pane {
            pane_id: "w-close:p1".to_owned(),
        },
    }];
    runtime.restore_hint_pending = false;

    let payload: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "panes": [{"pane_id": "w-close:p2", "cwd": checkout_path}],
        "focused_pane_id": "w-close:p2",
        "tabs": [{"workspace_id": "w-close", "tab_id": "w-close:t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w-close",
            "tab_id": "w-close:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w-close:p2",
            "panes": [{
                "pane_id": "w-close:p2",
                "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
            }],
            "splits": []
        }]
    }))
    .expect("remaining pane payload");

    assert!(runtime.ingest_session_with_catalog(
        Ok(payload),
        Some(session_sync::PrecomputedCatalog {
            worktrees: runtime.worktree_catalog(),
            registrations: vec![registration],
            workspaces: vec![current_workspace],
            // This fixture's checkout has answered facts; unknown paths are
            // deliberately held apart by the asynchronous catalog contract.
            paths: paths.into(),
        }),
    ));
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-close:p2")
    );
    assert_eq!(
        runtime.snapshot().ui_state.selected_pane_id.as_deref(),
        Some("w-close:p2")
    );
    assert_eq!(
        runtime
            .snapshot()
            .active_pane_layout()
            .map(|layout| layout.focused_pane_id.as_str()),
        Some("w-close:p2")
    );
    assert!(runtime.snapshot().status.last_error.is_none());
}

/// R2, AC2. The read record pass prunes text scales on the same tick, and
/// it may only drop what it has the authority to drop. Unscoped it deleted
/// every remote pane's zoom on the next local sync, and it deleted the file
/// editor's zoom on every agent state change, because the editor's scale
/// was keyed into the pane map under a name no pane is ever reported under.
#[test]
fn read_record_change_leaves_remote_and_editor_zoom_alone() {
    let mut runtime = runtime();
    let state_path = runtime.state_path.clone();
    let zoom_pane = |runtime: &mut Runtime, pane_id: &str| {
        let event = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "pane_text_scale",
            "payload": {"pane_id": pane_id, "direction": "in"}
        }))
        .expect("pane text scale event");
        assert!(runtime.dispatch_json(&event));
    };
    let idle_payload = || -> SessionSnapshotPayload {
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [{
                "pane_id": "w1:p1",
                "workspace_label": "Fixture",
                "agent": "codex",
                "agent_status": "idle",
                "tokens": {"status_idle": "\u{25cb}", "activity": "0000000000002"}
            }],
            "tabs": [{"workspace_id": "w1", "tab_id": "t1", "label": ""}],
            "layouts": [{
                "workspace_id": "w1", "tab_id": "t1", "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": "w1:p1",
                "panes": [{"pane_id": "w1:p1",
                           "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": []
            }]
        }))
        .expect("idle payload")
    };

    runtime.ingest_session(Ok(working_payload()));
    zoom_pane(&mut runtime, "w1:p1");
    zoom_pane(&mut runtime, "remote:mini:pane:w9:p1");
    zoom_pane(&mut runtime, "w1:p9");
    let editor_zoom = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "editor_text_scale",
        "payload": {"direction": "in"}
    }))
    .expect("editor text scale event");
    assert!(runtime.dispatch_json(&editor_zoom));
    assert_eq!(runtime.snapshot().ui_state.editor_text_scale, 1.1);

    // The focused pane's state moves, so the read record moves and the
    // prune runs. This is the tick that used to lose both zooms.
    runtime.ingest_session(Ok(idle_payload()));
    let scales = runtime.snapshot().ui_state.pane_text_scales.clone();
    assert_eq!(
        scales.get("remote:mini:pane:w9:p1"),
        Some(&1.1),
        "a local sync holds no remote pane list and must not prune remote keys"
    );
    assert_eq!(
        scales.get("w1:p1"),
        Some(&1.1),
        "a live pane keeps its zoom"
    );
    assert!(
        !scales.contains_key("w1:p9"),
        "a local pane the server stopped reporting still loses its zoom"
    );
    assert_eq!(
        runtime.snapshot().ui_state.editor_text_scale,
        1.1,
        "the editor is not a pane, so a pane prune cannot reach its zoom"
    );

    // A navigator or keyboard save carries the editor zoom through for the
    // same reason it carries the pane map through.
    let ui_state_update = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_state_update",
        "payload": {"left_sidebar_visible": false}
    }))
    .expect("ui state update event");
    runtime.dispatch_json(&ui_state_update);
    assert_eq!(runtime.snapshot().ui_state.editor_text_scale, 1.1);

    let stored = std::fs::read_to_string(&state_path).expect("state file");
    assert!(
        stored.contains("editor_text_scale"),
        "the editor zoom is persisted, so it survives a restart"
    );
    let _ = std::fs::remove_file(&state_path);
}

#[test]
fn ingesting_the_same_snapshot_twice_reports_no_further_change() {
    let mut runtime = runtime();
    assert!(runtime.ingest_session(Ok(working_payload())));
    assert!(
        !runtime.ingest_session(Ok(working_payload())),
        "an unchanged snapshot must not wake the shell on every refresh"
    );
}

/// AC5, R2, SC1. One click on a file printed by a pane in another checkout
/// brings that checkout forward, opens the tree on it, expands every
/// ancestor, selects the file and opens its editor tab - in one event,
/// because the shell's dispatch is fire-and-forget and could not order
/// four of them.
#[test]
fn revealing_a_file_switches_the_checkout_and_settles_the_whole_screen_at_once() {
    let (mut runtime, _first, first_id, second, second_id) = reveal_runtime();
    let target = second.join("deep/nested/leaf/target.txt");
    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some(first_id.as_str())
    );

    assert!(runtime.dispatch_json(&reveal_event("workspace:1", &second_id, &target, false)));

    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some(second_id.as_str())
    );
    assert!(snapshot.ui_state.right_panel_visible);
    assert_eq!(
        snapshot.ui_state.right_panel_section,
        RightPanelSection::Explorer
    );
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(target.to_string_lossy().as_ref())
    );
    for ancestor in ["deep", "deep/nested", "deep/nested/leaf"] {
        let expected = second.join(ancestor).to_string_lossy().into_owned();
        assert!(
            snapshot.ui_state.expanded_paths.contains(&expected),
            "the tree does not open {expected}: {:?}",
            snapshot.ui_state.expanded_paths
        );
    }
    assert!(
        !snapshot
            .ui_state
            .expanded_paths
            .contains(&target.to_string_lossy().into_owned()),
        "a file is not a folder to expand"
    );
    let tab = snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.path == target.to_string_lossy())
        .expect("the revealed file takes an editor tab");
    assert_eq!(
        snapshot.editor.active_tab_id.as_deref(),
        Some(tab.id.as_str())
    );
    assert!(snapshot.status.last_error.is_none());

    // Rule 11: the same click again keeps the tab it already opened and
    // the selection it already made.
    assert!(runtime.dispatch_json(&reveal_event("workspace:1", &second_id, &target, false)));
    assert_eq!(runtime.snapshot().editor.tabs.len(), 1);
    assert_eq!(
        runtime.snapshot().ui_state.selected_path.as_deref(),
        Some(target.to_string_lossy().as_ref())
    );
}

/// AC5, R3, SC2. A folder opens the tree on itself and takes no editor
/// AC5, R4. A reveal settles the whole screen at once or not at all. A
/// file that cannot be read - deleted between being printed and being
/// clicked, or unreadable - leaves the focused checkout, the panel, the
/// expanded set and the selection exactly as they were, and says why.
/// Reading after the screen has moved would leave a reveal half applied.
#[test]
fn revealing_a_file_that_cannot_be_read_moves_nothing_and_says_why() {
    let (mut runtime, _first, _first_id, second, second_id) = reveal_runtime();
    let missing = second.join("deep/nested/leaf/gone.txt");
    let before = runtime.snapshot().clone();

    assert!(runtime.dispatch_json(&reveal_event("workspace:1", &second_id, &missing, false)));

    let after = runtime.snapshot().clone();
    assert_eq!(
        after.navigator.focused_checkout_id, before.navigator.focused_checkout_id,
        "the checkout must not move for a file that cannot be read"
    );
    assert_eq!(
        after.ui_state.right_panel_visible,
        before.ui_state.right_panel_visible
    );
    assert_eq!(
        after.ui_state.right_panel_section,
        before.ui_state.right_panel_section
    );
    assert_eq!(
        after.ui_state.expanded_paths,
        before.ui_state.expanded_paths
    );
    assert_eq!(after.ui_state.selected_path, before.ui_state.selected_path);
    assert!(
        after.editor.tabs.is_empty(),
        "no tab for a file with no contents"
    );
    let error = after
        .status
        .last_error
        .as_ref()
        .expect("an unreadable file is reported");
    assert_eq!(error.kind, "file.open_failed");
}

/// tab. A4 records that the folder itself expands, not only its ancestors.
#[test]
fn revealing_a_folder_expands_it_and_opens_no_editor_tab() {
    let (mut runtime, _first, _first_id, second, second_id) = reveal_runtime();
    let target = second.join("deep/nested");

    assert!(runtime.dispatch_json(&reveal_event("workspace:1", &second_id, &target, true)));

    let snapshot = runtime.snapshot();
    assert!(snapshot.ui_state.right_panel_visible);
    assert_eq!(
        snapshot.ui_state.right_panel_section,
        RightPanelSection::Explorer
    );
    assert!(
        snapshot
            .ui_state
            .expanded_paths
            .contains(&target.to_string_lossy().into_owned()),
        "the clicked folder itself stays closed: {:?}",
        snapshot.ui_state.expanded_paths
    );
    assert!(
        snapshot
            .ui_state
            .expanded_paths
            .contains(&second.join("deep").to_string_lossy().into_owned())
    );
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(target.to_string_lossy().as_ref())
    );
    assert!(
        snapshot.editor.tabs.is_empty(),
        "a folder is not a document"
    );
}

/// AC5, R5. A reveal aimed at a checkout Hide does not have leaves the
/// screen exactly as it was and says why.
#[test]
fn revealing_into_an_unregistered_checkout_changes_nothing_and_reports_it() {
    let (mut runtime, first, first_id, second, _second_id) = reveal_runtime();
    let before = runtime.snapshot().ui_state.clone();

    assert!(runtime.dispatch_json(&reveal_event(
        "workspace:9",
        "checkout:missing",
        &second.join("deep"),
        true
    )));

    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("reveal.unknown_checkout")
    );
    assert_eq!(
        snapshot.navigator.focused_checkout_id.as_deref(),
        Some(first_id.as_str())
    );
    assert_eq!(
        snapshot.ui_state.right_panel_visible,
        before.right_panel_visible
    );
    assert_eq!(
        snapshot.ui_state.right_panel_section,
        before.right_panel_section
    );
    assert_eq!(snapshot.ui_state.expanded_paths, before.expanded_paths);
    assert_eq!(snapshot.ui_state.selected_path, before.selected_path);
    assert!(snapshot.editor.tabs.is_empty());
    let _ = first;
}

#[test]
fn selecting_a_change_opens_one_diff_tab_and_keeps_its_reader_alive() {
    let (mut runtime, root, checkout_id, _second, _second_id) = reveal_runtime();
    let path = root.join("tracked.json");
    runtime.snapshot.changes.edit().root_path = Some(root.to_string_lossy().into_owned());
    runtime.snapshot.changes.edit().entries = vec![crate::model::ChangedFileSnapshot {
        path: path.to_string_lossy().into_owned(),
        relative_path: "tracked.json".to_owned(),
        previous_relative_path: None,
        status: crate::model::ChangedFileStatus::Modified,
        added_lines: Some(2),
        removed_lines: Some(1),
    }];
    let event = || {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "changes_select",
            "payload": {"path": path, "committed": false}
        }))
        .expect("changes event")
    };

    assert!(runtime.dispatch_json(&event()));
    assert!(
        !runtime.dispatch_json(&event()),
        "opening the active diff publishes no duplicate state"
    );

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.editor.tabs.len(), 1);
    let tab = &snapshot.editor.tabs[0];
    assert_eq!(tab.kind, EditorTabKind::Diff);
    assert_eq!(tab.checkout_id, checkout_id);
    assert_eq!(tab.diff_committed, Some(false));
    assert_eq!(
        snapshot.editor.active_tab_id.as_deref(),
        Some(tab.id.as_str())
    );
    assert!(snapshot.editor.document.is_none());
    assert!(!snapshot.ui_state.right_panel_visible);
    let request = runtime
        .changes_request()
        .expect("an active diff keeps its Changes reader alive");
    assert_eq!(
        request.selected_path.as_deref(),
        Some(path.to_string_lossy().as_ref())
    );

    std::fs::write(&path, "{}\n").expect("file fixture");
    runtime.open_file_tab(
        "workspace:0",
        &checkout_id,
        path.to_string_lossy().as_ref(),
        false,
    );
    assert_eq!(runtime.snapshot().editor.tabs.len(), 2);
    assert_eq!(
        runtime
            .snapshot()
            .editor
            .tabs
            .iter()
            .map(|tab| tab.kind)
            .collect::<Vec<_>>(),
        vec![EditorTabKind::Diff, EditorTabKind::File],
        "opening the source does not reuse its diff tab"
    );
}

#[test]
fn explorer_visibility_keeps_one_checkout_scoped_changes_reader_alive() {
    let (mut runtime, root, _checkout_id, _second, _second_id) = reveal_runtime();
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Explorer;

    let request = runtime
        .changes_request()
        .expect("a visible Explorer requests Git decorations");
    assert_eq!(request.root_path, root);
    assert!(request.selected_path.is_none());

    runtime.snapshot.ui_state.right_panel_visible = false;
    assert!(
        runtime.changes_request().is_none(),
        "a hidden Explorer adds no background Git work"
    );
}

/// The machine's node, counting each `Changes` call it answers.
struct CountingChanges {
    changes: std::sync::atomic::AtomicUsize,
}

impl crate::node_access::NodeLink for CountingChanges {
    fn call(
        &self,
        call: hide_node_link::protocol::Call,
        timeout: std::time::Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        if matches!(call, hide_node_link::protocol::Call::Changes { .. }) {
            self.changes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        hide_node::Local::of_process().call(call, timeout)
    }
}

/// With no renderer attached History asks its node nothing, whatever panel
/// the operator left open, and the first renderer back reads the same
/// checkout again: one `Changes` call per change of what is shown.
#[test]
fn the_changes_reader_asks_nothing_while_no_renderer_is_attached() {
    let (mut runtime, root, _checkout_id, _second, _second_id) = reveal_runtime();
    let node = std::sync::Arc::new(CountingChanges {
        changes: std::sync::atomic::AtomicUsize::new(0),
    });
    runtime.own_node = node.clone();
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    let mut reader = crate::changes::ChangesReader::new();
    let mut wake = |runtime: &mut Runtime| {
        let _ = reader.read_if_due(runtime.changes_request());
        reader.join_pending();
        let _ = reader.read_if_due(runtime.changes_request());
        node.changes.load(std::sync::atomic::Ordering::SeqCst)
    };
    let attached = |runtime: &mut Runtime, attached: bool| {
        runtime.dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": 2, "kind": "ui_attached", "payload": {"attached": attached}
            }))
            .unwrap(),
        );
    };
    assert_eq!(wake(&mut runtime), 1);
    attached(&mut runtime, false);
    assert!(runtime.changes_request().is_none());
    assert_eq!([wake(&mut runtime), wake(&mut runtime)], [1, 1]);
    attached(&mut runtime, true);
    assert_eq!(
        runtime.changes_request().map(|request| request.root_path),
        Some(root.to_string_lossy().into_owned()),
        "the panel left open shows the same checkout again"
    );
    assert_eq!(wake(&mut runtime), 2);
}

/// B3, D-04: a create lands on disk, settles the slot, and moves the
/// selection to the new item without touching the expanded set.
#[test]
fn explorer_create_writes_the_item_and_selects_it() {
    let (mut runtime, root) = explorer_runtime();
    let src = root.join("src").to_string_lossy().into_owned();
    runtime.snapshot.ui_state.expanded_paths = vec![src.clone()];

    assert!(runtime.dispatch_json(&explorer_event(
        "file_create",
        serde_json::json!({"root": root.to_string_lossy(), "parent": src, "name": "new.rs"})
    )));

    let expected = root.join("src/new.rs");
    assert!(expected.is_file());
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a settled slot");
    assert_eq!(operation.phase, "finished");
    assert_eq!(operation.kind, "file_create");
    assert_eq!(operation.destination, expected.to_string_lossy());
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert_eq!(snapshot.ui_state.expanded_paths, vec![src]);

    assert!(runtime.dispatch_json(&explorer_event(
        "dir_create",
        serde_json::json!({"root": root.to_string_lossy(), "parent": root.to_string_lossy(), "name": "docs"})
    )));
    assert!(root.join("docs").is_dir());
    std::fs::remove_dir_all(&root).ok();
}

/// B6, B11: renaming an expanded folder keeps it expanded under its new
/// name, leaves every other expanded folder alone, and an open tab on
/// a file inside it now saves to where the file is.
#[test]
fn explorer_rename_carries_expansion_and_open_tabs_to_the_new_path() {
    let (mut runtime, root) = explorer_runtime();
    let checkout_id = workspace::checkout_id_for_path("workspace:0", &root);
    let src = root.join("src").to_string_lossy().into_owned();
    let nested = root.join("src/nested").to_string_lossy().into_owned();
    runtime.snapshot.ui_state.expanded_paths = vec![src.clone(), nested.clone()];
    let lib = root.join("src/lib.rs").to_string_lossy().into_owned();
    let tab = |id: &str, workspace_id: &str, checkout_id: &str| EditorTabSnapshot {
        id: id.to_owned(),
        workspace_id: workspace_id.to_owned(),
        checkout_id: checkout_id.to_owned(),
        path: lib.clone(),
        label: "lib.rs".to_owned(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: false,
        preview: false,
        unavailable_reason: None,
    };
    runtime
        .snapshot
        .editor
        .tabs
        .push(tab("file:lib", "workspace:0", &checkout_id));
    // The same path in a checkout on another device is another file.
    runtime.snapshot.editor.tabs.push(tab(
        "file:device",
        "remote:macbook:project:x",
        "remote:macbook:checkout:w1",
    ));
    let (document, place) = files::open_document(
        &hide_node::Local::of_process(),
        &files::DocumentRoot {
            device_id: crate::node::TEST_NODE.to_owned(),
            path: root.to_string_lossy().into_owned(),
            identity: None,
        },
        &lib,
    )
    .expect("fixture document");
    runtime
        .editor_documents
        .insert("file:lib".to_owned(), crate::model::Edited::new(document));
    runtime.document_places.insert("file:lib".to_owned(), place);

    assert!(runtime.dispatch_json(&explorer_event(
        "path_rename",
        serde_json::json!({"root": root.to_string_lossy(), "path": src, "name": "lib"})
    )));

    let renamed = root.join("lib");
    assert!(renamed.join("lib.rs").is_file());
    assert!(!root.join("src").exists());
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot
            .explorer_operation
            .as_ref()
            .map(|o| o.phase.as_str()),
        Some("finished")
    );
    assert_eq!(
        snapshot.ui_state.expanded_paths,
        vec![
            renamed.to_string_lossy().into_owned(),
            renamed.join("nested").to_string_lossy().into_owned()
        ]
    );
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(renamed.to_string_lossy().as_ref())
    );
    assert_eq!(
        snapshot.editor.tabs[0].path,
        renamed.join("lib.rs").to_string_lossy()
    );
    assert_eq!(
        snapshot.editor.tabs[1].path, lib,
        "another device's tab stays"
    );
    assert_eq!(
        runtime.editor_documents["file:lib"].path,
        renamed.join("lib.rs").to_string_lossy()
    );
    // The tab's saves go to the file it now shows, not to the old path.
    assert_eq!(
        runtime.document_places["file:lib"].relative.as_str(),
        "lib/lib.rs"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// B8, B9: a move lands under the destination folder and a move onto an
/// existing name is refused with the tree unchanged.
#[test]
fn explorer_move_relocates_the_item_and_refuses_a_name_clash() {
    let (mut runtime, root) = explorer_runtime();
    let lib = root.join("src/lib.rs");
    let nested = root.join("src/nested");

    assert!(runtime.dispatch_json(&explorer_event(
        "path_move",
        serde_json::json!({"root": root.to_string_lossy(), "path": lib.to_string_lossy(), "destination": nested.to_string_lossy()})
    )));
    assert!(nested.join("lib.rs").is_file());
    assert!(!lib.exists());
    assert_eq!(
        runtime.snapshot().ui_state.selected_path.as_deref(),
        Some(nested.join("lib.rs").to_string_lossy().as_ref())
    );

    std::fs::write(&lib, "again\n").expect("fixture file");
    assert!(runtime.dispatch_json(&explorer_event(
        "path_move",
        serde_json::json!({"root": root.to_string_lossy(), "path": lib.to_string_lossy(), "destination": nested.to_string_lossy()})
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a settled slot");
    assert_eq!(operation.phase, "failed");
    assert!(
        operation
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("already exists"),
        "{:?}",
        operation.message
    );
    assert_eq!(operation.path, lib.to_string_lossy());
    assert_eq!(std::fs::read_to_string(&lib).unwrap(), "again\n");
    assert_eq!(
        std::fs::read_to_string(nested.join("lib.rs")).unwrap(),
        "lib\n"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// B4, B6, D-05: a trash removes the item, moves the selection to the
/// row the tree named, drops the expanded folders that left with it,
/// and keeps the file tab that was open on it.
#[test]
fn explorer_trash_removes_the_item_selects_the_named_row_and_keeps_its_tab() {
    let (mut runtime, root) = explorer_runtime();
    // The items really reach the account Trash; they carry names nothing
    // else there has and are removed from it at the end (files.rs).
    let (file_name, folder_name) = crate::files::tests::unique_trash_names();
    let src = root.join(&folder_name);
    let lib = src.join(&file_name);
    let nested = src.join("nested");
    std::fs::create_dir_all(&nested).expect("fixture folder");
    std::fs::write(&lib, "lib\n").expect("fixture file");
    runtime.snapshot.ui_state.expanded_paths = vec![
        src.to_string_lossy().into_owned(),
        nested.to_string_lossy().into_owned(),
    ];
    runtime.snapshot.editor.tabs.push(EditorTabSnapshot {
        id: "file:lib".to_owned(),
        workspace_id: "workspace:0".to_owned(),
        checkout_id: workspace::checkout_id_for_path("workspace:0", &root),
        path: lib.to_string_lossy().into_owned(),
        label: file_name.clone(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: false,
        preview: false,
        unavailable_reason: None,
    });

    let mut document = files::tests::open_local(&lib).0;
    document.contents_utf8 = Some("draft\n".to_owned());
    document.dirty = true;
    runtime
        .editor_documents
        .insert("file:lib".to_owned(), crate::model::Edited::new(document));

    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": lib.to_string_lossy(),
            "select_after": nested.to_string_lossy(),
        })
    )));
    assert!(!lib.exists(), "the file left the tree");
    // The tab keeps its draft and shows the file as removed, so a save does
    // not look as if it could land (B18).
    let document = &runtime.editor_documents["file:lib"];
    assert_eq!(document.contents_utf8.as_deref(), Some("draft\n"));
    assert!(document.dirty);
    assert_eq!(
        document
            .conflict
            .as_ref()
            .map(|conflict| conflict.disk_revision.clone()),
        Some(None)
    );
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a settled slot");
    assert_eq!(operation.kind, "path_trash");
    assert_eq!(operation.phase, "finished");
    assert_eq!(operation.path, lib.to_string_lossy());
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(nested.to_string_lossy().as_ref()),
        "the selection moves to the row the tree named"
    );
    assert_eq!(
        snapshot
            .editor
            .tabs
            .iter()
            .map(|tab| tab.path.as_str())
            .collect::<Vec<_>>(),
        vec![lib.to_string_lossy().as_ref()],
        "the open tab stays (B6)"
    );

    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": src.to_string_lossy(),
            "select_after": root.to_string_lossy(),
        })
    )));
    assert!(!src.exists(), "the folder left the tree");
    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(root.to_string_lossy().as_ref()),
        "with no sibling the parent is selected"
    );
    assert!(
        snapshot.ui_state.expanded_paths.is_empty(),
        "expanded folders inside the trashed folder are dropped: {:?}",
        snapshot.ui_state.expanded_paths
    );
    std::fs::remove_dir_all(&root).ok();
    crate::files::tests::remove_from_trash(&[&file_name, &folder_name]);
}

/// D-03: the confirm carries the inode the prompt was built from, and an
/// item replaced under the open modal is refused rather than moved.
#[test]
fn explorer_trash_refuses_an_item_replaced_while_the_prompt_was_open() {
    let (mut runtime, root) = explorer_runtime();
    let lib = root.join("src/lib.rs");
    let shown = u64::try_from(
        hide_platform::fs::identity::file_id_nofollow(&lib)
            .expect("fixture")
            .index(),
    )
    .expect("a 64-bit file number");
    std::fs::rename(&lib, root.join("src/old.rs")).expect("keep the shown inode alive");
    std::fs::write(&lib, "rewritten").expect("replacement");

    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": lib.to_string_lossy(),
            "select_after": root.join("src").to_string_lossy(),
            "inode": shown.to_string(),
        })
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot.explorer_operation.as_ref().expect("a failed slot");
    assert_eq!(operation.phase, "failed");
    assert_eq!(
        operation.message.as_deref(),
        Some("lib.rs changed while the prompt was open; nothing was moved")
    );
    assert_eq!(
        std::fs::read_to_string(&lib).expect("still there"),
        "rewritten"
    );
    assert_eq!(snapshot.ui_state.selected_path, None);
    std::fs::remove_dir_all(&root).ok();
}

/// The inode a trash sends is the listing's decimal text, so an id past
/// what a browser number holds arrives exact, and the item it names goes.
#[test]
fn explorer_trash_takes_the_item_the_prompt_named_by_its_inode_text() {
    let (mut runtime, root) = explorer_runtime();
    let lib = root.join("src/lib.rs");
    let shown = hide_platform::fs::identity::file_id_nofollow(&lib)
        .expect("fixture")
        .index();
    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": lib.to_string_lossy(),
            "select_after": root.join("src").to_string_lossy(),
            "inode": shown.to_string(),
        })
    )));
    assert!(!lib.exists(), "the named item left the tree");
    std::fs::remove_dir_all(&root).ok();
}

/// D-02, D-06: a selection that would leave with the item, and an item
/// that is already gone, are both refused with the item untouched and
/// the reason on the slot.
#[test]
fn explorer_trash_refuses_a_selection_inside_the_item_and_a_missing_item() {
    let (mut runtime, root) = explorer_runtime();
    let src = root.join("src");
    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": src.to_string_lossy(),
            "select_after": src.join("lib.rs").to_string_lossy(),
        })
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a refused slot");
    assert_eq!(operation.phase, "failed");
    assert!(
        operation
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("cannot move into the item"),
        "{:?}",
        operation.message
    );
    assert!(src.join("lib.rs").is_file(), "nothing moved");

    let gone = src.join("gone.rs");
    assert!(runtime.dispatch_json(&explorer_event(
        "path_trash",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": gone.to_string_lossy(),
            "select_after": src.to_string_lossy(),
        })
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot.explorer_operation.as_ref().expect("a failed slot");
    assert_eq!(operation.phase, "failed");
    assert_eq!(
        operation.message.as_deref(),
        Some("gone.rs no longer exists")
    );
    assert_eq!(operation.path, gone.to_string_lossy());
    assert_eq!(
        snapshot.ui_state.selected_path, None,
        "a failure moves nothing"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// D-04: a path outside the focused checkout, or a root that is not the
/// focused checkout, is refused before any filesystem call and the
/// refusal is readable under the row the request named.
#[test]
fn explorer_refuses_paths_outside_the_focused_checkout_without_touching_disk() {
    let (mut runtime, root) = explorer_runtime();
    let outside_folder = scratch_dir("hide-explorer-outside-");
    let outside = outside_folder.path().to_path_buf();
    std::fs::create_dir_all(&outside).expect("outside dir");

    assert!(runtime.dispatch_json(&explorer_event(
        "file_create",
        serde_json::json!({"root": root.to_string_lossy(), "parent": outside.to_string_lossy(), "name": "leak"})
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a refused slot");
    assert_eq!(operation.phase, "failed");
    assert!(
        operation
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("outside the workspace"),
        "{:?}",
        operation.message
    );
    assert_eq!(operation.path, outside.to_string_lossy());
    assert!(!outside.join("leak").exists());
    assert_eq!(snapshot.ui_state.selected_path, None);

    assert!(runtime.dispatch_json(&explorer_event(
        "dir_create",
        serde_json::json!({"root": outside.to_string_lossy(), "parent": outside.to_string_lossy(), "name": "leak"})
    )));
    let snapshot = runtime.snapshot();
    let operation = snapshot
        .explorer_operation
        .as_ref()
        .expect("a refused slot");
    assert_eq!(operation.phase, "failed");
    assert!(
        operation
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("not the focused checkout"),
        "{:?}",
        operation.message
    );
    assert!(!outside.join("leak").exists());
    std::fs::remove_dir_all(&outside).ok();
    std::fs::remove_dir_all(&root).ok();
}

/// D-12, B13: New File opens the created file as an editor tab in the
/// same result - one frame, no second dispatch - and it is the active
/// tab. New Folder opens nothing, and Rename opens no new tab.
#[test]
fn explorer_file_create_opens_the_created_file_as_a_tab_and_others_do_not() {
    let root_folder = scratch_dir("hide-explorer-open-");
    let root = root_folder.path().to_path_buf();
    std::fs::create_dir_all(root.join("src")).expect("fixture tree");
    let root = root.canonicalize().expect("a real root");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&root)
            .status()
            .expect("git init runs")
            .success()
    );
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:0".to_owned(),
        label: "workspace 0".to_owned(),
        path: root.to_string_lossy().into_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    let checkout_id = workspace::checkout_id_for_path("workspace:0", &root);
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:0".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.navigator.root_path = Some(root.to_string_lossy().into_owned());

    let src = root.join("src").to_string_lossy().into_owned();
    assert!(runtime.dispatch_json(&explorer_event(
        "file_create",
        serde_json::json!({"root": root.to_string_lossy(), "parent": src, "name": "new.rs"})
    )));

    let created = root.join("src/new.rs");
    assert!(created.is_file());
    let snapshot = runtime.snapshot();
    let file_tabs: Vec<_> = snapshot
        .editor
        .tabs
        .iter()
        .filter(|tab| tab.kind == EditorTabKind::File)
        .collect();
    assert_eq!(file_tabs.len(), 1, "the created file takes exactly one tab");
    assert_eq!(file_tabs[0].path, created.to_string_lossy());
    assert_eq!(
        snapshot.editor.active_tab_id.as_deref(),
        Some(file_tabs[0].id.as_str()),
        "the created file's tab is active"
    );
    assert_eq!(
        snapshot
            .explorer_operation
            .as_ref()
            .and_then(|operation| operation.message.as_deref()),
        None,
        "a readable created file opens without a failure reason"
    );

    // A folder created next opens no tab.
    assert!(runtime.dispatch_json(&explorer_event(
        "dir_create",
        serde_json::json!({"root": root.to_string_lossy(), "parent": src, "name": "docs"})
    )));
    assert_eq!(
        runtime
            .snapshot()
            .editor
            .tabs
            .iter()
            .filter(|tab| tab.kind == EditorTabKind::File)
            .count(),
        1,
        "New Folder opens no editor tab"
    );

    // Renaming the created file opens no new tab; its one tab is
    // retargeted to the new path.
    assert!(runtime.dispatch_json(&explorer_event(
        "path_rename",
        serde_json::json!({
            "root": root.to_string_lossy(),
            "path": created.to_string_lossy(),
            "name": "renamed.rs"
        })
    )));
    let snapshot = runtime.snapshot();
    let file_tabs: Vec<_> = snapshot
        .editor
        .tabs
        .iter()
        .filter(|tab| tab.kind == EditorTabKind::File)
        .collect();
    assert_eq!(file_tabs.len(), 1, "Rename opens no new tab");
    assert_eq!(
        file_tabs[0].path,
        root.join("src/renamed.rs").to_string_lossy()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn retarget_path_is_component_wise() {
    assert_eq!(
        retarget_path("/repo/src", "/repo/src", "/repo/lib").as_deref(),
        Some("/repo/lib")
    );
    assert_eq!(
        retarget_path("/repo/src/a/b", "/repo/src", "/repo/lib").as_deref(),
        Some("/repo/lib/a/b")
    );
    assert_eq!(retarget_path("/repo/src2", "/repo/src", "/repo/lib"), None);
    assert_eq!(retarget_path("/repo", "/repo/src", "/repo/lib"), None);
}

#[test]
fn close_capture_completion_cannot_invert_user_close_order() {
    let mut runtime = runtime();
    let first = close_capture_request("first");
    let second = close_capture_request("second");
    runtime.ensure_pending_close_from_request(&first);
    runtime.close_capture_order = VecDeque::from([first.key.clone(), second.key.clone()]);

    let (_, early_effects) = runtime.ingest_close_capture_result(
        &second,
        Ok(live::CloseCaptureOutcome {
            item: Some(closed_file("second", "/repo/second.rs")),
        }),
    );
    assert_eq!(
        early_effects
            .iter()
            .map(|effect| effect.key.as_str())
            .collect::<Vec<_>>(),
        ["second"]
    );
    assert_eq!(runtime.snapshot().recent_closed.count, 0);
    assert_eq!(runtime.snapshot().recent_closed.pending.len(), 2);

    let (_, ordered_effects) = runtime.ingest_close_capture_result(
        &first,
        Ok(live::CloseCaptureOutcome {
            item: Some(closed_file("first", "/repo/first.rs")),
        }),
    );
    assert_eq!(
        ordered_effects
            .iter()
            .map(|effect| effect.key.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    assert_eq!(runtime.snapshot().recent_closed.count, 0);
    assert_eq!(runtime.snapshot().recent_closed.pending.len(), 2);

    runtime.ingest_close_effect_result(&early_effects[0], Ok(()));
    runtime.ingest_close_effect_result(&ordered_effects[0], Ok(()));
    assert!(runtime.mark_close_topology_confirmed("first"));
    assert!(runtime.promote_close_reservations());
    assert_eq!(runtime.snapshot().recent_closed.count, 1);
    assert_eq!(
        runtime.snapshot().recent_closed.top_label.as_deref(),
        Some("first.rs")
    );
    assert!(runtime.mark_close_topology_confirmed("second"));
    assert!(runtime.promote_close_reservations());
    assert_eq!(runtime.snapshot().recent_closed.count, 2);
    assert_eq!(
        runtime.snapshot().recent_closed.top_label.as_deref(),
        Some("second.rs")
    );
}

#[test]
fn rejected_close_removes_only_its_reserved_item() {
    let mut runtime = runtime();
    runtime.push_recent_closed(closed_file("first", "/repo/first.rs"));
    runtime.push_recent_closed(closed_file("second", "/repo/second.rs"));

    runtime.ingest_close_effect_result(
        &live::CloseEffectRequest {
            safety: None,
            failed_terminals: Vec::new(),
            retain_for_reopen: true,
            allow_replacement_create: true,
            replacement: None,
            key: "first".to_owned(),
            connection_generation: 0,
            target: live::CloseCaptureTarget::Tab {
                tab_id: "tab:first".to_owned(),
            },
        },
        Err(hide_herdr_client::ApiError::Remote {
            code: "refused".to_owned(),
            message: "close refused".to_owned(),
        }),
    );

    assert_eq!(runtime.snapshot().recent_closed.count, 1);
    assert_eq!(
        runtime.snapshot().recent_closed.top_label.as_deref(),
        Some("second.rs")
    );
    let notice = &runtime.snapshot().recent_closed.notices[0].message;
    assert!(notice.contains("not closed"));
    assert!(!notice.contains("replacement"));
}

#[test]
fn ambiguous_close_result_keeps_the_reserved_item_for_reconciliation() {
    for error in [
        hide_herdr_client::ApiError::Transport("acknowledgement timed out".to_owned()),
        hide_herdr_client::ApiError::Malformed("acknowledgement was invalid".to_owned()),
    ] {
        let mut runtime = runtime();
        runtime.push_recent_closed(closed_file("first", "/repo/first.rs"));
        runtime.ingest_close_effect_result(
            &live::CloseEffectRequest {
                safety: None,
                failed_terminals: Vec::new(),
                retain_for_reopen: true,
                allow_replacement_create: true,
                replacement: None,
                key: "first".to_owned(),
                connection_generation: 0,
                target: live::CloseCaptureTarget::Tab {
                    tab_id: "tab:first".to_owned(),
                },
            },
            Err(error),
        );

        assert_eq!(runtime.snapshot().recent_closed.count, 0);
        assert_eq!(runtime.snapshot().recent_closed.pending.len(), 1);
        assert_eq!(runtime.snapshot().recent_closed.pending[0].phase, "unknown");
        assert!(!runtime.snapshot().recent_closed.can_reopen);
        assert!(
            runtime.snapshot().recent_closed.notices[0]
                .message
                .contains("unknown")
        );
    }
}

/// B6, D-07, D-19: only user-closeable file tabs enter the unified stack;
/// diff tabs stay outside it and the shell receives the current count and
/// top label in the same snapshot.
#[test]
fn recent_closed_tracks_file_tabs_but_not_diff_tabs() {
    let mut runtime = runtime();
    for (id, kind) in [
        ("diff:one", EditorTabKind::Diff),
        ("file:one", EditorTabKind::File),
    ] {
        runtime.snapshot.editor.tabs.push(EditorTabSnapshot {
            id: id.to_owned(),
            workspace_id: "workspace:0".to_owned(),
            checkout_id: "checkout:0".to_owned(),
            path: format!("/repo/{id}.rs"),
            label: format!("{id}.rs"),
            kind,
            diff_committed: (kind == EditorTabKind::Diff).then_some(false),
            markdown_live: false,
            wrap: false,
            dirty: false,
            preview: false,
            unavailable_reason: None,
        });
    }
    assert!(runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": "diff:one"}),
    )));
    assert_eq!(runtime.snapshot().recent_closed.count, 0);

    assert!(runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": "file:one"}),
    )));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.recent_closed.count, 1);
    assert_eq!(
        snapshot.recent_closed.top_label.as_deref(),
        Some("file:one.rs")
    );
    assert!(!snapshot.recent_closed.restoring);
}

/// B15, B16, B23: an external-effect failure retains the same top item for
/// retry, while a definitively missing file consumes it and leaves an
/// explicit Trash recovery instruction.
#[test]
fn recent_closed_failure_retains_and_missing_file_consumes() {
    let mut runtime = runtime();
    let item = closed_file("reopen-fixture", "/repo/gone.rs");
    runtime.push_recent_closed(item.clone());
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item,
        workspace_exists: true,
        tab_exists: true,
        fallback_pane_id: None,
        owner: None,
    };

    runtime.reopen_in_flight = Some("reopen-fixture".to_owned());
    assert!(runtime.ingest_reopen_result(&request, Err("layout.apply timed out".to_owned()),));
    let retained = runtime.snapshot();
    assert_eq!(retained.recent_closed.count, 1);
    assert!(!retained.recent_closed.restoring);
    assert!(retained.recent_closed.notices[0].message.contains("retry"));

    runtime.reopen_in_flight = Some("reopen-fixture".to_owned());
    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::File(
            live::FileReopenResult::Missing,
        )),
    ));
    let consumed = runtime.snapshot();
    assert_eq!(consumed.recent_closed.count, 0);
    assert!(
        consumed.recent_closed.notices[0]
            .message
            .contains("Finder Trash")
    );
}

/// A completed reopen consumes the item that started the request. A newer
/// close remains the visible LIFO top while that older request finishes.
#[test]
fn reopen_completion_preserves_a_newer_close() {
    let mut runtime = runtime();
    let first = closed_file("first", "/repo/first.rs");
    runtime.push_recent_closed(first.clone());
    runtime.reopen_in_flight = Some("first".to_owned());
    runtime.push_recent_closed(closed_file("second", "/repo/second.rs"));
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item: first,
        workspace_exists: true,
        tab_exists: true,
        fallback_pane_id: None,
        owner: None,
    };

    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: None,
            consumed: true,
            focused_pane_id: None,
            notices: vec![],
            agents: Vec::new(),
        },)),
    ));

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.recent_closed.count, 1);
    assert_eq!(
        snapshot.recent_closed.top_label.as_deref(),
        Some("second.rs")
    );
    assert!(!snapshot.recent_closed.restoring);
}

#[test]
fn every_consuming_reopen_result_removes_only_its_request_key() {
    let assert_newer_close_survives = |result: live::FileReopenResultOrHerdr, first_path: &str| {
        let mut runtime = runtime();
        let first = closed_file("first", first_path);
        runtime.push_recent_closed(first.clone());
        runtime.reopen_in_flight = Some("first".to_owned());
        runtime.push_recent_closed(closed_file("second", "/repo/second.rs"));
        let request = live::ReopenRequest {
            codex_daemon: Default::default(),
            item: first,
            workspace_exists: true,
            tab_exists: true,
            fallback_pane_id: None,
            owner: None,
        };
        assert!(runtime.ingest_reopen_result(&request, Ok(result)));
        assert_eq!(runtime.snapshot().recent_closed.count, 1);
        assert_eq!(
            runtime.snapshot().recent_closed.top_label.as_deref(),
            Some("second.rs")
        );
    };

    assert_newer_close_survives(
        live::FileReopenResultOrHerdr::File(live::FileReopenResult::Missing),
        "/repo/missing.rs",
    );
    assert_newer_close_survives(
        live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: None,
            consumed: true,
            focused_pane_id: None,
            notices: vec![],
            agents: Vec::new(),
        }),
        "/repo/pane-placeholder.rs",
    );

    let folder = tempfile::Builder::new()
        .prefix("herdr-core-reopen-file-result-")
        .tempdir_in("/tmp")
        .unwrap();
    let root = folder.path().to_path_buf();
    let path = root.join("first.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let opened = crate::files::tests::open_local(&path);
    assert_newer_close_survives(
        live::FileReopenResultOrHerdr::File(live::FileReopenResult::Opened(Box::new(opened))),
        path.to_str().unwrap(),
    );
}

#[test]
fn completion_is_safe_after_the_in_flight_item_was_evicted() {
    let mut runtime = runtime();
    let first = closed_file("first", "/repo/first.rs");
    runtime.push_recent_closed(first.clone());
    runtime.reopen_in_flight = Some("first".to_owned());
    for index in 0..crate::recent_closed::RECENT_CLOSED_LIMIT {
        runtime.push_recent_closed(closed_file(
            &format!("newer-{index}"),
            &format!("/repo/newer-{index}.rs"),
        ));
    }
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item: first,
        workspace_exists: true,
        tab_exists: true,
        fallback_pane_id: None,
        owner: None,
    };

    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: None,
            consumed: true,
            focused_pane_id: None,
            notices: vec![],
            agents: Vec::new(),
        },)),
    ));

    let snapshot = runtime.snapshot();
    assert_eq!(
        snapshot.recent_closed.count,
        crate::recent_closed::RECENT_CLOSED_LIMIT
    );
    assert_eq!(
        snapshot.recent_closed.top_label.as_deref(),
        Some("newer-19.rs")
    );
}

/// B9, B18: a renamed tab keeps the id its old path gave it, so a new file
/// opened at that old path takes a free numbered id instead of being dropped
/// as already open; the renamed file itself still finds its own tab.
#[test]
fn a_new_file_at_a_renamed_tabs_old_path_gets_its_own_tab_id() {
    let (mut runtime, _root) = explorer_runtime();
    let base = Runtime::file_tab_id("w", "c", "/r/a.txt");
    assert_eq!(runtime.new_file_tab_id("w", "c", "/r/a.txt"), base);
    runtime.snapshot.editor.tabs.push(EditorTabSnapshot {
        id: base.clone(),
        workspace_id: "w".to_owned(),
        checkout_id: "c".to_owned(),
        path: "/r/b.txt".to_owned(),
        label: "b.txt".to_owned(),
        kind: EditorTabKind::File,
        diff_committed: None,
        markdown_live: false,
        wrap: false,
        dirty: false,
        preview: false,
        unavailable_reason: None,
    });
    assert_eq!(
        runtime.new_file_tab_id("w", "c", "/r/a.txt"),
        format!("{base}#2")
    );
    assert_eq!(
        runtime.new_file_tab_id("w", "c", "/r/b.txt"),
        Runtime::file_tab_id("w", "c", "/r/b.txt")
    );
}
