use super::*;

// The preview tab rules of PRD editor-preview-tab (D-01..D-05, D-09, D-10):
// what a single click, a double-click, an edit, Keep Open, a drag and a
// dirty tab do to the checkout's one preview slot.

fn open(runtime: &mut Runtime, checkout_id: &str, path: &Path, preview: bool) -> bool {
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": path.to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "preview": preview
        }),
    ))
}

fn tabs(runtime: &Runtime) -> Vec<(String, bool, bool)> {
    runtime
        .snapshot()
        .editor
        .tabs
        .iter()
        .map(|tab| (tab.label.clone(), tab.preview, tab.dirty))
        .collect()
}

fn strip_previews(runtime: &Runtime, checkout_id: &str) -> Vec<(String, bool)> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .expect("the registered checkout")
        .strip
        .iter()
        .map(|entry| (entry.label.clone(), entry.preview))
        .collect()
}

fn draft(runtime: &mut Runtime, contents: &str) -> bool {
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"contents_utf8": contents}),
    ))
}

/// B1, B2, B14, B20: a single click opens the preview tab at the end of
/// the strip; the next single click replaces it in the same slot, drops the
/// replaced document, records nothing for reopening, and moves the
/// Explorer selection.
#[test]
fn a_single_click_opens_one_preview_tab_and_the_next_replaces_it_in_place() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-replace");
    let tabs_ids = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs_ids,
        &tabs_ids,
        "w-order:t1"
    ))));
    let notes = directory.join("notes.md");
    let second = directory.join("second.md");
    let third = directory.join("third.md");
    std::fs::write(&second, "second\n").expect("fixture");
    std::fs::write(&third, "third\n").expect("fixture");

    assert!(open(&mut runtime, &checkout_id, &notes, true));
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), true, false)]);
    assert_eq!(
        strip_previews(&runtime, &checkout_id),
        vec![
            ("Tab 1".to_owned(), false),
            ("Tab 2".to_owned(), false),
            ("notes.md".to_owned(), true)
        ]
    );
    // An ordinary tab beside it, so the slot the preview holds is not the end.
    assert!(open(&mut runtime, &checkout_id, &second, false));
    assert_eq!(
        tabs(&runtime),
        vec![
            ("notes.md".to_owned(), true, false),
            ("second.md".to_owned(), false, false)
        ]
    );

    assert!(open(&mut runtime, &checkout_id, &third, true));
    let snapshot = runtime.snapshot();
    assert_eq!(
        tabs(&runtime),
        vec![
            ("third.md".to_owned(), true, false),
            ("second.md".to_owned(), false, false)
        ],
        "the new preview inherits the replaced tab's slot"
    );
    assert_eq!(
        strip_previews(&runtime, &checkout_id)[2..],
        [
            ("third.md".to_owned(), true),
            ("second.md".to_owned(), false)
        ]
    );
    let third_id = Runtime::file_tab_id("workspace:order", &checkout_id, &third.to_string_lossy());
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());
    assert_eq!(
        snapshot.editor.active_tab_id.as_deref(),
        Some(third_id.as_str())
    );
    assert_eq!(
        snapshot
            .editor
            .document
            .as_ref()
            .map(|document| document.path.as_str()),
        Some(third.to_string_lossy().as_ref())
    );
    assert_eq!(
        snapshot.ui_state.selected_path.as_deref(),
        Some(third.to_string_lossy().as_ref())
    );
    assert!(!runtime.editor_documents.contains_key(&notes_id));
    assert!(!runtime.editor_tab_history.contains(&notes_id));
    assert_eq!(
        snapshot.recent_closed.count, 0,
        "a replaced preview is not a close"
    );
    assert!(snapshot.status.last_error.is_none());

    std::fs::remove_dir_all(&directory).ok();
}

/// B4, D-10: opening the preview file as an ordinary tab promotes it where it
/// sits without reading the document again.
#[test]
fn an_ordinary_open_of_the_preview_file_promotes_it_in_place() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-promote-open");
    let notes = directory.join("notes.md");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());
    let before = runtime
        .editor_documents
        .get(&notes_id)
        .cloned()
        .expect("document");

    assert!(open(&mut runtime, &checkout_id, &notes, false));
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), false, false)]);
    assert_eq!(
        strip_previews(&runtime, &checkout_id),
        vec![("notes.md".to_owned(), false)]
    );
    assert_eq!(runtime.editor_documents.get(&notes_id), Some(&before));

    std::fs::remove_dir_all(&directory).ok();
}

