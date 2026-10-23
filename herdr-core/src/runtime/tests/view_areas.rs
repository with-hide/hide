use super::workspace_view::{
    active_label, layout, second_checkout, views_path, with_new_views, with_views,
};
use super::*;
use crate::model::{ViewDisplaySnapshot, ViewDisplayState, ViewLayoutSnapshot, ViewNodeSnapshot};
use crate::workspace_control::Action;
use crate::workspace_views::Tool;

// PRD S7: a Workspace's View areas hold displays of documents; where an open
// lands, what the operator's layout actions do, how a display follows its
// document, and what comes back after a restart. Each test reads the tree the
// snapshot publishes.

fn open(runtime: &mut Runtime, checkout_id: &str, path: &Path, preview: bool, beside: bool) {
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": path.to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "preview": preview,
            "beside": beside,
        }),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// Sends a `view_layout` action as the web does, named for the Workspace of
/// the frame it was taken on.
fn act(runtime: &mut Runtime, payload: serde_json::Value) -> bool {
    runtime.sync_workspace_view();
    let view = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("a front Workspace");
    let workspace = serde_json::json!({"device_id": view.device_id, "path": view.path});
    act_on(runtime, workspace, payload)
}

fn act_on(
    runtime: &mut Runtime,
    workspace: serde_json::Value,
    mut payload: serde_json::Value,
) -> bool {
    payload["workspace"] = workspace;
    runtime.dispatch_json(&explorer_event("view_layout", payload))
}

fn tree(runtime: &mut Runtime) -> ViewLayoutSnapshot {
    runtime.sync_workspace_view();
    runtime
        .snapshot
        .workspace_view
        .clone()
        .expect("a front Workspace")
        .layout
}

/// Every area in tree order with its displays.
fn areas(layout: &ViewLayoutSnapshot) -> Vec<(String, Option<String>, Vec<ViewDisplaySnapshot>)> {
    let mut found = Vec::new();
    let mut nodes = vec![&layout.root];
    while let Some(node) = nodes.pop() {
        match node {
            ViewNodeSnapshot::Area(area) => {
                found.push((area.id.clone(), area.active.clone(), area.displays.clone()))
            }
            ViewNodeSnapshot::Split(split) => nodes.extend([&*split.second, &*split.first]),
        }
    }
    found
}

/// Each area's display labels, a preview marked with `*`.
fn labels(runtime: &mut Runtime) -> Vec<Vec<String>> {
    areas(&tree(runtime))
        .into_iter()
        .map(|(_, _, displays)| {
            displays
                .iter()
                .map(|display| {
                    format!(
                        "{}{}",
                        display.label,
                        if display.preview { "*" } else { "" }
                    )
                })
                .collect()
        })
        .collect()
}

/// The id of the display labelled `label` in area `index`.
fn display(runtime: &mut Runtime, index: usize, label: &str) -> String {
    areas(&tree(runtime))[index]
        .2
        .iter()
        .find(|display| display.label == label)
        .unwrap_or_else(|| panic!("{label} in area {index}"))
        .id
        .clone()
}

fn area_id(runtime: &mut Runtime, index: usize) -> String {
    areas(&tree(runtime))[index].0.clone()
}

fn split(runtime: &mut Runtime, display_id: &str, area_id: &str, request_id: &str) -> bool {
    act(
        runtime,
        serde_json::json!({
            "action": "split", "display_id": display_id, "area_id": area_id,
            "edge": "right", "request_id": request_id,
        }),
    )
}

fn draft(runtime: &mut Runtime, tab_id: &str, contents: &str) {
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"tab_id": tab_id, "contents_utf8": contents}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

fn tab_of(runtime: &Runtime, label: &str) -> String {
    runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.label == label)
        .unwrap_or_else(|| panic!("{label} is open"))
        .id
        .clone()
}

fn files(directory: &Path, names: &[&str]) {
    for name in names {
        std::fs::write(directory.join(name), format!("{name}\n")).expect("fixture");
    }
}

fn views_runtime(name: &str) -> (Runtime, String, PathBuf) {
    let (runtime, checkout_id, directory) = strip_checkout(name);
    let runtime = with_new_views(runtime, name);
    (runtime, checkout_id, directory)
}

/// B1: an open lands in the area the operator used last.
#[test]
fn an_open_lands_in_the_view_area_used_last() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-last-used");
    files(&directory, &["a.md", "b.md", "c.md", "d.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        false,
    );
    let (b, first) = (display(&mut runtime, 0, "b.md"), area_id(&mut runtime, 0));
    assert!(split(&mut runtime, &b, &first, "split-b"));

    let a = display(&mut runtime, 0, "a.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("c.md"),
        false,
        false,
    );
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": b}),
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("d.md"),
        false,
        false,
    );

    assert_eq!(
        labels(&mut runtime),
        vec![vec!["a.md", "c.md"], vec!["b.md", "d.md"]]
    );
    let layout = tree(&mut runtime);
    assert_eq!(layout.active_area, area_id(&mut runtime, 1));
    assert_eq!(active_label(&runtime).as_deref(), Some("d.md"));
}

/// B2, D-02: a preview open takes the area's preview display in place; the
/// document it showed goes, with no Recent Closed entry, only when no other
/// display shows it.
#[test]
fn a_preview_open_takes_the_preview_display_and_retires_an_undisplayed_document() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-preview");
    files(&directory, &["x.md", "y.md", "z.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("x.md"),
        true,
        false,
    );
    let slot = display(&mut runtime, 0, "x.md");
    // x also shows in a new area to the right.
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("x.md"),
        false,
        true,
    );
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": slot}),
    );

    open(
        &mut runtime,
        &checkout_id,
        &directory.join("y.md"),
        true,
        false,
    );
    assert_eq!(
        labels(&mut runtime),
        vec![vec!["notes.md", "y.md*"], vec!["x.md"]]
    );
    assert_eq!(
        display(&mut runtime, 0, "y.md"),
        slot,
        "retargeted in place"
    );
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.label == "x.md"),
        "x still shows to the right"
    );

    open(
        &mut runtime,
        &checkout_id,
        &directory.join("z.md"),
        true,
        false,
    );
    assert_eq!(display(&mut runtime, 0, "z.md"), slot);
    assert!(
        !runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.label == "y.md"),
        "y showed nowhere else, so it went"
    );
    assert_eq!(runtime.snapshot.recent_closed.count, 0);
}

/// B3: a second single click on a file already shown focuses the display
/// that showed it last, wherever that is, and adds nothing.
#[test]
fn a_second_single_click_focuses_the_display_that_showed_the_file_last() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-refocus");
    files(&directory, &["x.md", "w.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("x.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("w.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("x.md"),
        false,
        true,
    );
    let beside = display(&mut runtime, 1, "x.md");
    let w = display(&mut runtime, 0, "w.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": w}),
    );

    open(
        &mut runtime,
        &checkout_id,
        &directory.join("x.md"),
        true,
        false,
    );

    let layout = tree(&mut runtime);
    assert_eq!(layout.display_count, 3);
    let right = &areas(&layout)[1];
    assert_eq!(layout.active_area, right.0);
    assert_eq!(right.1.as_deref(), Some(beside.as_str()));
}

/// B4: Open to the side uses the area next to the one in use, and makes a
/// new area to the right only when there is none.
#[test]
fn open_to_the_side_uses_the_neighbour_or_a_new_area_to_the_right() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-beside");
    files(&directory, &["a.md", "b.md", "c.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        true,
    );
    let layout = tree(&mut runtime);
    let ViewNodeSnapshot::Split(split) = &layout.root else {
        panic!("a new area beside");
    };
    assert_eq!(split.axis, crate::view_layout::SplitAxis::Row);

    let a = display(&mut runtime, 0, "a.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("c.md"),
        false,
        true,
    );

    assert_eq!(
        labels(&mut runtime),
        vec![vec!["a.md"], vec!["b.md", "c.md"]]
    );
    assert_eq!(active_label(&runtime).as_deref(), Some("c.md"));
}

/// B18, B19, contract 2: a split past the area cap and an open past the
/// checkout's view cap are refused with a reason, and the tree stays as it was.
#[test]
fn a_split_or_an_open_past_the_caps_is_refused_and_changes_nothing() {
    use crate::view_layout::{DisplayKind, Edge, Layout, MAX_VIEW_DISPLAYS};
    let (mut runtime, checkout_id, directory) = views_runtime("view-caps");
    let root = directory.to_string_lossy().into_owned();
    // Six areas, three splits deep, holding the display cap between them.
    let mut full = Layout::default();
    for index in 0..MAX_VIEW_DISPLAYS - 5 {
        let shown = full.new_display(
            &format!("{root}/f{index}.md"),
            DisplayKind::File,
            None,
            false,
        );
        full.insert("a1", shown, 1).unwrap();
    }
    let beside = |layout: &mut Layout, area: &str, edge: Edge, name: &str| {
        let shown = layout.new_display(&format!("{root}/{name}"), DisplayKind::File, None, false);
        layout.split_new(area, edge, shown, 1).unwrap()
    };
    let right = beside(&mut full, "a1", Edge::Right, "r.md");
    let below = beside(&mut full, "a1", Edge::Down, "d.md");
    beside(&mut full, &right, Edge::Down, "rd.md");
    beside(&mut full, "a1", Edge::Right, "ar.md");
    beside(&mut full, &below, Edge::Right, "dr.md");
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &root)
        .layout = full;
    let before = tree(&mut runtime);
    assert_eq!(before.display_count, MAX_VIEW_DISPLAYS);
    let tabs = runtime.snapshot.editor.tabs.clone();

    let first = areas(&before)[0].2[0].id.clone();
    split(&mut runtime, &first, "a1", "past-the-cap");
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("view_layout.limit")
    );
    assert_eq!(tree(&mut runtime), before);

    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": directory.join("notes.md"), "workspace_id": "workspace:order",
            "checkout_id": checkout_id, "preview": false,
        }),
    ));
    let refused = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(refused.kind, "view_layout.display_limit");
    assert_eq!(
        refused.message,
        "This Workspace has 32 views open. Close a view to open another."
    );
    assert_eq!(tree(&mut runtime), before);
    assert_eq!(runtime.snapshot.editor.tabs, tabs, "nothing was read");
}

/// `count` views of files that are not open, in the front Workspace's one
/// area, as a restored Workspace holds them before they are read.
fn fill(runtime: &mut Runtime, directory: &Path, count: usize) {
    use crate::view_layout::{DisplayKind, Layout};
    let mut layout = Layout::default();
    for index in 0..count {
        let path = directory.join(format!("f{index}.md"));
        let shown = layout.new_display(&path.to_string_lossy(), DisplayKind::File, None, false);
        layout.insert("a1", shown, 1).unwrap();
    }
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
        .layout = layout;
}

/// B19: at its 32 views nothing adds a 33rd to the checkout when none of them
/// can close. Reopen Closed, a created file and a revealed file are refused
/// with the reason before anything is read or made, and the closed file
/// stays reopenable.
#[test]
fn at_the_display_cap_a_reopen_a_created_file_or_a_reveal_is_refused() {
    use crate::view_layout::MAX_VIEW_DISPLAYS;
    let (mut runtime, checkout_id, directory) = views_runtime("view-cap-origins");
    files(&directory, &["closed.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("closed.md"),
        false,
        false,
    );
    let closed = tab_of(&runtime, "closed.md");
    runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": closed}),
    ));
    assert_eq!(runtime.snapshot.recent_closed.count, 1);
    fill(&mut runtime, &directory, MAX_VIEW_DISPLAYS);
    let before = tree(&mut runtime);
    assert_eq!(before.display_count, MAX_VIEW_DISPLAYS);
    let tabs = runtime.snapshot.editor.tabs.clone();

    for (kind, payload) in [
        ("reopen_closed", serde_json::json!({})),
        (
            "file_create",
            serde_json::json!({"root": directory, "parent": directory, "name": "new.md"}),
        ),
        (
            "reveal_path",
            serde_json::json!({
                "path": directory.join("notes.md"), "workspace_id": "workspace:order",
                "checkout_id": checkout_id, "is_directory": false,
            }),
        ),
    ] {
        runtime.dispatch_json(&explorer_event(kind, payload));
        let error = runtime
            .snapshot
            .status
            .last_error
            .clone()
            .unwrap_or_else(|| panic!("{kind} at the cap is refused"));
        assert_eq!(error.kind, "view_layout.display_limit", "{kind}");
    }
    assert_eq!(tree(&mut runtime), before);
    assert_eq!(runtime.snapshot.editor.tabs, tabs, "nothing was read");
    assert!(!directory.join("new.md").exists(), "nothing was made");
    assert_eq!(
        runtime.snapshot.recent_closed.count, 1,
        "the closed file stays reopenable"
    );
}

