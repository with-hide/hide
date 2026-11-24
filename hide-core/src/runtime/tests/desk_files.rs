use super::workspace_view::{second_checkout, with_new_views};
use super::*;
use crate::model::ViewNodeSnapshot;

// PRD desk-2-ui D-22, B19, B28, B36: a file Desk's tab panel opens is its
// checkout's own document, opened by the one `desk_file` event hided sends
// for a window's message, which brings no checkout forward and puts no
// display in any Workspace's View; a Workspace closing its last display of
// it leaves it open while Desk holds it, and Desk letting it go closes it
// unless a View shows it, or hands an unsaved one to its checkout's View.

fn desk_runtime(name: &str) -> (Runtime, String, PathBuf) {
    let (runtime, checkout_id, directory) = strip_checkout(name);
    let mut runtime = with_new_views(runtime, name);
    runtime.start_test_desk();
    (runtime, checkout_id, directory)
}

/// `desk_runtime` with a second checkout, not in front, whose root is open
/// as every registered checkout's is.
fn desk_runtime_with_other(name: &str) -> (Runtime, PathBuf, String) {
    let (mut runtime, _, directory) = strip_checkout(name);
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, name);
    runtime.start_test_desk();
    (runtime, other, other_checkout)
}

fn desk_file(runtime: &mut Runtime, payload: serde_json::Value) {
    runtime.dispatch_json(&explorer_event("desk_file", payload));
}

fn open_payload(workspace_id: &str, checkout_id: &str, path: &Path) -> serde_json::Value {
    serde_json::json!({"open": [{"workspace_id": workspace_id, "checkout_id": checkout_id, "path": path.to_string_lossy()}]})
}

fn close_payload(workspace_id: &str, checkout_id: &str, path: &Path) -> serde_json::Value {
    serde_json::json!({"close": [{"workspace_id": workspace_id, "checkout_id": checkout_id, "path": path.to_string_lossy()}]})
}

fn front(runtime: &Runtime) -> (Option<String>, Option<String>) {
    let navigator = &runtime.snapshot.navigator;
    (
        navigator.focused_workspace_id.clone(),
        navigator.focused_checkout_id.clone(),
    )
}

fn open_tab(runtime: &Runtime, path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.path == path)
        .map(|tab| tab.id.clone())
}

/// Every display path the front Workspace's View shows.
fn shown(runtime: &mut Runtime) -> Vec<String> {
    runtime.sync_workspace_view();
    let Some(view) = runtime.snapshot.workspace_view.clone() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut nodes = vec![view.layout.root];
    while let Some(node) = nodes.pop() {
        match node {
            ViewNodeSnapshot::Area(area) => {
                found.extend(area.displays.into_iter().map(|display| display.path))
            }
            ViewNodeSnapshot::Split(split) => nodes.extend([*split.second, *split.first]),
        }
    }
    found
}

#[test]
fn a_file_desk_opens_brings_no_checkout_forward_and_shows_in_no_view() {
    let (mut runtime, other, other_checkout) = desk_runtime_with_other("desk-file-open");
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");
    let before = front(&runtime);

    desk_file(
        &mut runtime,
        open_payload("workspace:other", &other_checkout, &file),
    );

    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(front(&runtime), before, "the front checkout stays");
    let tab = open_tab(&runtime, &file).expect("the document is open");
    assert!(
        runtime.visible_view_documents().contains(&tab),
        "its text rides the documents section for Desk's file view"
    );
    assert!(
        !shown(&mut runtime).contains(&file.to_string_lossy().into_owned()),
        "no View takes it in"
    );
}

#[test]
fn a_workspace_closing_its_last_display_leaves_a_file_desk_holds_and_desk_letting_go_closes_it() {
    let (mut runtime, checkout_id, directory) = desk_runtime("desk-file-shared");
    let file = directory.join("notes.md");
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({"path": file.to_string_lossy(), "workspace_id": "workspace:order", "checkout_id": checkout_id, "preview": false}),
    ));
    desk_file(
        &mut runtime,
        open_payload("workspace:order", &checkout_id, &file),
    );
    let tab = open_tab(&runtime, &file).expect("one shared document");
    runtime.sync_workspace_view();
    let view = runtime
        .snapshot
        .workspace_view
        .clone()
        .expect("a front Workspace");
    let display = match &view.layout.root {
        ViewNodeSnapshot::Area(area) => area.displays[0].id.clone(),
        ViewNodeSnapshot::Split(_) => panic!("one area"),
    };
    runtime.dispatch_json(&explorer_event(
        "view_layout",
        serde_json::json!({"action": "close", "display_id": display, "workspace": {"device_id": view.device_id, "path": view.path}}),
    ));

    assert_eq!(open_tab(&runtime, &file).as_deref(), Some(tab.as_str()));
    assert!(
        shown(&mut runtime).is_empty(),
        "the View does not take it back"
    );

    desk_file(
        &mut runtime,
        close_payload("workspace:order", &checkout_id, &file),
    );
    assert_eq!(open_tab(&runtime, &file), None);
}