/// B9, B10: a single click on a file that is already open focuses its tab
/// and neither creates nor replaces a preview.
#[test]
fn a_single_click_on_an_open_file_focuses_without_touching_the_preview_slot() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-focus-only");
    let notes = directory.join("notes.md");
    let second = directory.join("second.md");
    std::fs::write(&second, "second\n").expect("fixture");
    assert!(open(&mut runtime, &checkout_id, &notes, false));
    assert!(open(&mut runtime, &checkout_id, &second, true));
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());
    let second_id =
        Runtime::file_tab_id("workspace:order", &checkout_id, &second.to_string_lossy());

    assert!(open(&mut runtime, &checkout_id, &notes, true));
    assert_eq!(
        runtime.snapshot().editor.active_tab_id.as_deref(),
        Some(notes_id.as_str())
    );
    assert_eq!(
        tabs(&runtime),
        vec![
            ("notes.md".to_owned(), false, false),
            ("second.md".to_owned(), true, false)
        ]
    );

    assert!(open(&mut runtime, &checkout_id, &second, true));
    assert_eq!(
        runtime.snapshot().editor.active_tab_id.as_deref(),
        Some(second_id.as_str())
    );
    assert_eq!(
        tabs(&runtime),
        vec![
            ("notes.md".to_owned(), false, false),
            ("second.md".to_owned(), true, false)
        ]
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// B5: the first edit promotes the preview tab in that frame, and the next
/// single click opens a new preview beside it. An echo of the same contents
/// is not an edit.
#[test]
fn the_first_edit_keeps_a_preview_tab_and_the_next_click_opens_beside_it() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-edit");
    let notes = directory.join("notes.md");
    let second = directory.join("second.md");
    std::fs::write(&second, "second\n").expect("fixture");
    assert!(open(&mut runtime, &checkout_id, &notes, true));

    assert!(draft(&mut runtime, "notes\n"));
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), true, false)]);

    assert!(draft(&mut runtime, "notes edited\n"));
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), false, true)]);

    assert!(open(&mut runtime, &checkout_id, &second, true));
    assert_eq!(
        tabs(&runtime),
        vec![
            ("notes.md".to_owned(), false, true),
            ("second.md".to_owned(), true, false)
        ]
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// B8, D-05: however a preview tab came to be dirty, the next single click
/// keeps it, promotes it, and opens the new preview beside it; the draft
/// survives.
#[test]
fn a_dirty_preview_tab_is_never_replaced() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-dirty");
    let notes = directory.join("notes.md");
    let second = directory.join("second.md");
    std::fs::write(&second, "second\n").expect("fixture");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    assert!(draft(&mut runtime, "notes edited\n"));
    // The shell debounces nothing here, but the core does not rely on the
    // edit having promoted the tab: a dirty preview is refused replacement
    // on its own.
    runtime.snapshot.editor.tabs[0].preview = true;
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());

    assert!(open(&mut runtime, &checkout_id, &second, true));
    assert_eq!(
        tabs(&runtime),
        vec![
            ("notes.md".to_owned(), false, true),
            ("second.md".to_owned(), true, false)
        ]
    );
    assert_eq!(
        runtime
            .editor_documents
            .get(&notes_id)
            .and_then(|document| document.contents_utf8.as_deref()),
        Some("notes edited\n")
    );
    assert_eq!(runtime.snapshot().recent_closed.count, 0);

    std::fs::remove_dir_all(&directory).ok();
}