/// B19: a file that lands after the checkout filled, here a Reopen Closed
/// admitted at 31 views, is refused with the reason as it lands rather than
/// making a 33rd view; nothing is opened and the file stays reopenable.
#[test]
fn a_reopen_that_lands_after_the_views_filled_is_refused_and_stays_reopenable() {
    use crate::view_layout::{DisplayKind, MAX_VIEW_DISPLAYS};
    let (runtime, checkout_id, directory) = views_runtime("view-cap-landing");
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    open(
        &mut shared.lock().unwrap(),
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    wait(&shared, "the file to be read", |runtime| {
        !runtime.editor_documents.is_empty()
    });
    {
        let mut runtime = shared.lock().unwrap();
        let notes = tab_of(&runtime, "notes.md");
        runtime.dispatch_json(&explorer_event(
            "file_close",
            serde_json::json!({"tab_id": notes}),
        ));
        fill(&mut runtime, &directory, MAX_VIEW_DISPLAYS - 1);
        runtime.dispatch_json(&explorer_event("reopen_closed", serde_json::json!({})));
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert!(runtime.snapshot.recent_closed.restoring);
        // Another view takes the last place while the file is read.
        let layout = &mut runtime
            .workspace_views
            .as_mut()
            .unwrap()
            .views
            .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
            .layout;
        let last = layout.new_display(
            &directory.join("last.md").to_string_lossy(),
            DisplayKind::File,
            None,
            false,
        );
        layout.insert("a1", last, 2).unwrap();
    }
    wait(&shared, "the reopen to land", |runtime| {
        !runtime.snapshot.recent_closed.restoring
    });
    let mut runtime = shared.lock().unwrap();
    let refused = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(refused.kind, "view_layout.display_limit");
    assert_eq!(tree(&mut runtime).display_count, MAX_VIEW_DISPLAYS);
    assert!(
        runtime.snapshot.editor.tabs.is_empty(),
        "nothing was opened"
    );
    assert_eq!(runtime.snapshot.recent_closed.count, 1, "still reopenable");
    assert!(
        runtime.snapshot.recent_closed.notices[0]
            .message
            .contains(&refused.message)
    );
}

/// B19: the reconcile holds the cap too. A document of the Workspace
/// that no view shows while the tab has 16, which no action makes now but a
/// future path might, waits off screen, is reported once in the diagnostic
/// log, and shows once a view is closed.
#[test]
fn a_document_the_full_views_cannot_hold_shows_once_a_view_is_closed() {
    use crate::view_layout::MAX_VIEW_DISPLAYS;
    let (mut runtime, checkout_id, directory) = views_runtime("view-cap-reconcile");
    fill(&mut runtime, &directory, MAX_VIEW_DISPLAYS);
    let path = directory.join("notes.md").to_string_lossy().into_owned();
    runtime.insert_diff_tab("workspace:order", &checkout_id, &path, false, false);
    let shown = |runtime: &mut Runtime| {
        labels(runtime)
            .concat()
            .contains(&"notes.md (working diff)".to_owned())
    };
    let reported = |runtime: &Runtime| {
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .filter(|logged| logged.kind == "view_layout.display_limit")
            .count()
    };

    let full = tree(&mut runtime);
    assert_eq!(full.display_count, MAX_VIEW_DISPLAYS);
    assert!(!shown(&mut runtime));
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert_eq!(reported(&runtime), 1, "reported once, not on every pass");

    let first = areas(&full)[0].2[0].id.clone();
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": first}),
    ));
    assert!(shown(&mut runtime));
    assert_eq!(tree(&mut runtime).display_count, MAX_VIEW_DISPLAYS);
}

/// Engineering principle 11: a split sent twice splits once.
#[test]
fn a_repeated_split_request_splits_once() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-split-twice");
    files(&directory, &["a.md", "b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        false,
    );
    let (b, first) = (display(&mut runtime, 0, "b.md"), area_id(&mut runtime, 0));
    assert!(split(&mut runtime, &b, &first, "split-1"));
    let once = tree(&mut runtime);

    assert!(!split(&mut runtime, &b, &first, "split-1"));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(tree(&mut runtime), once);
    assert_eq!(labels(&mut runtime), vec![vec!["a.md"], vec!["b.md"]]);
}

/// B2: Keep Open, from the view's own action or from the file's, pins a
/// preview view, so the next single click opens a new preview beside it
/// rather than replacing it.
#[test]
fn keep_open_pins_a_preview_view_so_the_next_click_opens_beside_it() {
    for entry in ["view_layout", "file_keep_open"] {
        let name = format!("view-keep-open-{entry}");
        let (mut runtime, checkout_id, directory) = views_runtime(&name);
        files(&directory, &["x.md", "y.md"]);
        open(
            &mut runtime,
            &checkout_id,
            &directory.join("x.md"),
            true,
            false,
        );
        assert_eq!(labels(&mut runtime), vec![vec!["x.md*"]]);

        if entry == "view_layout" {
            let x = display(&mut runtime, 0, "x.md");
            act(
                &mut runtime,
                serde_json::json!({"action": "keep_open", "display_id": x}),
            );
        } else {
            let tab_id = tab_of(&runtime, "x.md");
            runtime.dispatch_json(&explorer_event(
                "file_keep_open",
                serde_json::json!({"tab_id": tab_id}),
            ));
        }
        assert_eq!(runtime.snapshot.status.last_error, None, "{entry}");
        assert_eq!(labels(&mut runtime), vec![vec!["x.md"]], "{entry}");

        open(
            &mut runtime,
            &checkout_id,
            &directory.join("y.md"),
            true,
            false,
        );
        assert_eq!(labels(&mut runtime), vec![vec!["x.md", "y.md*"]], "{entry}");
    }
}

/// Contract 4.1: `focus_area` moves the area in use, as keyboard focus
/// between areas does, and an id the Workspace does not have is refused
/// with its reason and changes nothing.
#[test]
fn focus_area_moves_the_area_in_use_and_an_unknown_id_changes_nothing() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-focus-area");
    files(&directory, &["a.md", "b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        true,
    );
    let first = area_id(&mut runtime, 0);
    assert_ne!(tree(&mut runtime).active_area, first);

    act(
        &mut runtime,
        serde_json::json!({"action": "focus_area", "area_id": first}),
    );
    assert_eq!(tree(&mut runtime).active_area, first);
    assert_eq!(active_label(&runtime).as_deref(), Some("a.md"));

    let before = tree(&mut runtime);
    for (payload, kind) in [
        (
            serde_json::json!({"action": "focus", "display_id": "d404"}),
            "view_layout.unknown_display",
        ),
        (
            serde_json::json!({"action": "focus_area", "area_id": "a404"}),
            "view_layout.unknown_area",
        ),
        (
            serde_json::json!({"action": "resize", "split_id": "s404", "ratio": 0.3}),
            "view_layout.unknown_split",
        ),
    ] {
        act(&mut runtime, payload);
        let refused = runtime.snapshot.status.last_error.clone().expect(kind);
        assert_eq!(refused.kind, kind);
        assert_eq!(tree(&mut runtime), before, "{kind}");
    }
}

/// B9: moving an area's last display away closes the area, and its
/// neighbour takes the space.
#[test]
fn moving_an_areas_last_display_away_collapses_the_area() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-move");
    files(&directory, &["a.md", "b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        false,
    );
    let (b, first) = (display(&mut runtime, 0, "b.md"), area_id(&mut runtime, 0));
    split(&mut runtime, &b, &first, "split-b");

    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "move", "display_id": b, "area_id": first, "index": 0}),
    ));

    let layout = tree(&mut runtime);
    assert!(matches!(layout.root, ViewNodeSnapshot::Area(_)));
    assert_eq!(labels(&mut runtime), vec![vec!["b.md", "a.md"]]);
    assert_eq!(active_label(&runtime).as_deref(), Some("b.md"));
}

/// Contract 4.1: an action names the Workspace of the frame it
/// was taken on. Ids like `d2` repeat across Workspaces, so a close or move
/// that arrives after the front moved on changes neither tree and closes no
/// document, least of all the other Workspace's dirty one; the same action
/// for the Workspace in front applies.
#[test]
fn an_action_for_a_workspace_no_longer_in_front_changes_nothing() {
    let (mut runtime, checkout_id, directory) = strip_checkout("view-stale-workspace");
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, "view-stale-workspace");
    files(&other, &["b.md", "c.md"]);
    let identity = |runtime: &mut Runtime| {
        runtime.sync_workspace_view();
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        serde_json::json!({"device_id": view.device_id, "path": view.path})
    };
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let notes = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &notes, "draft\n");
    let (first, dirty_display) = (identity(&mut runtime), display(&mut runtime, 0, "notes.md"));
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:other", "checkout_id": other_checkout}),
    ));
    for name in ["b.md", "c.md"] {
        runtime.dispatch_json(&explorer_event(
            "file_open",
            serde_json::json!({
                "path": other.join(name).to_string_lossy(),
                "workspace_id": "workspace:other",
                "checkout_id": other_checkout,
                "preview": false,
            }),
        ));
    }
    let front = identity(&mut runtime);
    let shown = display(&mut runtime, 0, "b.md");
    assert_eq!(shown, dirty_display, "both Workspaces hold the same id");
    let area = area_id(&mut runtime, 0);
    let open_tabs = |runtime: &Runtime| {
        let mut tabs: Vec<_> = runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .map(|tab| (tab.label.clone(), tab.dirty))
            .collect();
        tabs.sort();
        tabs
    };
    let before = open_tabs(&runtime);

    let moved =
        serde_json::json!({"action": "move", "display_id": shown, "area_id": area, "index": 1});
    let closed = serde_json::json!({"action": "close", "display_id": shown});
    for action in [&moved, &closed] {
        assert!(act_on(&mut runtime, first.clone(), action.clone()));
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert_eq!(labels(&mut runtime), vec![vec!["b.md", "c.md"]]);
        assert_eq!(open_tabs(&runtime), before);
        let logged = runtime.snapshot.status.diagnostics.last().unwrap();
        assert_eq!(logged.kind, "view_layout.stale_workspace");
        assert!(
            logged
                .message
                .contains(&directory.to_string_lossy().into_owned())
                && logged
                    .message
                    .contains(&other.to_string_lossy().into_owned()),
            "{}",
            logged.message
        );
    }
    assert_eq!(
        before,
        vec![
            ("b.md".to_owned(), false),
            ("c.md".to_owned(), false),
            ("notes.md".to_owned(), true)
        ]
    );

    act_on(&mut runtime, front.clone(), moved);
    assert_eq!(labels(&mut runtime), vec![vec!["c.md", "b.md"]]);
    act_on(&mut runtime, front, closed);
    assert_eq!(labels(&mut runtime), vec![vec!["c.md"]]);
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:order", "checkout_id": checkout_id}),
    ));
    assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == notes && tab.dirty)
    );
}