#[test]
fn desk_letting_go_keeps_a_file_a_view_shows_and_hands_unsaved_work_to_its_view() {
    let (mut runtime, checkout_id, directory) = desk_runtime("desk-file-kept");
    let shared = directory.join("notes.md");
    let edited = directory.join("edited.md");
    std::fs::write(&edited, "edited\n").expect("fixture");
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({"path": shared.to_string_lossy(), "workspace_id": "workspace:order", "checkout_id": checkout_id, "preview": false}),
    ));
    for file in [&shared, &edited] {
        desk_file(
            &mut runtime,
            open_payload("workspace:order", &checkout_id, file),
        );
    }
    let draft = open_tab(&runtime, &edited).expect("open");
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"tab_id": draft, "contents_utf8": "changed\n"}),
    ));

    desk_file(
        &mut runtime,
        serde_json::json!({"close": [
            {"workspace_id": "workspace:order", "checkout_id": checkout_id, "path": shared.to_string_lossy()},
            {"workspace_id": "workspace:order", "checkout_id": checkout_id, "path": edited.to_string_lossy()},
        ]}),
    );

    assert!(open_tab(&runtime, &shared).is_some(), "a View shows it");
    assert!(
        open_tab(&runtime, &edited).is_some(),
        "it holds unsaved work"
    );
    assert!(
        shown(&mut runtime).contains(&edited.to_string_lossy().into_owned()),
        "its checkout's View shows the unsaved document Desk let go"
    );
}

/// B28: a file whose checkout is gone is a tab that says why, read again by
/// `desk_file_retry`, never a tab left loading.
#[test]
fn a_file_desk_cannot_open_is_a_tab_that_says_why_and_retry_reads_it_again() {
    let (mut runtime, _, directory) = desk_runtime("desk-file-unavailable");
    let file = directory.join("notes.md");
    desk_file(
        &mut runtime,
        open_payload("workspace:gone", "checkout:gone", &file),
    );
    let reason = |runtime: &Runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.checkout_id == "checkout:gone")
            .and_then(|tab| tab.unavailable_reason.clone())
    };
    assert!(reason(&runtime).is_some(), "the tab says why");
    assert!(runtime.dispatch_json(&explorer_event(
        "desk_file_retry",
        serde_json::json!({"workspace_id": "workspace:gone", "checkout_id": "checkout:gone", "path": file.to_string_lossy()}),
    )));
    assert!(reason(&runtime).is_some(), "still gone, still said");

    desk_file(
        &mut runtime,
        close_payload("workspace:gone", "checkout:gone", &file),
    );
    assert_eq!(reason(&runtime), None, "letting it go closes the tab");
}

#[test]
fn a_file_desk_opens_is_refused_while_desk_is_off() {
    let (runtime, checkout_id, directory) = strip_checkout("desk-file-off");
    let mut runtime = with_new_views(runtime, "desk-file-off");
    let file = directory.join("notes.md");
    desk_file(
        &mut runtime,
        open_payload("workspace:order", &checkout_id, &file),
    );
    assert_eq!(open_tab(&runtime, &file), None);
}

/// B35: Desk's Explorer changes its session's checkout whatever is in front,
/// and the core opens the file it creates in no View, since the window's
/// room opens and holds it; the same change from the Workspace's tree is
/// still refused there.
#[test]
fn desks_explorer_changes_its_sessions_checkout_and_opens_its_created_file_nowhere() {
    let (mut runtime, other, _) = desk_runtime_with_other("desk-explorer");
    let before = front(&runtime);
    let root = other.to_string_lossy().into_owned();

    runtime.dispatch_json(&explorer_event(
        "dir_create",
        serde_json::json!({"root": root, "parent": root, "name": "front-only"}),
    ));
    assert!(
        !other.join("front-only").exists(),
        "the Workspace's tree changes only the checkout in front"
    );

    runtime.dispatch_json(&explorer_event(
        "file_create",
        serde_json::json!({"desk": true, "root": root, "parent": root, "name": "made.md"}),
    ));

    let made = other.join("made.md");
    assert!(made.is_file());
    assert_eq!(front(&runtime), before, "the front checkout stays");
    assert_eq!(
        open_tab(&runtime, &made),
        None,
        "the window's room opens it, not the core"
    );
    assert!(!shown(&mut runtime).contains(&made.to_string_lossy().into_owned()));
}

/// B36: a single press replaces the preview tab's file: the window's one
/// message reaches the core as one event that opens one and lets the other go.
#[test]
fn a_preview_open_lets_the_file_it_replaces_go_in_the_same_event() {
    let (mut runtime, checkout_id, directory) = desk_runtime("desk-file-replace");
    let first = directory.join("notes.md");
    let second = directory.join("next.md");
    std::fs::write(&second, "next\n").expect("fixture");
    desk_file(
        &mut runtime,
        open_payload("workspace:order", &checkout_id, &first),
    );
    desk_file(
        &mut runtime,
        serde_json::json!({
            "open": [{"workspace_id": "workspace:order", "checkout_id": checkout_id, "path": second.to_string_lossy()}],
            "close": [{"workspace_id": "workspace:order", "checkout_id": checkout_id, "path": first.to_string_lossy()}],
        }),
    );
    assert_eq!(open_tab(&runtime, &first), None);
    assert!(open_tab(&runtime, &second).is_some());
}