/// B6: Keep Open promotes the preview tab in its slot, changes nothing on an
/// ordinary tab, and refuses a tab that is not open.
#[test]
fn keep_open_promotes_a_preview_tab_once() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-keep-open");
    let notes = directory.join("notes.md");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());
    let keep = |runtime: &mut Runtime, tab_id: &str| {
        runtime.dispatch_json(&explorer_event(
            "file_keep_open",
            serde_json::json!({"tab_id": tab_id}),
        ))
    };

    assert!(keep(&mut runtime, &notes_id));
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), false, false)]);
    assert_eq!(
        strip_previews(&runtime, &checkout_id),
        vec![("notes.md".to_owned(), false)]
    );

    assert!(
        !keep(&mut runtime, &notes_id),
        "an ordinary tab has nothing to keep"
    );
    assert!(runtime.snapshot().status.last_error.is_none());

    assert!(keep(&mut runtime, "file:missing"));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("editor.keep_open_unknown_tab")
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// B7: a preview tab the operator drags to a new slot becomes an ordinary
/// tab there.
#[test]
fn dragging_a_preview_tab_promotes_it() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-drag");
    let tabs_ids = ["w-order:t1", "w-order:t2"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs_ids,
        &tabs_ids,
        "w-order:t1"
    ))));
    let notes = directory.join("notes.md");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    let entry = strip_ids(&runtime, &checkout_id)[2].clone();

    assert!(reorder_tab(&mut runtime, &checkout_id, &entry, 1));
    assert_eq!(
        strip_previews(&runtime, &checkout_id),
        vec![
            ("Tab 1".to_owned(), false),
            ("notes.md".to_owned(), false),
            ("Tab 2".to_owned(), false)
        ]
    );
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), false, false)]);

    std::fs::remove_dir_all(&directory).ok();
}

/// B12, D-02: a Changes single click opens a preview diff in the same slot a
/// preview file tab holds, and a file click takes the slot back from the
/// diff, clearing the Changes selection the diff was showing.
#[test]
fn file_and_diff_previews_share_one_slot() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-diff");
    let notes = directory.join("notes.md");
    let second = directory.join("second.md");
    std::fs::write(&second, "second\n").expect("fixture");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    runtime.snapshot.changes.edit().root_path = Some(directory.to_string_lossy().into_owned());
    runtime.snapshot.changes.edit().entries = vec![crate::model::ChangedFileSnapshot {
        path: second.to_string_lossy().into_owned(),
        relative_path: "second.md".to_owned(),
        previous_relative_path: None,
        status: crate::model::ChangedFileStatus::Untracked,
        added_lines: Some(1),
        removed_lines: Some(0),
    }];

    assert!(runtime.dispatch_json(&explorer_event(
        "changes_select",
        serde_json::json!({"path": second, "committed": false, "preview": true}),
    )));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.editor.tabs.len(), 1);
    assert_eq!(snapshot.editor.tabs[0].kind, EditorTabKind::Diff);
    assert!(snapshot.editor.tabs[0].preview);
    assert_eq!(
        snapshot.changes.selected_path.as_deref(),
        Some(second.to_string_lossy().as_ref())
    );
    assert!(strip_previews(&runtime, &checkout_id)[0].1);

    assert!(open(&mut runtime, &checkout_id, &notes, true));
    let snapshot = runtime.snapshot();
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), true, false)]);
    assert!(snapshot.changes.selected_path.is_none());
    assert!(snapshot.changes.diff.is_none());
    assert_eq!(snapshot.recent_closed.count, 0);

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn double_clicking_the_active_diff_keeps_it_open_in_the_same_slot() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-diff-promote");
    let changed = directory.join("second.md");
    runtime.snapshot.changes.edit().root_path = Some(directory.to_string_lossy().into_owned());
    runtime.snapshot.changes.edit().entries = vec![crate::model::ChangedFileSnapshot {
        path: changed.to_string_lossy().into_owned(),
        relative_path: "second.md".to_owned(),
        previous_relative_path: None,
        status: crate::model::ChangedFileStatus::Deleted,
        added_lines: None,
        removed_lines: Some(1),
    }];
    let select = |preview| {
        explorer_event(
            "changes_select",
            serde_json::json!({"path": changed, "committed": false, "preview": preview}),
        )
    };
    assert!(runtime.dispatch_json(&select(true)));
    assert!(runtime.snapshot().editor.tabs[0].preview);
    assert!(runtime.dispatch_json(&select(false)));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.editor.tabs.len(), 1);
    assert_eq!(snapshot.editor.tabs[0].checkout_id, checkout_id);
    assert!(!snapshot.editor.tabs[0].preview);
    assert!(!strip_previews(&runtime, &checkout_id)[0].1);
    std::fs::remove_dir_all(&directory).ok();
}