/// B10: closing one of two displays of a document closes only that display;
/// the document and its unsaved draft stay.
#[test]
fn closing_one_of_two_displays_keeps_the_document_and_its_draft() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-close-one");
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        true,
    );
    let tab_id = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &tab_id, "draft\n");

    let beside = display(&mut runtime, 1, "notes.md");
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": beside}),
    ));

    assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
    let tab = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .expect("the document stays open");
    assert!(tab.dirty);
    let documents = runtime.snapshot_delta_payload(0).documents.unwrap();
    assert_eq!(
        documents.changed[0].1.contents_utf8.as_deref(),
        Some("draft\n")
    );
    // A close of the display already gone is a converged no-op.
    assert!(!act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": beside}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// A pane command has no Save or Don't Save permission. The second close
/// must leave the draft and its remaining View in place.
#[test]
fn pane_close_preserves_a_dirty_last_view_after_closing_its_twin() {
    let (mut runtime, checkout_id, directory) = views_runtime("pane-close-unsaved");
    let path = directory.join("notes.md");
    open(&mut runtime, &checkout_id, &path, false, false);
    open(&mut runtime, &checkout_id, &path, false, true);
    let tab_id = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &tab_id, "unsaved draft\n");
    runtime.snapshot.status.herdr.state = "connected".to_owned();
    let pane_id = "pane-cli";
    let checkout = runtime
        .snapshot
        .navigator
        .workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.checkouts)
        .find(|checkout| checkout.id == checkout_id)
        .unwrap();
    // The close acts on the current tab's two displays. A different Agent
    // tab inherits its own copies and therefore is not the last reference.
    checkout.tabs.push(tab(
        "workspace:order",
        &checkout_id,
        Some(pane(pane_id, &directory.to_string_lossy())),
    ));
    checkout.active_tab_id = checkout.tabs[0].id.clone();
    let first = display(&mut runtime, 0, "notes.md");
    let second = display(&mut runtime, 1, "notes.md");
    let expected = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            pane_id,
            crate::workspace_control::Query::Info,
        )
        .unwrap()
        .context;
    let first_id = format!("{}-first", unix_milliseconds());
    assert!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                pane_id,
                &expected,
                &first_id,
                Action::Close {
                    view_id: first,
                    expected_browser_area: None,
                },
                Ok(None),
            )
            .unwrap()
            .changed
    );
    let second_id = format!("{}-second", unix_milliseconds());
    assert_eq!(
        runtime
            .workspace_control_action(
                crate::node::TEST_NODE,
                pane_id,
                &expected,
                &second_id,
                Action::Close {
                    view_id: second.clone(),
                    expected_browser_area: None,
                },
                Ok(None),
            )
            .unwrap_err()
            .reason,
        "unsaved_document"
    );
    assert_eq!(display(&mut runtime, 0, "notes.md"), second);
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && tab.dirty)
    );
}

/// Contract 4.1, B5: a web frame that still shows a dirty document in two
/// views sends each close as if another view remained. The first is not
/// the last, so the save it carries is not needed; the second reaches the
/// last view without a save and is refused, naming the document, with
/// nothing changed. Don't Save then closes it without saving.
#[test]
fn a_close_that_reaches_the_last_view_of_unsaved_text_without_a_save_is_refused() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-close-unsaved");
    let path = directory.join("notes.md");
    open(&mut runtime, &checkout_id, &path, false, false);
    open(&mut runtime, &checkout_id, &path, false, true);
    let tab_id = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &tab_id, "draft\n");
    let (first, second) = (
        display(&mut runtime, 0, "notes.md"),
        display(&mut runtime, 1, "notes.md"),
    );
    let kept = |runtime: &Runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && tab.dirty)
            && runtime.editor_documents[&tab_id].save.is_none()
    };

    assert!(act(
        &mut runtime,
        serde_json::json!({
            "action": "close", "display_id": first,
            "pending_save": {"tab_id": tab_id, "path": path, "contents_utf8": "draft\n"},
        }),
    ));
    assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
    assert!(kept(&runtime), "a close that is not the last saves nothing");

    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": second}),
    ));
    let refused = runtime.snapshot.status.last_error.clone().expect("refused");
    assert_eq!(refused.kind, "view_layout.unsaved");
    assert!(refused.message.contains("notes.md"), "{}", refused.message);
    assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
    assert!(kept(&runtime), "the draft stays in the core");

    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": second, "discard": true}),
    ));
    assert_eq!(tree(&mut runtime).display_count, 0);
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "notes\n");
}

/// B10, S5.5 save-then-close: closing a dirty document's last display saves
/// it first; a refused save keeps the display and the draft, and a save that
/// lands closes both.
#[test]
fn closing_the_last_display_of_a_dirty_document_saves_it_first() {
    let (runtime, checkout_id, directory) = views_runtime("view-close-last");
    let path = directory.join("notes.md");
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    open(
        &mut shared.lock().unwrap(),
        &checkout_id,
        &path,
        false,
        false,
    );
    wait(&shared, "the file to be read", |runtime| {
        !runtime.editor_documents.is_empty()
    });
    let tab_id = tab_of(&shared.lock().unwrap(), "notes.md");
    draft(&mut shared.lock().unwrap(), &tab_id, "draft\n");
    std::fs::write(&path, "changed elsewhere\n").expect("fixture");
    let close = |runtime: &mut Runtime| {
        let shown = display(runtime, 0, "notes.md");
        act(
            runtime,
            serde_json::json!({
                "action": "close", "display_id": shown,
                "pending_save": {"tab_id": tab_id, "path": path, "contents_utf8": "draft\n"},
            }),
        )
    };

    close(&mut shared.lock().unwrap());
    wait(&shared, "the save to be refused", |runtime| {
        runtime.editor_documents[&tab_id].conflict.is_some()
    });
    {
        let mut runtime = shared.lock().unwrap();
        assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
        assert!(
            runtime
                .snapshot
                .editor
                .tabs
                .iter()
                .any(|tab| tab.id == tab_id && tab.dirty)
        );
        // The lane waits on the save with the view it shows.
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        assert!(view.open && view.documents);
        runtime.dispatch_json(&explorer_event(
            "file_conflict",
            serde_json::json!({"tab_id": tab_id, "action": "keep_editing"}),
        ));
        close(&mut runtime);
    }
    wait(&shared, "the document to close", |runtime| {
        runtime.snapshot.editor.tabs.is_empty()
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft\n");
    let mut runtime = shared.lock().unwrap();
    let emptied = tree(&mut runtime);
    assert_eq!(emptied.display_count, 0);
    // B10: the last area stays, empty; the viewing column closes once the
    // save has landed, and the lane with it, there being no tool column
    // (PRD lane-two-columns B12).
    assert!(matches!(emptied.root, ViewNodeSnapshot::Area(_)));
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!((view.documents, view.tool), (false, None));
}

/// B5: a draft in one display keeps every display of that document open.
#[test]
fn a_draft_in_one_display_keeps_every_display_of_its_document_open() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-draft-pins");
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        true,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        true,
    );
    assert_eq!(
        labels(&mut runtime),
        vec![vec!["notes.md*"], vec!["notes.md"]]
    );

    let tab_id = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &tab_id, "edited\n");

    assert_eq!(
        labels(&mut runtime),
        vec![vec!["notes.md"], vec!["notes.md"]]
    );
    assert!(!runtime.snapshot.editor.tabs[0].preview);
}

/// B5, contract 4.2: a save keeps every display of its document open while
/// it runs and after it lands, though no draft came before it.
#[test]
fn a_save_without_a_draft_keeps_every_display_of_its_document_open() {
    let (runtime, checkout_id, directory) = views_runtime("view-save-pins");
    let path = directory.join("notes.md");
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    open(
        &mut shared.lock().unwrap(),
        &checkout_id,
        &path,
        true,
        false,
    );
    wait(&shared, "the file to be read", |runtime| {
        !runtime.editor_documents.is_empty()
    });
    let tab_id = {
        let mut runtime = shared.lock().unwrap();
        open(&mut runtime, &checkout_id, &path, false, true);
        assert_eq!(
            labels(&mut runtime),
            vec![vec!["notes.md*"], vec!["notes.md"]]
        );
        let tab_id = tab_of(&runtime, "notes.md");
        runtime.dispatch_json(&explorer_event(
            "file_save",
            serde_json::json!({"tab_id": tab_id, "path": path, "contents_utf8": "notes\n"}),
        ));
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert_eq!(
            labels(&mut runtime),
            vec![vec!["notes.md"], vec!["notes.md"]],
            "a document being saved is never a preview"
        );
        tab_id
    };
    wait(&shared, "the save to land", |runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && !tab.dirty)
    });
    let mut runtime = shared.lock().unwrap();
    assert_eq!(
        labels(&mut runtime),
        vec![vec!["notes.md"], vec!["notes.md"]]
    );
    assert!(!runtime.snapshot.editor.tabs[0].preview);
}

/// Contract 4.2: a draft or a conflict choice names its document. One that
/// names a document no longer open is refused with the reason and never
/// lands in the document in use, where a stray draft would overwrite the
/// operator's edit and a stray reload would discard it.
#[test]
fn a_draft_or_conflict_choice_for_a_closed_document_is_refused_and_changes_nothing() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-closed-draft");
    files(&directory, &["closed.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("closed.md"),
        false,
        false,
    );
    let closed = tab_of(&runtime, "closed.md");
    assert!(runtime.dispatch_json(&explorer_event(
        "file_close",
        serde_json::json!({"tab_id": closed}),
    )));
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let notes = tab_of(&runtime, "notes.md");
    draft(&mut runtime, &notes, "draft\n");
    let in_use = |runtime: &mut Runtime| {
        let documents = runtime
            .snapshot_delta_payload(0)
            .documents
            .expect("a fresh reader gets the section");
        let (_, document) = documents
            .changed
            .iter()
            .find(|(tab_id, _)| *tab_id == notes)
            .expect("the document in use is on screen");
        (
            labels(runtime),
            document.contents_utf8.clone(),
            document.dirty,
        )
    };
    let before = in_use(&mut runtime);
    assert_eq!(
        before,
        (
            vec![vec!["notes.md".to_owned()]],
            Some("draft\n".to_owned()),
            true
        )
    );

    for (kind, payload, refusal) in [
        (
            "file_draft",
            serde_json::json!({"tab_id": closed, "contents_utf8": "stray\n"}),
            "file.draft_rejected",
        ),
        (
            "file_conflict",
            serde_json::json!({"tab_id": closed, "action": "reload"}),
            "file.conflict_without_tab",
        ),
    ] {
        runtime.dispatch_json(&explorer_event(kind, payload));
        let error = runtime
            .snapshot
            .status
            .last_error
            .clone()
            .unwrap_or_else(|| panic!("{kind} naming a closed document is refused"));
        assert_eq!(error.kind, refusal);
        assert!(error.message.contains(&closed), "{}", error.message);
        assert_eq!(in_use(&mut runtime), before, "{kind} changed nothing");
    }
}

/// S5.5 B9: a renamed file keeps its display, which shows the new name.
#[test]
fn a_renamed_file_keeps_its_display() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-rename");
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let before = areas(&tree(&mut runtime))[0].2[0].clone();

    assert!(runtime.dispatch_json(&explorer_event(
        "path_rename",
        serde_json::json!({
            "root": directory.to_string_lossy(),
            "path": directory.join("notes.md").to_string_lossy(),
            "name": "renamed.md",
        }),
    )));

    let after = areas(&tree(&mut runtime))[0].2[0].clone();
    assert_eq!(
        (after.id, after.tab_id, after.label.as_str(), after.state),
        (
            before.id,
            before.tab_id,
            "renamed.md",
            ViewDisplayState::Open
        )
    );
    assert_eq!(after.path, directory.join("renamed.md").to_string_lossy());
}

/// B14-B17, contract 5: a restart brings back the tree, its ratio, each
/// area's order, preview and active display, the area in use, the mode and
/// the tools; a file that is gone comes back as an unavailable display that
/// Retry reads again once the file is back.
#[test]
fn a_restart_restores_the_view_tree_and_marks_a_missing_file_unavailable() {
    let (runtime, checkout_id, directory) = strip_checkout("view-restart");
    let (_views, state) = views_path("view-restart");
    let mut runtime = with_views(runtime, &state);
    // The test keeps the checkout folder: it restarts on that checkout after
    // the runtime is gone.
    let _checkout = hold_dirs(&mut runtime);
    files(&directory, &["a.md", "b.md", "gone.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        true,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("gone.md"),
        false,
        true,
    );
    let ViewNodeSnapshot::Split(divider) = tree(&mut runtime).root else {
        panic!("two areas");
    };
    act(
        &mut runtime,
        serde_json::json!({"action": "resize", "split_id": divider.id, "ratio": 0.3}),
    );
    let a = display(&mut runtime, 0, "a.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    layout(
        &mut runtime,
        serde_json::json!({"tool": "changes", "pinned": true, "explorer_width": 300}),
    );
    let before = tree(&mut runtime);
    drop(runtime);
    std::fs::remove_file(directory.join("gone.md")).expect("remove fixture");

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views(restarted, &state);
    let after = tree(&mut restarted);

    let shape = |layout: &ViewLayoutSnapshot| {
        let ViewNodeSnapshot::Split(split) = &layout.root else {
            panic!("two areas");
        };
        let displays = areas(layout)
            .into_iter()
            .map(|(id, active, displays)| {
                let shown: Vec<_> = displays
                    .iter()
                    .map(|display| (display.id.clone(), display.label.clone(), display.preview))
                    .collect();
                (id, active, shown)
            })
            .collect::<Vec<_>>();
        (
            split.axis,
            split.ratio,
            layout.active_area.clone(),
            displays,
        )
    };
    assert_eq!(shape(&after), shape(&before));
    assert!(
        (divider.ratio - 0.5).abs() < f32::EPSILON && (shape(&after).1 - 0.3).abs() < f32::EPSILON
    );
    let states = |layout: &ViewLayoutSnapshot| {
        areas(layout)
            .into_iter()
            .flat_map(|(_, _, displays)| displays)
            .map(|display| (display.label, display.state))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        states(&after),
        vec![
            ("a.md".to_owned(), ViewDisplayState::Open),
            ("b.md".to_owned(), ViewDisplayState::Open),
            ("gone.md".to_owned(), ViewDisplayState::Unavailable),
        ]
    );
    let view = restarted.snapshot.workspace_view.clone().unwrap();
    assert_eq!(
        (view.documents, view.tool, view.pinned, view.explorer_width),
        (
            true,
            Some(crate::workspace_views::Tool::Changes),
            true,
            Some(300)
        )
    );
    assert_eq!(active_label(&restarted).as_deref(), Some("a.md"));

    std::fs::write(directory.join("gone.md"), "back\n").expect("fixture");
    let gone = display(&mut restarted, 1, "gone.md");
    assert!(act(
        &mut restarted,
        serde_json::json!({"action": "retry", "display_id": gone}),
    ));
    assert_eq!(
        states(&tree(&mut restarted))[2],
        ("gone.md".to_owned(), ViewDisplayState::Open)
    );
}

/// B4, B20: Open to the side of a file restored as unavailable, once the
/// file is back, opens it in a new area beside the view that could not show
/// it, as it would any file, rather than only focusing that view.
#[test]
fn open_to_the_side_of_a_restored_file_that_came_back_opens_it_beside() {
    let (runtime, checkout_id, directory) = strip_checkout("view-back-beside");
    let (_views, state) = views_path("view-back-beside");
    let gone = directory.join("gone.md");
    let mut views = crate::workspace_views::WorkspaceViews::default();
    let layout = &mut views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
        .layout;
    let restored = layout.new_display(
        &gone.to_string_lossy(),
        crate::view_layout::DisplayKind::File,
        None,
        false,
    );
    layout.insert("a1", restored, 1).unwrap();
    crate::workspace_views::save(&state, &views).unwrap();
    let mut runtime = with_views(runtime, &state);
    assert_eq!(
        display_states(&mut runtime)
            .into_iter()
            .map(|(label, state, _)| (label, state))
            .collect::<Vec<_>>(),
        vec![("gone.md".to_owned(), ViewDisplayState::Unavailable)]
    );

    std::fs::write(&gone, "back\n").expect("fixture");
    open(&mut runtime, &checkout_id, &gone, false, true);

    assert_eq!(labels(&mut runtime), vec![vec!["gone.md"], vec!["gone.md"]]);
    let view = tree(&mut runtime);
    assert_eq!(
        view.active_area,
        areas(&view)[1].0,
        "the view beside is in use"
    );
    assert!(
        display_states(&mut runtime)
            .iter()
            .all(|(_, state, _)| *state == ViewDisplayState::Open)
    );
}

/// A restarted runtime whose stored Views hold one diff display of `x.md`,
/// stored with the Changes group `committed`.
fn restored_diff(name: &str, committed: Option<bool>) -> (Runtime, PathBuf) {
    let (runtime, _checkout_id, directory) = strip_checkout(name);
    let (_views, state) = views_path(name);
    let mut views = crate::workspace_views::WorkspaceViews::default();
    let layout = &mut views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
        .layout;
    let mut diff = layout.new_display(
        &directory.join("x.md").to_string_lossy(),
        crate::view_layout::DisplayKind::Diff,
        Some(false),
        false,
    );
    diff.committed = committed;
    layout.insert("a1", diff, 1).unwrap();
    crate::workspace_views::save(&state, &views).unwrap();
    (with_views(runtime, &state), directory)
}

fn display_states(runtime: &mut Runtime) -> Vec<(String, ViewDisplayState, Option<String>)> {
    areas(&tree(runtime))
        .into_iter()
        .flat_map(|(_, _, displays)| displays)
        .map(|display| (display.label, display.state, display.reason))
        .collect()
}

/// B16: a diff view stored without its Changes group, as a hand-edited file
/// can hold it, comes back as the working diff it defaults to, once, rather
/// than as a view that never finds its tab beside a second one that does.
#[test]
fn a_diff_view_stored_without_its_group_comes_back_open_once() {
    let (mut runtime, _directory) = restored_diff("view-diff-no-group", None);
    assert_eq!(
        display_states(&mut runtime),
        vec![(
            "x.md (working diff)".to_owned(),
            ViewDisplayState::Open,
            None
        )]
    );
}

/// B16: Retry on a diff view that has lost its tab opens the diff again,
/// where it used to do nothing while the view offered it.
#[test]
fn retry_on_a_diff_view_without_its_tab_opens_the_diff_again() {
    let (mut runtime, directory) = restored_diff("view-diff-retry", Some(false));
    // The diff's tab goes away without the reconcile seeing it close.
    let tab = tab_of(&runtime, "x.md (working diff)");
    runtime.snapshot.editor.tabs.retain(|held| held.id != tab);
    let store = runtime.workspace_views.as_mut().unwrap();
    for display in store
        .views
        .entry(crate::node::TEST_NODE, &directory.to_string_lossy())
        .layout
        .displays_mut()
    {
        display.tab_id = None;
    }
    assert_eq!(
        display_states(&mut runtime),
        vec![(
            "x.md (working diff)".to_owned(),
            ViewDisplayState::Unavailable,
            Some("This view's diff is not open; Retry opens it again".to_owned())
        )]
    );

    let lost = display(&mut runtime, 0, "x.md (working diff)");
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "retry", "display_id": lost}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        display_states(&mut runtime),
        vec![(
            "x.md (working diff)".to_owned(),
            ViewDisplayState::Open,
            None
        )]
    );
}

/// Contract 1: an S6 file's View tabs come back as one area, in their saved
/// order, with the active tab and the preview, and the next save writes the
/// new schema.
#[test]
fn a_schema_1_views_file_migrates_into_one_area() {
    let (runtime, _checkout_id, directory) = strip_checkout("view-v1");
    let (_views, state) = views_path("view-v1");
    std::fs::write(directory.join("b.md"), "b\n").expect("fixture");
    let file = |name: &str, preview: bool| serde_json::json!({"path": directory.join(name), "kind": "file", "preview": preview});
    let v1 = serde_json::json!({"schema_version": 1, "workspaces": [{
        "device_id": crate::node::TEST_NODE, "path": directory, "mode": "together",
        "tabs": [file("b.md", false), file("notes.md", true)],
        "active": file("b.md", false),
    }]});
    std::fs::write(&state, serde_json::to_vec(&v1).unwrap()).expect("fixture");
    let (_, diagnostics) = WorkspaceViewStore::open(state.clone(), Default::default());
    assert_eq!(
        diagnostics
            .iter()
            .map(|(kind, _)| *kind)
            .collect::<Vec<_>>(),
        vec!["workspace_views.migrated"]
    );

    let mut runtime = with_views(runtime, &state);
    assert_eq!(labels(&mut runtime), vec![vec!["b.md", "notes.md*"]]);
    assert_eq!(active_label(&runtime).as_deref(), Some("b.md"));

    let notes = display(&mut runtime, 0, "notes.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": notes}),
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
    assert_eq!(written["schema_version"], 5);
}

/// Contract 3.1, B19: the documents on screen ride their own section, each
/// with its own revision: a fresh reader gets all of them, an unchanged one
/// is not sent again, and a keystroke re-sends its own document and not the
/// editor.
#[test]
fn a_keystroke_resends_only_its_own_document() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-documents");
    files(&directory, &["b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        true,
    );
    let (notes, b) = (tab_of(&runtime, "notes.md"), tab_of(&runtime, "b.md"));
    let changed = |documents: &crate::model::DocumentsDelta| {
        documents
            .changed
            .iter()
            .map(|(tab_id, _)| tab_id.clone())
            .collect::<Vec<_>>()
    };

    let fresh = runtime.snapshot_delta_payload(0);
    let documents = fresh.documents.expect("a fresh reader gets the section");
    assert_eq!(documents.visible, vec![notes.clone(), b.clone()]);
    assert_eq!(changed(&documents), vec![notes.clone(), b.clone()]);
    let cursor = fresh.revision;
    assert!(runtime.snapshot_delta_payload(cursor).documents.is_none());

    draft(&mut runtime, &notes, "one\n");
    let first = runtime.snapshot_delta_payload(cursor);
    assert_eq!(
        changed(first.documents.as_ref().unwrap()),
        vec![notes.clone()]
    );
    draft(&mut runtime, &notes, "two\n");
    let second = runtime.snapshot_delta_payload(first.revision);
    let documents = second.documents.expect("the edited document");
    assert_eq!(documents.visible, vec![notes.clone(), b]);
    assert_eq!(changed(&documents), vec![notes]);
    assert_eq!(
        documents.changed[0].1.contents_utf8.as_deref(),
        Some("two\n")
    );
    assert!(second.editor.is_none(), "the editor section is not re-sent");
}

/// A5, A10: the diffs the View areas show are taken in the Changes read,
/// one per area, and none while the Views are hidden.
#[test]
fn the_changes_read_takes_every_diff_on_screen() {
    let (mut runtime, _checkout_id, directory) = views_runtime("view-diffs");
    let changed: Vec<PathBuf> = ["x.md", "y.md"]
        .iter()
        .map(|name| directory.join(name))
        .collect();
    runtime.snapshot.changes.edit().root_path = Some(directory.to_string_lossy().into_owned());
    runtime.snapshot.changes.edit().entries = changed
        .iter()
        .map(|path| crate::model::ChangedFileSnapshot {
            path: path.to_string_lossy().into_owned(),
            relative_path: path.file_name().unwrap().to_string_lossy().into_owned(),
            previous_relative_path: None,
            status: crate::model::ChangedFileStatus::Untracked,
            added_lines: Some(1),
            removed_lines: Some(0),
        })
        .collect();
    for (path, beside) in changed.iter().zip([false, true]) {
        assert!(runtime.dispatch_json(&explorer_event(
            "changes_select",
            serde_json::json!({"path": path, "committed": false, "preview": false, "beside": beside}),
        )));
    }
    assert_eq!(
        labels(&mut runtime),
        vec![vec!["x.md (working diff)"], vec!["y.md (working diff)"]]
    );
    let diffs = |runtime: &mut Runtime| {
        runtime
            .changes_request()
            .map(|request| {
                request
                    .diffs
                    .into_iter()
                    .map(|target| (target.path, target.committed))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    assert_eq!(
        diffs(&mut runtime),
        changed
            .iter()
            .map(|path| (path.to_string_lossy().into_owned(), false))
            .collect::<Vec<_>>()
    );

    layout(&mut runtime, serde_json::json!({"open": false}));
    assert!(
        diffs(&mut runtime).is_empty(),
        "a closed lane reads no diff"
    );
}

/// B4, B10: Open to the side with no view open has nothing to stand beside,
/// so it opens in the empty area rather than splitting next to it; an empty
/// area is only ever the root.
#[test]
fn open_to_the_side_with_no_view_open_opens_in_the_empty_area() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-beside-empty");
    files(&directory, &["a.md", "b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        true,
    );
    assert_eq!(labels(&mut runtime), vec![vec!["a.md"]]);
    assert_eq!(active_label(&runtime).as_deref(), Some("a.md"));

    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        true,
    );
    assert_eq!(labels(&mut runtime), vec![vec!["a.md"], vec!["b.md"]]);
}

/// B6, D-03: a view dragged into an area that already shows its document
/// takes its twin's place there instead of standing beside it, so no area
/// ever holds one document twice; the document itself stays open.
#[test]
fn a_view_moved_onto_its_documents_twin_replaces_it() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-move-twin");
    files(&directory, &["a.md", "b.md"]);
    for name in ["a.md", "b.md"] {
        open(
            &mut runtime,
            &checkout_id,
            &directory.join(name),
            false,
            false,
        );
    }
    let a = display(&mut runtime, 0, "a.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": a}),
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        true,
    );
    assert_eq!(
        labels(&mut runtime),
        vec![vec!["a.md", "b.md"], vec!["a.md"]]
    );

    let twin = display(&mut runtime, 1, "a.md");
    let first = area_id(&mut runtime, 0);
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "move", "display_id": twin, "area_id": first, "index": 2}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(labels(&mut runtime), vec![vec!["b.md", "a.md"]]);
    assert_eq!(display(&mut runtime, 0, "a.md"), twin);
    assert_eq!(active_label(&runtime).as_deref(), Some("a.md"));
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.label == "a.md")
    );
}