/// B3, D-02: each checkout has its own preview slot.
#[test]
fn preview_tabs_are_per_checkout() {
    let (mut runtime, first, first_id, second, second_id) = reveal_runtime();
    let first_file = first.join("deep/nested/leaf/target.txt");
    let second_file = second.join("deep/nested/leaf/target.txt");
    runtime.open_file_tab(
        "workspace:0",
        &first_id,
        &first_file.to_string_lossy(),
        true,
    );
    runtime.open_file_tab(
        "workspace:1",
        &second_id,
        &second_file.to_string_lossy(),
        true,
    );

    let previews = runtime
        .snapshot()
        .editor
        .tabs
        .iter()
        .map(|tab| (tab.checkout_id.clone(), tab.preview))
        .collect::<Vec<_>>();
    assert_eq!(
        previews,
        vec![(first_id.clone(), true), (second_id.clone(), true)]
    );

    std::fs::remove_dir_all(&first).ok();
    std::fs::remove_dir_all(&second).ok();
}

/// B13, B15: closing a preview tab records it for reopening like any file
/// tab, and a click on a file that cannot be read leaves the preview tab as
/// it was.
#[test]
fn a_preview_tab_closes_normally_and_survives_a_failed_open() {
    let (mut runtime, checkout_id, directory) = strip_checkout("preview-close-fail");
    let notes = directory.join("notes.md");
    assert!(open(&mut runtime, &checkout_id, &notes, true));
    let notes_id = Runtime::file_tab_id("workspace:order", &checkout_id, &notes.to_string_lossy());

    assert!(open(
        &mut runtime,
        &checkout_id,
        &directory.join("missing.md"),
        true
    ));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("file.open_failed")
    );
    assert_eq!(tabs(&runtime), vec![("notes.md".to_owned(), true, false)]);
    assert_eq!(
        runtime.snapshot().editor.active_tab_id.as_deref(),
        Some(notes_id.as_str())
    );

    assert!(runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": notes_id}),
    )));
    let snapshot = runtime.snapshot();
    assert!(snapshot.editor.tabs.is_empty());
    assert_eq!(snapshot.recent_closed.count, 1);
    assert_eq!(
        snapshot.recent_closed.top_label.as_deref(),
        Some("notes.md")
    );

    std::fs::remove_dir_all(&directory).ok();
}

/// D-09: a revealed path and a reopened tab are ordinary tabs.
#[test]
fn reveal_and_reopen_open_ordinary_tabs() {
    let (mut runtime, root, checkout_id, _second, _second_id) = reveal_runtime();
    let target = root.join("deep/nested/leaf/target.txt");
    assert!(runtime.dispatch_json(&reveal_event("workspace:0", &checkout_id, &target, false)));
    assert_eq!(
        runtime
            .snapshot()
            .editor
            .tabs
            .iter()
            .map(|tab| tab.preview)
            .collect::<Vec<_>>(),
        vec![false]
    );

    let tab_id = Runtime::file_tab_id("workspace:0", &checkout_id, &target.to_string_lossy());
    assert!(runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": tab_id}),
    )));
    let item = runtime
        .recent_closed
        .back()
        .cloned()
        .expect("the closed file");
    let request = crate::recent_closed::reopen::ReopenRequest {
        codex_daemon: Default::default(),
        permissions: Default::default(),
        item,
        workspace_exists: true,
        tab_exists: true,
        fallback_pane_id: None,
        owner: None,
    };
    runtime.reopen_in_flight = Some(request.item.key().to_owned());
    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(crate::recent_closed::reopen::FileReopenResultOrHerdr::File(
            crate::recent_closed::reopen::FileReopenResult::Opened(Box::new(
                files::tests::open_local(&target)
            ))
        )),
    ));
    assert_eq!(
        runtime
            .snapshot()
            .editor
            .tabs
            .iter()
            .map(|tab| tab.preview)
            .collect::<Vec<_>>(),
        vec![false]
    );

    std::fs::remove_dir_all(&root).ok();
}