/// The Changes pump's order when each read takes less than one tick
/// (`BackgroundRead::poll`): a wake builds its request, then takes the answer
/// to the read the last wake started, and starts a read of its own request
/// unless that is the one just read. `started` is the History row each read
/// started asks for.
#[derive(Default)]
struct HistoryPump {
    inflight: Option<crate::changes::ChangesRequest>,
    settled: Option<crate::changes::ChangesRequest>,
    started: Vec<Option<String>>,
}

impl HistoryPump {
    fn wake(&mut self, runtime: &mut Runtime) {
        let request = runtime.changes_request().expect("History is read");
        let landed = self.inflight.take().map(|asked| {
            let changes = crate::changes::read(&asked).0;
            let answer = crate::changes::ChangesAnswer {
                detail: None,
                key: Some(asked.key()),
                selection: asked.selection(),
                changes,
            };
            self.settled = Some(asked);
            answer
        });
        if self.settled.as_ref() != Some(&request) {
            self.started.push(request.selected_path.clone());
            self.inflight = Some(request);
        }
        if let Some(answer) = landed {
            runtime.ingest_changes(answer);
        }
        // The frame the answer announces reads the snapshot.
        runtime.sync_workspace_view();
    }
}

/// A5, B19: every Changes answer lands one wake after its request, so a read
/// started for a History row the operator has left since must not bring
/// that row back. It would be asked for again, the answer after it would
/// bring the newer row back, and the two would chase each other with a Git
/// read and a frame on every tick while nothing is driven.
#[test]
fn a_late_history_answer_does_not_bring_back_the_row_the_operator_left() {
    let (mut runtime, checkout_id, directory) = views_runtime("view-history-late");
    // History reads while the lane shows the Changes tool.
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    files(&directory, &["a.ts", "e.rs", "f.go"]);
    for arguments in [
        &["add", "."][..],
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "base",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(arguments)
                .current_dir(&directory)
                .status()
                .expect("git runs")
                .success()
        );
    }
    for name in ["e.rs", "f.go"] {
        std::fs::write(directory.join(name), format!("{name} changed\n")).expect("an edit");
    }
    let select = |runtime: &mut Runtime, name: &str| {
        assert!(runtime.dispatch_json(&explorer_event(
            "changes_select",
            serde_json::json!({"path": directory.join(name), "committed": false, "preview": false}),
        )));
        assert_eq!(runtime.snapshot.status.last_error, None);
    };
    let row = |name: &str| Some(directory.join(name).to_string_lossy().into_owned());
    let mut pump = HistoryPump::default();
    // The first read lists the two changed files.
    pump.wake(&mut runtime);
    pump.wake(&mut runtime);

    select(&mut runtime, "f.go");
    pump.wake(&mut runtime);
    select(&mut runtime, "e.rs");
    // A file takes the area, so no diff is in front, and the operator goes
    // back to History in the lane: the request names the row it has
    // selected.
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.ts"),
        false,
        false,
    );
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    pump.started.clear();
    for _ in 0..4 {
        pump.wake(&mut runtime);
    }

    assert_eq!(pump.started, vec![row("e.rs")]);
    assert_eq!(runtime.snapshot.changes.selected_path, row("e.rs"));
}

// Issue 155: a browser display shows a page the desktop host draws. It binds
// to no document, so the reconcile never takes it away, and its address and
// title are what the page last said.

fn browser_open(runtime: &mut Runtime, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event("browser_open", payload))
}

fn browser_displays(runtime: &mut Runtime) -> Vec<ViewDisplaySnapshot> {
    areas(&tree(runtime))
        .into_iter()
        .flat_map(|(_, _, displays)| displays)
        .filter(|display| display.kind == crate::view_layout::DisplayKind::Browser)
        .collect()
}

fn receipt(runtime: &Runtime, request_id: &str) -> crate::model::BrowserOpenReceiptSnapshot {
    runtime
        .snapshot
        .status
        .browser_opens
        .iter()
        .find(|receipt| receipt.request_id == request_id)
        .unwrap_or_else(|| panic!("a receipt for {request_id}"))
        .clone()
}

/// A Herdr session with one tab in the checkout, and the id of its pane.
fn with_pane(runtime: &mut Runtime, directory: &Path) -> String {
    let tabs = ["w-order:t1"];
    assert!(runtime.ingest_session(Ok(tab_order_payload(
        &directory.to_string_lossy(),
        &tabs,
        &tabs,
        "w-order:t1"
    ))));
    runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .next()
        .expect("the session's pane")
        .id
        .clone()
}

fn last_error_kind(runtime: &Runtime) -> Option<&str> {
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.as_str())
}

/// PRD sidebar-task-lens B14, B27, B47, B48, D-43, D-50: a pull request's
/// card opens in the lane's 보기 slot, never as a View, with the lane, the
/// Workspace and its Views where they were; the same card again is not a
/// second entry, ← steps back, a tool called hides the slot and keeps it on
/// the rail, ✕ empties it, and an unknown pull request is refused.
#[test]
fn a_pull_request_card_opens_in_the_detail_slot_and_never_as_a_view() {
    let (mut runtime, checkout_id, directory) = views_runtime("pr-card");
    files(&directory, &["changed.rs"]);
    layout(&mut runtime, serde_json::json!({"tool": null}));
    let pull = |number: u32| -> crate::model::PullRequestSnapshot {
        serde_json::from_value(serde_json::json!({
            "number":number,"title":"Ship","url":format!("https://github.com/acme/app/pull/{number}"),
            "head_branch":format!("feature-{number}"),"base_branch":"main","badge":"open","checks":"passing",
            "is_draft":false,"closing_issues":[]
        }))
        .unwrap()
    };
    runtime.snapshot.navigator.workspaces[0].pull_requests = vec![pull(42), pull(43)];
    let event = |runtime: &mut Runtime, number: u32| {
        runtime.dispatch_json(&explorer_event(
            "pr_open",
            serde_json::json!({"workspace_id": "workspace:order", "pr_number": number}),
        ))
    };
    let displays = |runtime: &mut Runtime| {
        areas(&tree(runtime))
            .into_iter()
            .flat_map(|(_, _, displays)| displays)
            .count()
    };
    let item = |number: u32| crate::model::DetailItem::PullRequest {
        workspace_id: "workspace:order".into(),
        number,
    };
    let before = displays(&mut runtime);
    let front = runtime.snapshot.ui_state.focused_checkout_id.clone();
    assert!(event(&mut runtime, 42));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let detail = runtime.snapshot.ui_state.detail.clone();
    assert_eq!((detail.items, detail.shown), (vec![item(42)], true));
    assert_eq!(displays(&mut runtime), before, "no View is opened");
    assert_eq!(
        runtime.snapshot.workspace_view.as_ref().unwrap().tool,
        None,
        "the Workspace's own tool is kept for ✕ to return to"
    );
    assert_eq!(runtime.snapshot.ui_state.focused_checkout_id, front);
    event(&mut runtime, 43);
    event(&mut runtime, 43);
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        vec![item(42), item(43)]
    );
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert!(
        runtime.snapshot.ui_state.detail.shown,
        "a tool called draws in the tool column and leaves the slot in front (lane-two-columns B9)"
    );
    layout(&mut runtime, serde_json::json!({"documents": true}));
    assert!(
        !runtime.snapshot.ui_state.detail.shown,
        "the documents brought in front hide the slot and keep what it holds"
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items.len(), 2);
    assert!(runtime.dispatch_json(&explorer_event(
        "detail_show",
        serde_json::json!({"shown": true})
    )));
    assert!(runtime.dispatch_json(&explorer_event("detail_back", serde_json::json!({}))));
    assert_eq!(runtime.snapshot.ui_state.detail.items, vec![item(42)]);
    // A changed file opened from the card shows as a document in front of
    // the card, which keeps its rail slot to come back to (B49).
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("changed.rs"),
        false,
        false,
    );
    assert!(runtime.snapshot.workspace_view.as_ref().unwrap().documents);
    assert!(!runtime.snapshot.ui_state.detail.shown);
    assert_eq!(runtime.snapshot.ui_state.detail.items, vec![item(42)]);
    // The slot is the window's: an open in a Workspace behind the one in
    // front, such as an agent's `hide view open`, leaves it on screen.
    assert!(runtime.dispatch_json(&explorer_event(
        "detail_show",
        serde_json::json!({"shown": true})
    )));
    runtime.apply_area_intent_to(
        &("local".to_owned(), "/elsewhere".to_owned()),
        AreaIntent::Documents,
    );
    assert!(runtime.snapshot.ui_state.detail.shown);
    assert!(runtime.dispatch_json(&explorer_event("detail_close", serde_json::json!({}))));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default()
    );
    event(&mut runtime, 99);
    assert_eq!(
        last_error_kind(&runtime),
        Some("pr_open.unknown_pull_request")
    );
    assert!(runtime.snapshot.ui_state.detail.items.is_empty());
}

/// PRD lane-two-columns B15, B22: a lane closed whole keeps the 보기 slot
/// shown, so ⌘E brings the card back, beside its tool or alone; and the
/// rail's 보기 button opens a closed lane while the slot is already shown,
/// as after a checkout change into a lane that never opened.
#[test]
fn closing_the_lane_whole_keeps_the_card_and_the_slot_button_opens_a_closed_lane() {
    let (mut runtime, _checkout_id, _directory) = views_runtime("lane-close-card");
    layout(&mut runtime, serde_json::json!({"tool": null}));
    let pull: crate::model::PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature-42","base_branch":"main","badge":"open","checks":"passing",
        "is_draft":false,"closing_issues":[]
    }))
    .unwrap();
    runtime.snapshot.navigator.workspaces[0].pull_requests = vec![pull];
    assert!(runtime.dispatch_json(&explorer_event(
        "pr_open",
        serde_json::json!({"workspace_id": "workspace:order", "pr_number": 42}),
    )));
    let lane = |runtime: &Runtime| {
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        (
            view.open,
            view.documents,
            view.tool,
            runtime.snapshot.ui_state.detail.shown,
        )
    };
    // The card beside Changes: ⌘E closes both and brings both back.
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Changes), true));
    layout(&mut runtime, serde_json::json!({"open": false}));
    assert_eq!(lane(&runtime), (false, false, Some(Tool::Changes), true));
    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(lane(&runtime), (true, false, Some(Tool::Changes), true));
    // The card alone: no column of the Workspace's own, and the reopen
    // draws the slot rather than choosing the Explorer.
    layout(&mut runtime, serde_json::json!({"tool": null}));
    layout(&mut runtime, serde_json::json!({"open": false}));
    assert_eq!(lane(&runtime), (false, false, None, true));
    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(lane(&runtime), (true, false, None, true));
    // The slot shown in a closed lane: the button opens the lane (B22).
    layout(&mut runtime, serde_json::json!({"open": false}));
    let calls = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .unwrap()
        .lane_called;
    assert!(runtime.dispatch_json(&explorer_event(
        "detail_show",
        serde_json::json!({"shown": true})
    )));
    assert_eq!(lane(&runtime), (true, false, None, true));
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(
        view.lane_called > calls,
        "the button is a call that shows the lane"
    );
    assert_eq!(view.lane_called_side, crate::model::LaneCallSide::View);
    // The documents brought in front still hide the slot (B10).
    layout(&mut runtime, serde_json::json!({"documents": true}));
    assert!(!runtime.snapshot.ui_state.detail.shown);
}

/// Operator decision 2026-10-23 (B15): a lane ⌘E closed with the card
/// beside a tool opens on what calls it alone; a tool pressed leaves the
/// card on the rail, and the card's button leaves the tool closed.
#[test]
fn a_call_into_a_lane_closed_with_the_card_opens_its_own_column_alone() {
    let (mut runtime, _checkout_id, _directory) = views_runtime("lane-closed-card-call");
    let pull: crate::model::PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature-42","base_branch":"main","badge":"open","checks":"passing",
        "is_draft":false,"closing_issues":[]
    }))
    .unwrap();
    runtime.snapshot.navigator.workspaces[0].pull_requests = vec![pull];
    let lane = |runtime: &Runtime| {
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        (
            view.open,
            view.tool,
            runtime.snapshot.ui_state.detail.shown,
            runtime.snapshot.ui_state.detail.items.len(),
        )
    };
    layout(&mut runtime, serde_json::json!({"tool": "changes"}));
    assert!(runtime.dispatch_json(&explorer_event(
        "pr_open",
        serde_json::json!({"workspace_id": "workspace:order", "pr_number": 42}),
    )));
    layout(&mut runtime, serde_json::json!({"open": false}));
    assert_eq!(lane(&runtime), (false, Some(Tool::Changes), true, 1));

    layout(&mut runtime, serde_json::json!({"tool": "memory"}));
    assert_eq!(
        lane(&runtime),
        (true, Some(Tool::Memory), false, 1),
        "the tool alone, the card kept on the rail"
    );

    layout(&mut runtime, serde_json::json!({"open": false}));
    assert!(runtime.dispatch_json(&explorer_event(
        "detail_show",
        serde_json::json!({"shown": true})
    )));
    assert_eq!(
        lane(&runtime),
        (true, None, true, 1),
        "the card alone, the tool closed"
    );
}

/// PRD lane-two-columns B9, B19: with the lane already open on a tool, a
/// card opened, the rail's button on a shown slot and ← are each a call
/// the viewing column's, so a stacked lane draws the slot over the tool
/// instead of leaving it behind.
#[test]
fn every_way_the_slot_comes_forward_is_a_viewing_column_call() {
    use crate::model::LaneCallSide;
    let (mut runtime, _checkout_id, _directory) = views_runtime("lane-slot-forward");
    let pulls: Vec<crate::model::PullRequestSnapshot> = [42, 43]
        .into_iter()
        .map(|number| {
            serde_json::from_value(serde_json::json!({
                "number":number,"title":"Ship","url":format!("https://github.com/acme/app/pull/{number}"),
                "head_branch":format!("feature-{number}"),"base_branch":"main","badge":"open","checks":"passing",
                "is_draft":false,"closing_issues":[]
            }))
            .unwrap()
        })
        .collect();
    runtime.snapshot.navigator.workspaces[0].pull_requests = pulls;
    let called = |runtime: &Runtime| {
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        (
            view.open,
            view.tool,
            view.lane_called,
            view.lane_called_side,
        )
    };
    layout(&mut runtime, serde_json::json!({"tool": "project"}));
    let (_, _, tool_call, _) = called(&runtime);
    for (step, event, payload) in [
        (
            "a card",
            "pr_open",
            serde_json::json!({"workspace_id": "workspace:order", "pr_number": 42}),
        ),
        (
            "a second card",
            "pr_open",
            serde_json::json!({"workspace_id": "workspace:order", "pr_number": 43}),
        ),
        (
            "the button on a shown slot",
            "detail_show",
            serde_json::json!({"shown": true}),
        ),
        ("←", "detail_back", serde_json::json!({})),
    ] {
        let before = called(&runtime).2;
        assert!(
            runtime.dispatch_json(&explorer_event(event, payload)),
            "{step}"
        );
        let (open, tool, after, side) = called(&runtime);
        assert!(
            after > before,
            "{step} is a call with the lane already open"
        );
        assert_eq!(
            (open, tool, side),
            (true, Some(Tool::Project), LaneCallSide::View),
            "{step}"
        );
        assert!(runtime.snapshot.ui_state.detail.shown, "{step}");
    }
    assert!(called(&runtime).2 > tool_call);
}

/// PRD lane-two-columns B8, B19: a call records which column it brought
/// forward, so a reveal into the Explorer the tool column already shows is
/// read as the tool column's, not guessed from a tool that did not change.
#[test]
fn a_lane_call_records_the_column_it_brought_forward() {
    let (mut runtime, checkout_id, directory) = views_runtime("lane-call-side");
    files(&directory, &["notes.md"]);
    let side = |runtime: &Runtime| {
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        (view.lane_called, view.lane_called_side)
    };
    use crate::model::LaneCallSide;
    layout(&mut runtime, serde_json::json!({"tool": "explorer"}));
    let (first, tool_side) = side(&runtime);
    assert_eq!(tool_side, LaneCallSide::Tool);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let (second, view_side) = side(&runtime);
    assert!(second > first);
    assert_eq!(
        view_side,
        LaneCallSide::View,
        "an open is the viewing column's"
    );
    layout(
        &mut runtime,
        serde_json::json!({"reveal": directory.join("notes.md").to_string_lossy()}),
    );
    let (third, reveal_side) = side(&runtime);
    assert!(
        third > second,
        "a reveal into the shown Explorer is still a call"
    );
    assert_eq!(reveal_side, LaneCallSide::Tool, "and the tool column's");
    layout(&mut runtime, serde_json::json!({"documents": true}));
    assert_eq!(side(&runtime).1, LaneCallSide::View);
    // A folder a terminal link names shows in the Explorer the tool column
    // already holds: the tool column's call again (B8).
    std::fs::create_dir_all(directory.join("docs")).expect("fixture folder");
    runtime.dispatch_json(&reveal_event(
        "workspace:order",
        &checkout_id,
        &directory.join("docs"),
        true,
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let (fourth, folder_side) = side(&runtime);
    assert!(fourth > third, "a folder link is a call");
    assert_eq!(folder_side, LaneCallSide::Tool, "and the tool column's");
    layout(&mut runtime, serde_json::json!({"open": false}));
    layout(&mut runtime, serde_json::json!({"open": true}));
    assert_eq!(
        side(&runtime).1,
        LaneCallSide::View,
        "⌘E brings the viewing column forward"
    );
}

/// A page's own new tab (a `target=_blank` link, an image a page wraps in
/// one) opens beside that page, so the page that asked stays in view: in a
/// new area to its right the first time, then in that area, and never a
/// third area. An address already shown is focused there, and with the page
/// gone a tab opens where any page would: the area in use.
#[test]
fn a_pages_new_tab_opens_beside_it_and_keeps_it_in_view() {
    let (mut runtime, _, _directory) = views_runtime("browser-beside");
    layout(&mut runtime, serde_json::json!({"tool": null}));
    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "https://a.test/pr"})
    ));
    let [opener] = browser_displays(&mut runtime).try_into().expect("one page");
    let shown = |runtime: &mut Runtime| {
        areas(&tree(runtime))
            .into_iter()
            .map(|(_, active, displays)| {
                let urls = displays
                    .iter()
                    .filter_map(|display| display.url.clone())
                    .collect::<Vec<_>>();
                let active = displays
                    .iter()
                    .find(|display| Some(&display.id) == active.as_ref())
                    .and_then(|display| display.url.clone());
                (urls, active)
            })
            .collect::<Vec<_>>()
    };

    for url in ["https://a.test/image.png", "https://a.test/other.png"] {
        assert!(browser_open(
            &mut runtime,
            serde_json::json!({"url": url, "beside_display": opener.id})
        ));
        assert_eq!(runtime.snapshot.status.last_error, None);
    }
    assert_eq!(
        shown(&mut runtime),
        vec![
            (
                vec!["https://a.test/pr".to_owned()],
                Some("https://a.test/pr".to_owned())
            ),
            (
                vec![
                    "https://a.test/image.png".to_owned(),
                    "https://a.test/other.png".to_owned()
                ],
                Some("https://a.test/other.png".to_owned())
            ),
        ],
        "the page stays in view and its tabs gather beside it"
    );

    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "https://a.test/image.png", "beside_display": opener.id})
    ));
    assert_eq!(
        shown(&mut runtime)[1],
        (
            vec![
                "https://a.test/image.png".to_owned(),
                "https://a.test/other.png".to_owned()
            ],
            Some("https://a.test/image.png".to_owned())
        ),
        "an address already shown is focused, not opened twice"
    );

    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "https://a.test/third.png", "beside_display": "closed-page"})
    ));
    let after = shown(&mut runtime);
    assert_eq!(after.len(), 2, "a gone page adds no area: {after:?}");
    assert_eq!(
        after[1].1.as_deref(),
        Some("https://a.test/third.png"),
        "it opens in the area in use"
    );
}

/// A page opens in the Workspace of the pane that asked, in its area in
/// use, and opens the side panel; the same address again shows that
/// page and loads it again rather than opening a second one. A request that
/// names itself reads its answer in `status.browser_opens`.
#[test]
fn a_page_opens_in_the_workspace_of_the_pane_that_asked_and_once_per_address() {
    let (mut runtime, _, directory) = views_runtime("browser-open");
    layout(&mut runtime, serde_json::json!({"tool": null}));
    let pane = with_pane(&mut runtime, &directory);
    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "https://a.test/docs?x#y", "pane_id": pane, "request_id": "r1"}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    let opened = receipt(&runtime, "r1");
    let [page] = browser_displays(&mut runtime).try_into().expect("one page");
    let checkout = directory.to_string_lossy().into_owned();
    assert_eq!(
        (
            opened.ok,
            opened.device_id.as_deref(),
            opened.path.as_deref(),
            opened.display_id.as_deref()
        ),
        (
            true,
            Some(crate::node::TEST_NODE),
            Some(checkout.as_str()),
            Some(page.id.as_str())
        )
    );
    assert_eq!(
        (
            page.label.as_str(),
            page.path.as_str(),
            page.url.as_deref(),
            page.state,
            page.tab_id.as_deref()
        ),
        (
            "a.test",
            "",
            Some("https://a.test/docs?x#y"),
            ViewDisplayState::Open,
            None
        )
    );
    assert!(page.load > 0);
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(view.open && view.documents);

    // The front Workspace, named by nothing, shows the page again.
    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "https://a.test/docs?x#y"})
    ));
    let [again] = browser_displays(&mut runtime)
        .try_into()
        .expect("still one page");
    assert_eq!(again.id, page.id);
    assert!(again.load > page.load, "loaded again");

    // Refusals change nothing and say why.
    for (request, mut payload) in [
        ("r2", serde_json::json!({"url": "javascript:alert(1)"})),
        (
            "r3",
            serde_json::json!({"url": "https://b.test/", "pane_id": "nowhere:p9"}),
        ),
        (
            "r4",
            serde_json::json!({"url": "https://b.test/", "workspace": {"device_id": crate::node::TEST_NODE, "path": "/not/a/checkout"}}),
        ),
        (
            "r5",
            serde_json::json!({"url": "file:///etc/hosts", "workspace": {"device_id": crate::node::TEST_NODE, "path": checkout}}),
        ),
    ] {
        payload["request_id"] = serde_json::json!(request);
        browser_open(&mut runtime, payload);
        let refused = receipt(&runtime, request);
        if request == "r5" {
            // A file of this Mac in a Workspace on this Mac opens: the
            // daemon's boundary, not the core, keeps it inside a checkout.
            assert!(refused.ok, "{refused:?}");
            continue;
        }
        assert!(
            !refused.ok && refused.message.is_some(),
            "{request}: {refused:?}"
        );
        assert_eq!(last_error_kind(&runtime), Some("browser.open_refused"));
        assert_eq!(browser_displays(&mut runtime).len(), 1, "{request}");
    }

    // Receipts are bounded: the oldest go first.
    for n in 0..10 {
        browser_open(
            &mut runtime,
            serde_json::json!({"url": "https://a.test/docs?x#y", "request_id": format!("again-{n}")}),
        );
    }
    let kept: Vec<&str> = runtime
        .snapshot
        .status
        .browser_opens
        .iter()
        .map(|receipt| receipt.request_id.as_str())
        .collect();
    assert_eq!(kept.len(), 8);
    assert_eq!((kept[0], kept[7]), ("again-2", "again-9"));
}

/// A terminal HTML click selects its owning checkout and page in one event;
/// a background browser open keeps the checkout the operator was using.
#[test]
fn a_browser_open_reveals_its_checkout_only_when_asked() {
    let (mut runtime, _, directory) = views_runtime("browser-reveal");
    with_pane(&mut runtime, &directory);
    let (other, other_id) = second_checkout(&mut runtime, &directory);
    let url = "https://example.test/terminal-report";
    let workspace =
        serde_json::json!({"device_id": crate::node::TEST_NODE, "path": other.to_string_lossy()});
    let before = runtime.snapshot.navigator.focused_checkout_id.clone();
    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": url, "workspace": workspace})
    ));
    assert_eq!(runtime.snapshot.navigator.focused_checkout_id, before);
    assert!(browser_displays(&mut runtime).is_empty());

    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": url, "workspace": workspace, "reveal": true})
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(other_id.as_str())
    );
    let [page] = browser_displays(&mut runtime)
        .try_into()
        .expect("the revealed page");
    assert_eq!(page.url.as_deref(), Some(url));
    assert!(runtime.snapshot.workspace_view.as_ref().unwrap().documents);

    let before = runtime.snapshot.navigator.focused_checkout_id.clone();
    assert!(browser_open(
        &mut runtime,
        serde_json::json!({"url": "javascript:alert(1)", "workspace": workspace, "reveal": true})
    ));
    assert_eq!(runtime.snapshot.navigator.focused_checkout_id, before);
    assert_eq!(browser_displays(&mut runtime).len(), 1);
}

/// The page's own navigation is recorded, not loaded; the toolbar's address
/// is loaded. Neither touches a document display.
#[test]
fn a_page_records_where_it_went_and_the_toolbar_loads_what_was_typed() {
    let (mut runtime, checkout_id, directory) = views_runtime("browser-state");
    files(&directory, &["a.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    browser_open(&mut runtime, serde_json::json!({"url": "https://a.test/"}));
    let [page] = browser_displays(&mut runtime).try_into().expect("one page");
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});

    assert!(runtime.dispatch_json(&explorer_event(
        "browser_state",
        serde_json::json!({"workspace": workspace, "display_id": page.id, "url": "https://a.test/next", "title": " Next\tpage "}),
    )));
    let [moved] = browser_displays(&mut runtime).try_into().expect("one page");
    assert_eq!(
        (
            moved.url.as_deref(),
            moved.title.as_deref(),
            moved.label.as_str(),
            moved.load
        ),
        (
            Some("https://a.test/next"),
            Some("Next page"),
            "Next page",
            page.load
        )
    );
    // The same report again changes nothing; one the page may not hold is dropped.
    assert!(!runtime.dispatch_json(&explorer_event(
        "browser_state",
        serde_json::json!({"workspace": workspace, "display_id": page.id, "url": "https://a.test/next", "title": "Next page"}),
    )));
    assert!(!runtime.dispatch_json(&explorer_event(
        "browser_state",
        serde_json::json!({"workspace": workspace, "display_id": page.id, "url": "chrome://settings", "title": ""}),
    )));

    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "navigate", "display_id": page.id, "url": "http://localhost:3000/"}),
    ));
    let [typed] = browser_displays(&mut runtime).try_into().expect("one page");
    assert_eq!(
        (
            typed.url.as_deref(),
            typed.title.as_deref(),
            typed.label.as_str()
        ),
        (Some("http://localhost:3000/"), None, "localhost:3000")
    );
    assert!(typed.load > moved.load);

    let file = display(&mut runtime, 0, "a.md");
    act(
        &mut runtime,
        serde_json::json!({"action": "navigate", "display_id": file, "url": "https://a.test/"}),
    );
    assert_eq!(
        last_error_kind(&runtime),
        Some("view_layout.unknown_display")
    );
    assert_eq!(labels(&mut runtime), vec![vec!["a.md", "localhost:3000"]]);
}

/// A page is part of the tree the file keeps: a restart shows it again at
/// its last address with nothing to read back, the reconcile never takes it
/// for a closed document, and Close removes it like any view.
#[test]
fn a_page_survives_a_restart_and_closes_like_any_view() {
    let (runtime, checkout_id, directory) = strip_checkout("browser-restart");
    let (_views, state) = views_path("browser-restart");
    let mut runtime = with_views(runtime, &state);
    files(&directory, &["a.md", "report.html"]);
    let report = format!("file://{}", directory.join("report.html").to_string_lossy());
    browser_open(&mut runtime, serde_json::json!({"url": report}));
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    let [page] = browser_displays(&mut runtime).try_into().expect("one page");
    assert_eq!(page.label, "report.html");
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    runtime.dispatch_json(&explorer_event(
        "browser_state",
        serde_json::json!({"workspace": workspace, "display_id": page.id, "url": report, "title": "Report"}),
    ));
    drop(runtime);

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views(restarted, &state);
    assert_eq!(labels(&mut restarted), vec![vec!["Report", "a.md"]]);
    let [back] = browser_displays(&mut restarted)
        .try_into()
        .expect("one page");
    assert_eq!(
        (back.id.as_str(), back.url.as_deref(), back.load, back.state),
        (
            page.id.as_str(),
            Some(report.as_str()),
            0,
            ViewDisplayState::Open
        )
    );
    assert!(
        !restarted
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.path.is_empty()),
        "a page reads no document back"
    );

    assert!(act(
        &mut restarted,
        serde_json::json!({"action": "close", "display_id": back.id}),
    ));
    assert!(browser_displays(&mut restarted).is_empty());
    assert_eq!(labels(&mut restarted), vec![vec!["a.md"]]);
}

/// Recent navigation: a `focus_checkout` naming a display of another
/// Workspace brings that checkout forward on the display in one event, the
/// View area shown even when only Agents showed there; a display the
/// Workspace does not hold refuses the event and nothing moves.
#[test]
fn a_checkout_focus_naming_a_display_brings_it_forward_on_that_display() {
    let (runtime, checkout_id, directory) = strip_checkout("view-focus-display");
    let mut runtime = runtime;
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, "view-focus-display");
    files(&directory, &["a.md", "b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("b.md"),
        false,
        false,
    );
    let a = display(&mut runtime, 0, "a.md");
    layout(&mut runtime, serde_json::json!({"tool": null}));
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:other", "checkout_id": other_checkout}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);

    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:order", "checkout_id": checkout_id, "display_id": "d-missing"}),
    ));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("view_layout.unknown_display")
    );
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(other_checkout.as_str()),
        "a refused display moves no checkout"
    );

    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": "workspace:order", "checkout_id": checkout_id, "display_id": a}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(checkout_id.as_str())
    );
    runtime.sync_workspace_view();
    let view = runtime
        .snapshot
        .workspace_view
        .clone()
        .expect("a front Workspace");
    assert_eq!(view.path, directory.to_string_lossy());
    assert!(view.open && view.documents, "the documents open again");
    let (_, active, _) = areas(&view.layout).remove(0);
    assert_eq!(active.as_deref(), Some(a.as_str()));
    drop(other);
}

#[test]
fn empty_browser_tabs_are_distinct_replace_in_place_and_close_without_recovery() {
    let (mut runtime, checkout, directory) = views_runtime("new-tab");
    let path = directory.join("a.txt");
    std::fs::write(&path, "hello").unwrap();
    browser_open(&mut runtime, serde_json::json!({"url": ""}));
    let first = browser_displays(&mut runtime)[0].clone();
    assert_eq!(first.label, "New tab");
    assert_eq!(first.url, None);
    assert!(
        runtime.changes_request().is_some(),
        "new tab needs Git even with tools hidden"
    );
    browser_open(&mut runtime, serde_json::json!({"url": ""}));
    let pages = browser_displays(&mut runtime);
    assert_eq!(pages.len(), 2);
    let second = pages[1].id.clone();
    let before = runtime.snapshot.recent_closed.clone();
    act(
        &mut runtime,
        serde_json::json!({"action":"close", "display_id": second}),
    );
    assert_eq!(runtime.snapshot.recent_closed, before);
    open(&mut runtime, &checkout, &path, true, false);
    let layout = tree(&mut runtime);
    let displays = &areas(&layout)[0].2;
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].id, first.id);
    assert_eq!(displays[0].kind, crate::view_layout::DisplayKind::File);
    assert!(!displays[0].preview);
    assert_eq!(runtime.snapshot.recent_closed, before);
}

#[test]
fn empty_browser_open_targets_the_named_area_and_navigation_keeps_its_id() {
    let (mut runtime, checkout, directory) = views_runtime("new-tab-area");
    let file = directory.join("a.txt");
    std::fs::write(&file, "hello").unwrap();
    open(&mut runtime, &checkout, &file, false, false);
    open(&mut runtime, &checkout, &file, false, true);
    let layout = tree(&mut runtime);
    let original = areas(&layout)
        .into_iter()
        .find(|(id, _, _)| *id != layout.active_area)
        .unwrap()
        .0;
    browser_open(
        &mut runtime,
        serde_json::json!({"url":"", "area_id":original}),
    );
    let page = browser_displays(&mut runtime)[0].clone();
    assert_eq!(tree(&mut runtime).active_area, original);
    act(
        &mut runtime,
        serde_json::json!({"action":"navigate", "display_id":page.id, "url":"https://example.com"}),
    );
    let navigated = browser_displays(&mut runtime)[0].clone();
    assert_eq!(navigated.id, page.id);
    assert_eq!(navigated.url.as_deref(), Some("https://example.com"));
}

/// PRD right-lane D-02, D-13: a reveal of a file in another checkout calls
/// that checkout's lane once it settles there and leaves the Workspace that
/// was in front alone, so its lane is never opened by it.
#[test]
fn a_reveal_calls_the_lane_of_its_own_checkout() {
    let (mut runtime, checkout_id, directory) = strip_checkout("view-reveal-own");
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, "view-reveal-own");
    files(&other, &["b.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let calls = |runtime: &Runtime| {
        let store = runtime.workspace_views.as_ref().unwrap();
        let mut numbered: Vec<_> = store
            .lane_called
            .iter()
            .map(|((_, path), (number, _))| (path.clone(), *number))
            .collect();
        numbered.sort();
        (store.lane_calls, numbered)
    };
    let (count, first_call) = calls(&runtime);

    runtime.dispatch_json(&explorer_event(
        "reveal_path",
        serde_json::json!({
            "path": other.join("b.md"), "workspace_id": "workspace:other",
            "checkout_id": other_checkout, "is_directory": false,
        }),
    ));
    assert!(runtime.snapshot.status.last_error.is_none());
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.path, other.to_string_lossy());
    assert_eq!(
        (view.open, view.documents, view.tool),
        (true, true, None),
        "the reveal shows the document there and opens no tool (B16, lane-two-columns B6)"
    );
    assert_eq!(
        view.lane_called,
        count + 1,
        "the call is the revealed Workspace's"
    );
    let (_, numbered) = calls(&runtime);
    let front_before: Vec<_> = numbered
        .iter()
        .filter(|(path, _)| *path == directory.to_string_lossy())
        .cloned()
        .collect();
    let expected: Vec<_> = first_call
        .iter()
        .filter(|(path, _)| *path == directory.to_string_lossy())
        .cloned()
        .collect();
    assert_eq!(
        front_before, expected,
        "the Workspace in front before is not called"
    );
}

/// The Workspace in front as a `view_layout` or `file_open` names it.
fn front_view(runtime: &mut Runtime) -> serde_json::Value {
    runtime.sync_workspace_view();
    let view = runtime
        .snapshot
        .workspace_view
        .as_ref()
        .expect("a front Workspace");
    serde_json::json!({"device_id": view.device_id, "path": view.path})
}

/// A terminal link's `file_open` for a file of the second checkout, shown
/// in the Workspace `view`.
fn open_linked(runtime: &mut Runtime, path: &Path, other_checkout: &str, view: serde_json::Value) {
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": path.to_string_lossy(),
            "workspace_id": "workspace:other",
            "checkout_id": other_checkout,
            "view": view,
        }),
    ));
}

fn focus_checkout(runtime: &mut Runtime, workspace_id: &str, checkout_id: &str) {
    runtime.dispatch_json(&explorer_event(
        "focus_checkout",
        serde_json::json!({"workspace_id": workspace_id, "checkout_id": checkout_id}),
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// A terminal link to a file of another registered checkout opens it in the
/// View areas in front as an ordinary tab, and no checkout comes forward:
/// the Explorer keeps its selection, the file stays its own checkout's
/// document, and that checkout's View adds no display of it. Its own View
/// can show it too, and closing the view in front keeps the document there.
/// A link that names a Workspace not in front opens nothing.
#[test]
fn a_file_of_another_checkout_opens_in_the_views_in_front_and_moves_no_checkout() {
    let (mut runtime, checkout_id, directory) = strip_checkout("view-link-elsewhere");
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, "view-link-elsewhere");
    files(&other, &["shot.md"]);
    let shot = other.join("shot.md");
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let selected = runtime.snapshot.ui_state.selected_path.clone();
    let front = front_view(&mut runtime);

    open_linked(&mut runtime, &shot, &other_checkout, front.clone());
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(checkout_id.as_str()),
        "no checkout comes forward"
    );
    assert_eq!(front_view(&mut runtime), front);
    assert_eq!(labels(&mut runtime), vec![vec!["notes.md", "shot.md"]]);
    assert_eq!(active_label(&runtime).as_deref(), Some("shot.md"));
    assert!(runtime.snapshot.workspace_view.as_ref().unwrap().documents);
    assert_eq!(runtime.snapshot.ui_state.selected_path, selected);
    let tab = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.label == "shot.md")
        .unwrap();
    assert_eq!(
        (
            tab.workspace_id.as_str(),
            tab.checkout_id.as_str(),
            tab.preview
        ),
        ("workspace:other", other_checkout.as_str(), false),
        "an ordinary tab of the file's own checkout"
    );
    let shot_tab = tab.id.clone();

    focus_checkout(&mut runtime, "workspace:other", &other_checkout);
    let elsewhere = front_view(&mut runtime);
    assert!(
        labels(&mut runtime).concat().is_empty(),
        "the file's own checkout adds no view of it"
    );
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({
            "path": shot.to_string_lossy(),
            "workspace_id": "workspace:other",
            "checkout_id": other_checkout,
        }),
    ));
    assert_eq!(labels(&mut runtime), vec![vec!["shot.md"]]);
    focus_checkout(&mut runtime, "workspace:order", &checkout_id);
    let shown_here = display(&mut runtime, 0, "shot.md");
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": shown_here}),
    ));
    assert_eq!(labels(&mut runtime), vec![vec!["notes.md"]]);
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == shot_tab),
        "its own checkout's view still shows the document"
    );

    let before = tree(&mut runtime);
    open_linked(&mut runtime, &shot, &other_checkout, elsewhere);
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("file.invalid_context")
    );
    assert_eq!(tree(&mut runtime), before);
    assert_eq!(
        runtime.snapshot.navigator.focused_checkout_id.as_deref(),
        Some(checkout_id.as_str())
    );
}

/// A view a terminal link opened for another checkout's file comes back
/// after a restart, read from that checkout, and that checkout's View still
/// adds no display of it.
#[test]
fn a_linked_file_of_another_checkout_comes_back_after_a_restart() {
    let (mut runtime, checkout_id, directory) = strip_checkout("view-link-restart");
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let (_views, state) = views_path("view-link-restart");
    let mut runtime = with_views(runtime, &state);
    // The checkout folders outlive the runtime: the test restarts on them.
    let _checkouts = hold_dirs(&mut runtime);
    files(&other, &["shot.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("notes.md"),
        false,
        false,
    );
    let front = front_view(&mut runtime);
    open_linked(&mut runtime, &other.join("shot.md"), &other_checkout, front);
    assert_eq!(runtime.snapshot.status.last_error, None);
    drop(runtime);

    let (mut restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    assert_eq!(
        second_checkout(&mut restarted, &directory).1,
        other_checkout
    );
    let mut restarted = with_views(restarted, &state);
    let states: Vec<_> = areas(&tree(&mut restarted))
        .into_iter()
        .flat_map(|(_, _, displays)| displays)
        .map(|display| (display.label, display.state))
        .collect();
    assert_eq!(
        states,
        vec![
            ("notes.md".to_owned(), ViewDisplayState::Open),
            ("shot.md".to_owned(), ViewDisplayState::Open),
        ]
    );
    let tab = restarted
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.label == "shot.md")
        .unwrap();
    assert_eq!(
        tab.checkout_id, other_checkout,
        "read from its own checkout"
    );
    focus_checkout(&mut restarted, "workspace:other", &other_checkout);
    assert!(labels(&mut restarted).concat().is_empty());
}

// PRD outside-path-in-hide: a file outside every checkout that the operator
// clicked in the desktop window shows as an external display. The core holds
// the desktop host's grant id, the file's name and its viewer, never a path,
// and never stores it.

const GRANT_A: &str = "0123456789abcdef0123456789abcdef";
const GRANT_B: &str = "fedcba9876543210fedcba9876543210";

fn external_open(runtime: &mut Runtime, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event("external_open", payload))
}

fn external_displays(runtime: &mut Runtime) -> Vec<ViewDisplaySnapshot> {
    areas(&tree(runtime))
        .into_iter()
        .flat_map(|(_, _, displays)| displays)
        .filter(|display| display.kind == crate::view_layout::DisplayKind::External)
        .collect()
}

/// B1, B10, B12, B22: a granted file opens as one tab named by its file in
/// the Workspace in front, holding no path; the same grant again focuses that
/// tab and asks for a new read. The log of the open names the grant, its
/// kind and the outcome, and nothing of the file.
#[test]
fn an_outside_file_opens_once_per_grant_and_its_log_names_no_file() {
    let (mut runtime, checkout_id, directory) = views_runtime("external-open");
    layout(&mut runtime, serde_json::json!({"tool": null}));
    files(&directory, &["a.md"]);
    open(
        &mut runtime,
        &checkout_id,
        &directory.join("a.md"),
        false,
        false,
    );
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    let (opened, records) = crate::diagnostics::capture(|| {
        external_open(
            &mut runtime,
            serde_json::json!({"grant": GRANT_A, "name": "shot.png", "kind": "image", "workspace": workspace, "request_id": "x1"}),
        )
    });
    assert!(opened);
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert!(receipt(&runtime, "x1").ok);
    let [shown] = external_displays(&mut runtime)
        .try_into()
        .expect("one outside file");
    assert_eq!(
        (
            shown.label.as_str(),
            shown.path.as_str(),
            shown.url.as_deref(),
            shown.state,
            shown.tab_id.as_deref()
        ),
        ("shot.png", "", None, ViewDisplayState::Open, None)
    );
    let file = shown.external.clone().expect("its grant");
    assert_eq!(
        (file.grant.as_str(), file.kind),
        (GRANT_A, crate::view_layout::ExternalKind::Image)
    );
    let [record] = records
        .iter()
        .filter(|record| record["kind"] == "external.open")
        .collect::<Vec<_>>()
        .try_into()
        .expect("one open record");
    assert_eq!(
        record,
        &serde_json::json!({"component": "view_areas", "kind": "external.open", "grant": GRANT_A, "file_kind": "image", "outcome": "placed"})
    );
    assert!(
        !records
            .iter()
            .any(|record| record.to_string().contains("shot.png")),
        "{records:?}"
    );
    assert_eq!(runtime.snapshot.external_grants, vec![GRANT_A.to_owned()]);
    assert!(
        runtime.snapshot.browser_views.is_empty(),
        "an image is not a page"
    );
    // Pages are listed for the checkouts of a live Herdr.
    runtime.snapshot.status.herdr.state = "connected".to_owned();
    assert!(external_open(
        &mut runtime,
        serde_json::json!({"grant": GRANT_B, "name": "preview.html", "kind": "html", "workspace": workspace}),
    ));
    let page = external_displays(&mut runtime)
        .into_iter()
        .find(|display| display.label == "preview.html")
        .expect("the HTML file");
    assert_eq!(
        runtime
            .snapshot
            .browser_views
            .iter()
            .map(|row| row.view_id.as_str())
            .collect::<Vec<_>>(),
        vec![page.id.as_str()],
        "an HTML file is a page the host keeps alive while hidden"
    );
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": page.id})
    ));

    // The checkout's own file comes forward again; a second click on the same
    // outside file brings its one tab back and reads it again.
    let a = display(&mut runtime, 0, "a.md");
    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "focus", "display_id": a})
    ));
    assert!(external_open(
        &mut runtime,
        serde_json::json!({"grant": GRANT_A, "name": "shot.png", "kind": "image", "workspace": workspace}),
    ));
    let [again] = external_displays(&mut runtime)
        .try_into()
        .expect("still one tab");
    assert_eq!(again.id, shown.id);
    assert!(again.load > shown.load, "a second click reads it again");
    assert_eq!(
        areas(&tree(&mut runtime))[0].1.as_deref(),
        Some(shown.id.as_str())
    );
    assert_eq!(labels(&mut runtime), vec![vec!["a.md", "shot.png"]]);
}

/// A grant that is not the host's shape, or a name that is a path, is
/// refused and opens nothing.
#[test]
fn an_outside_file_needs_a_host_grant_and_a_plain_name() {
    let (mut runtime, _, directory) = views_runtime("external-refused");
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    for (grant, name) in [
        ("not-a-grant", "a.txt"),
        (GRANT_A, "/tmp/a.txt"),
        (GRANT_A, ""),
    ] {
        assert!(external_open(
            &mut runtime,
            serde_json::json!({"grant": grant, "name": name, "kind": "text", "workspace": workspace}),
        ));
        assert_eq!(
            runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("external.open_refused"),
            "{grant} {name}"
        );
        assert!(external_displays(&mut runtime).is_empty());
    }
}

/// B3, B14, B15: an outside HTML file is a page the host keeps alive; the
/// grants list names every outside tab, so closing one ends its grant; the
/// stored file never carries one, and a restart shows none.
#[test]
fn an_outside_file_is_never_stored_and_a_close_ends_its_grant() {
    let (runtime, _, directory) = strip_checkout("external-restart");
    let (_views, state) = views_path("external-restart");
    let mut runtime = with_views(runtime, &state);
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    for (grant, name, kind) in [
        (GRANT_A, "preview.html", "html"),
        (GRANT_B, "notes.txt", "text"),
    ] {
        assert!(external_open(
            &mut runtime,
            serde_json::json!({"grant": grant, "name": name, "kind": kind, "workspace": workspace}),
        ));
    }
    let shown = external_displays(&mut runtime);
    assert_eq!(shown.len(), 2);
    let mut grants = runtime.snapshot.external_grants.clone();
    grants.sort();
    assert_eq!(grants, vec![GRANT_A.to_owned(), GRANT_B.to_owned()]);
    let stored = std::fs::read_to_string(&state).expect("the views file");
    assert!(
        !stored.contains(GRANT_A)
            && !stored.contains("notes.txt")
            && !stored.contains("\"external\""),
        "{stored}"
    );

    assert!(act(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": shown[1].id})
    ));
    assert_eq!(runtime.snapshot.external_grants, vec![GRANT_A.to_owned()]);
    drop(runtime);

    let (restarted, _) = tab_order_runtime(&directory.to_string_lossy());
    let mut restarted = with_views(restarted, &state);
    assert!(external_displays(&mut restarted).is_empty());
    assert!(restarted.snapshot.external_grants.is_empty());
}

/// D-07: the desktop host's grants when its window attaches. A tab whose
/// grant the host no longer holds goes, in every Workspace; the rest stay.
#[test]
fn a_tab_whose_grant_the_host_lost_goes_when_the_host_says_so() {
    let (mut runtime, _, directory) = views_runtime("external-grants");
    let workspace = serde_json::json!({"device_id": crate::node::TEST_NODE, "path": directory.to_string_lossy()});
    for (grant, name) in [(GRANT_A, "a.png"), (GRANT_B, "b.png")] {
        assert!(external_open(
            &mut runtime,
            serde_json::json!({"grant": grant, "name": name, "kind": "image", "workspace": workspace}),
        ));
    }
    assert!(
        !runtime.dispatch_json(&explorer_event(
            "external_grants",
            serde_json::json!({"live": [GRANT_A, GRANT_B]}),
        )),
        "both grants live: nothing changes"
    );
    assert!(runtime.dispatch_json(&explorer_event(
        "external_grants",
        serde_json::json!({"live": [GRANT_B]}),
    )));
    assert_eq!(
        external_displays(&mut runtime)
            .iter()
            .map(|display| display.label.as_str())
            .collect::<Vec<_>>(),
        vec!["b.png"]
    );
    assert!(runtime.dispatch_json(&explorer_event(
        "external_grants",
        serde_json::json!({"live": []})
    )));
    assert!(external_displays(&mut runtime).is_empty());
}
