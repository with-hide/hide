use super::workspace_view::{second_checkout, with_new_views};
use super::*;
use crate::model::{ViewDisplaySnapshot, ViewNodeSnapshot};
use crate::workspace_control::{Action, ActionPreparation, ActionResult, DESK_CALLER, Refusal};

// PRD desk-own-desk: Desk's own desk is the View areas a Workspace's Agent
// tab keeps, kept by the core in Desk's store. A file there is its own
// checkout's document, opened from Desk without bringing anything forward,
// and the core holds it while a View of Desk's desk shows it; Desk's desk and
// the Workspaces' desks never take in each other's opens (D-04, B15).

fn desk_runtime(name: &str) -> (Runtime, String, PathBuf) {
    let (runtime, checkout_id, directory) = strip_checkout(name);
    let mut runtime = with_new_views(runtime, name);
    runtime.start_test_desk();
    (runtime, checkout_id, directory)
}

/// `desk_runtime` with a second checkout, not in front.
pub(super) fn desk_runtime_with_other(name: &str) -> (Runtime, PathBuf, String) {
    let (mut runtime, _, directory) = strip_checkout(name);
    let (other, other_checkout) = second_checkout(&mut runtime, &directory);
    let mut runtime = with_new_views(runtime, name);
    // This machine's engine is connected, as an agent's file open requires.
    runtime.snapshot.status.engine.state = "connected".into();
    runtime.open_test_desk(4);
    (runtime, other, other_checkout)
}

fn open_on_desk(runtime: &mut Runtime, workspace_id: &str, checkout_id: &str, path: &Path) {
    open_on_desk_as(
        runtime,
        workspace_id,
        checkout_id,
        path,
        serde_json::json!({}),
    );
}

fn open_on_desk_as(
    runtime: &mut Runtime,
    workspace_id: &str,
    checkout_id: &str,
    path: &Path,
    how: serde_json::Value,
) {
    let mut payload = serde_json::json!({
        "kind": "file",
        "workspace_id": workspace_id,
        "checkout_id": checkout_id,
        "path": path.to_string_lossy(),
    });
    if let (Some(payload), Some(how)) = (payload.as_object_mut(), how.as_object()) {
        payload.extend(how.clone());
    }
    runtime.dispatch_json(&explorer_event("desk_view_open", payload));
}

fn desk_layout(runtime: &mut Runtime, action: serde_json::Value) {
    let mut payload = action;
    payload["workspace"] =
        serde_json::json!({"device_id": runtime.node.as_str(), "path": "hide:desk"});
    runtime.dispatch_json(&explorer_event("view_layout", payload));
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

fn displays(node: &ViewNodeSnapshot) -> Vec<ViewDisplaySnapshot> {
    let mut found = Vec::new();
    let mut nodes = vec![node.clone()];
    while let Some(node) = nodes.pop() {
        match node {
            ViewNodeSnapshot::Area(area) => found.extend(area.displays),
            ViewNodeSnapshot::Split(split) => nodes.extend([*split.second, *split.first]),
        }
    }
    found
}

/// Every View of Desk's desk as the snapshot carries it.
pub(super) fn desk_views(runtime: &mut Runtime) -> Vec<ViewDisplaySnapshot> {
    runtime.sync_workspace_view();
    runtime
        .snapshot
        .desk_view
        .as_ref()
        .map(|view| displays(&view.layout.root))
        .unwrap_or_default()
}

/// Every display path the front Workspace's View shows.
fn shown(runtime: &mut Runtime) -> Vec<String> {
    runtime.sync_workspace_view();
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .map(|view| displays(&view.layout.root))
        .unwrap_or_default()
        .into_iter()
        .map(|display| display.path)
        .collect()
}

#[test]
fn a_file_desk_opens_is_on_desks_desk_in_front_and_in_no_workspace_view() {
    let (mut runtime, other, other_checkout) = desk_runtime_with_other("own-desk-open");
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");
    let before = front(&runtime);

    open_on_desk(&mut runtime, "workspace:other", &other_checkout, &file);

    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(front(&runtime), before, "the front checkout stays");
    let views = desk_views(&mut runtime);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].path, file.to_string_lossy());
    assert_eq!(views[0].state, crate::model::ViewDisplayState::Open);
    assert_eq!(
        views[0].origin.as_ref().map(|origin| origin.path.clone()),
        Some(other.to_string_lossy().into_owned()),
        "the View names its checkout"
    );
    let tab = open_tab(&runtime, &file).expect("the document is open");
    assert!(
        runtime.visible_view_documents().contains(&tab),
        "its text rides the documents section"
    );
    assert!(
        !shown(&mut runtime).contains(&file.to_string_lossy().into_owned()),
        "no Workspace's View takes it in (B15)"
    );
}

#[test]
fn opening_the_same_file_again_brings_its_tab_forward_instead_of_a_second() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-again");
    let notes = directory.join("notes.md");
    let next = directory.join("next.md");
    std::fs::write(&next, "next\n").expect("fixture");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &notes);
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &next);
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &notes);

    runtime.sync_workspace_view();
    let view = runtime.snapshot.desk_view.clone().expect("Desk's desk");
    let views = displays(&view.layout.root);
    assert_eq!(views.len(), 2, "one tab per file (B14)");
    let ViewNodeSnapshot::Area(area) = &view.layout.root else {
        panic!("one area");
    };
    let front = views
        .iter()
        .find(|display| Some(&display.id) == area.active.as_ref())
        .expect("a tab in front");
    assert_eq!(front.path, notes.to_string_lossy());
}

#[test]
fn a_workspaces_open_never_lands_on_desks_desk_nor_desks_on_the_workspaces() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-apart");
    let notes = directory.join("notes.md");
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({"path": notes.to_string_lossy(), "workspace_id": "workspace:order", "checkout_id": checkout_id, "preview": false}),
    ));
    assert!(
        desk_views(&mut runtime).is_empty(),
        "B15: terminal mode's open"
    );
    let next = directory.join("next.md");
    std::fs::write(&next, "next\n").expect("fixture");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &next);
    assert_eq!(
        shown(&mut runtime),
        vec![notes.to_string_lossy().into_owned()]
    );
    assert_eq!(desk_views(&mut runtime).len(), 1);
}

#[test]
fn a_workspace_closing_its_last_view_leaves_a_file_desks_desk_shows_and_desks_close_lets_it_go() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-shared");
    let file = directory.join("notes.md");
    runtime.dispatch_json(&explorer_event(
        "file_open",
        serde_json::json!({"path": file.to_string_lossy(), "workspace_id": "workspace:order", "checkout_id": checkout_id, "preview": false}),
    ));
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
    let tab = open_tab(&runtime, &file).expect("one shared document");
    runtime.sync_workspace_view();
    let view = runtime
        .snapshot
        .workspace_view
        .clone()
        .expect("a front Workspace");
    let display = displays(&view.layout.root)[0].id.clone();
    runtime.dispatch_json(&explorer_event(
        "view_layout",
        serde_json::json!({"action": "close", "display_id": display, "workspace": {"device_id": view.device_id, "path": view.path}}),
    ));
    assert_eq!(open_tab(&runtime, &file).as_deref(), Some(tab.as_str()));
    assert!(
        shown(&mut runtime).is_empty(),
        "the View does not take it back"
    );

    let desk = desk_views(&mut runtime)[0].id.clone();
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": desk}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        open_tab(&runtime, &file),
        None,
        "B32: nothing else shows it"
    );
    assert!(desk_views(&mut runtime).is_empty());
}

#[test]
fn closing_desks_last_view_of_unsaved_work_asks_for_save_or_dont_save() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-unsaved");
    let file = directory.join("notes.md");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
    let tab = open_tab(&runtime, &file).expect("open");
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"tab_id": tab, "contents_utf8": "changed\n"}),
    ));
    let desk = desk_views(&mut runtime)[0].id.clone();
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": desk}),
    );
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("view_layout.unsaved")
    );
    assert_eq!(desk_views(&mut runtime).len(), 1, "the tab stays");
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": desk, "discard": true}),
    );
    assert_eq!(open_tab(&runtime, &file), None);
}

#[test]
fn desks_desk_splits_and_resizes_as_a_workspaces_desk_does() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-split");
    let next = directory.join("next.md");
    std::fs::write(&next, "next\n").expect("fixture");
    open_on_desk(
        &mut runtime,
        "workspace:order",
        &checkout_id,
        &directory.join("notes.md"),
    );
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &next);
    runtime.sync_workspace_view();
    let view = runtime.snapshot.desk_view.clone().expect("Desk's desk");
    let ViewNodeSnapshot::Area(area) = &view.layout.root else {
        panic!("one area");
    };
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "split", "display_id": area.displays[1].id, "area_id": area.id, "edge": "right", "request_id": "s-1"}),
    );
    assert_eq!(runtime.snapshot.status.last_error, None);
    runtime.sync_workspace_view();
    let view = runtime.snapshot.desk_view.clone().expect("Desk's desk");
    let ViewNodeSnapshot::Split(split) = &view.layout.root else {
        panic!("two areas after the split (B5)");
    };
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "resize", "split_id": split.id, "ratio": 0.3}),
    );
    runtime.sync_workspace_view();
    let ViewNodeSnapshot::Split(split) = &runtime.snapshot.desk_view.clone().unwrap().layout.root
    else {
        panic!("still split");
    };
    assert!((split.ratio - 0.3).abs() < 1e-6);
}

#[test]
fn past_thirty_two_views_the_one_looked_at_longest_ago_closes() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-cap");
    let files: Vec<PathBuf> = (0..33)
        .map(|n| {
            let file = directory.join(format!("f{n:02}.md"));
            std::fs::write(&file, "x\n").expect("fixture");
            file
        })
        .collect();
    for file in &files {
        open_on_desk(&mut runtime, "workspace:order", &checkout_id, file);
    }
    let paths: Vec<String> = desk_views(&mut runtime)
        .into_iter()
        .map(|display| display.path)
        .collect();
    assert_eq!(paths.len(), crate::view_layout::MAX_VIEW_DISPLAYS);
    assert!(
        !paths.contains(&files[0].to_string_lossy().into_owned()),
        "B28: the oldest went"
    );
    assert!(paths.contains(&files[32].to_string_lossy().into_owned()));
    assert_eq!(open_tab(&runtime, &files[0]), None, "its document with it");
}

#[test]
fn desk_switched_off_lets_its_documents_go_keeps_its_tabs_and_reads_them_back_on() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-off");
    let file = directory.join("notes.md");
    let edited = directory.join("edited.md");
    std::fs::write(&edited, "edited\n").expect("fixture");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &edited);
    let draft = open_tab(&runtime, &edited).expect("open");
    runtime.dispatch_json(&explorer_event(
        "file_draft",
        serde_json::json!({"tab_id": draft, "contents_utf8": "changed\n"}),
    ));

    runtime.dispatch_json(&explorer_event(
        "desk_settings_set",
        serde_json::json!({"enabled": false}),
    ));
    assert_eq!(open_tab(&runtime, &file), None, "B33: let go");
    assert!(
        shown(&mut runtime).contains(&edited.to_string_lossy().into_owned()),
        "the unsaved one goes to its checkout's View"
    );
    assert!(runtime.snapshot.desk_view.is_none(), "no desk while off");
    assert_eq!(
        runtime.desk_stored_for_test().views.display_count(),
        2,
        "the tabs stay"
    );

    runtime.dispatch_json(&explorer_event(
        "desk_settings_set",
        serde_json::json!({"enabled": true}),
    ));
    assert!(open_tab(&runtime, &file).is_some(), "read back");
    let states: Vec<_> = desk_views(&mut runtime)
        .into_iter()
        .map(|display| display.state)
        .collect();
    assert_eq!(states.len(), 2);
}

#[test]
fn desks_desk_is_refused_while_desk_is_off() {
    let (runtime, checkout_id, directory) = strip_checkout("own-desk-refused-off");
    let mut runtime = with_new_views(runtime, "own-desk-refused-off");
    let file = directory.join("notes.md");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
    assert_eq!(open_tab(&runtime, &file), None);
    assert!(runtime.snapshot.desk_view.is_none());
}

#[test]
fn desks_desk_survives_a_restart_through_desks_store() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-store");
    open_on_desk(
        &mut runtime,
        "workspace:order",
        &checkout_id,
        &directory.join("notes.md"),
    );
    let stored = runtime.desk_stored_for_test().clone();
    let bytes = crate::desk::encode_for_test(&stored);
    let read = crate::desk::decode_for_test(&bytes).expect("reads");
    assert_eq!(read.views.display_count(), 1);
    let display = read.views.displays().next().unwrap();
    assert_eq!(
        display.origin.as_ref().map(|origin| origin.path.clone()),
        Some(directory.to_string_lossy().into_owned())
    );
    assert_eq!(display.tab_id, None, "the binding is never stored");
}

// `hide file open` from Desk's agent (D-13): the file is checked inside the
// project Hide shows that holds it, read on that checkout's host as an
// agent's file open is, and goes in front of Desk's desk; it brings no
// checkout forward and shows in no Workspace's View.

fn desk_context() -> crate::workspace_control::Context {
    crate::desk::desk_context(crate::node::TEST_NODE)
}

fn desk_action(runtime: &mut Runtime, id: &str, action: Action) -> Result<ActionResult, Refusal> {
    let expected = desk_context();
    let material = match runtime.workspace_control_prepare_action(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &expected,
        id,
        &action,
    )? {
        ActionPreparation::Cached(cached) => return cached,
        ActionPreparation::Ready => Ok(None),
        ActionPreparation::Read(source) => source.read().map(Some),
    };
    runtime.workspace_control_action(
        crate::node::TEST_NODE,
        DESK_CALLER,
        &expected,
        id,
        action,
        material,
    )
}

pub(super) fn agent_open(
    runtime: &mut Runtime,
    id: &str,
    path: &Path,
) -> Result<ActionResult, Refusal> {
    desk_action(
        runtime,
        id,
        Action::OpenFile {
            path: path.to_string_lossy().into_owned(),
        },
    )
}

#[test]
fn a_file_desks_agent_opens_goes_in_front_of_desks_desk() {
    let (mut runtime, other, _) = desk_runtime_with_other("own-desk-agent");
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");
    let before = front(&runtime);

    let opened = agent_open(&mut runtime, "r-1", &file).expect("Desk opens a file of a project");

    assert_eq!(opened.context, desk_context(), "it answers as Desk");
    assert_eq!(front(&runtime), before, "no checkout comes forward");
    let views = desk_views(&mut runtime);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].path, file.to_string_lossy());
    assert_eq!(
        runtime.snapshot.desk_view.as_ref().map(|view| view.opened),
        Some(1)
    );
    assert!(!shown(&mut runtime).contains(&file.to_string_lossy().into_owned()));

    // The same command again answers the same open.
    let again = agent_open(&mut runtime, "r-1", &file).unwrap();
    assert_eq!(again.view_id, opened.view_id);
    assert_eq!(desk_views(&mut runtime).len(), 1);
}

#[test]
fn an_agent_open_desks_desk_has_no_room_for_is_refused_and_opens_when_asked_again_after_room_is_made()
 {
    let (mut runtime, other, checkout) = desk_runtime_with_other("own-desk-agent-full");
    // Thirty-two Views of unsaved work, none of which may close to make room (B28).
    for n in 0..crate::view_layout::MAX_VIEW_DISPLAYS {
        let file = other.join(format!("f{n:02}.md"));
        std::fs::write(&file, "x\n").expect("fixture");
        open_on_desk(&mut runtime, "workspace:other", &checkout, &file);
        let tab = open_tab(&runtime, &file).expect("open");
        runtime.dispatch_json(&explorer_event(
            "file_draft",
            serde_json::json!({"tab_id": tab, "contents_utf8": "changed\n"}),
        ));
    }
    let file = other.join("plan.md");
    std::fs::write(&file, "plan\n").expect("fixture");

    let refused = agent_open(&mut runtime, "r-full", &file).unwrap_err();

    assert_eq!(refused.reason, "desk_open_refused");
    assert_eq!(
        runtime.snapshot.desk_view.as_ref().map(|view| view.opened),
        Some(0),
        "a refused open is no open the window shows the desk for"
    );
    assert!(
        !desk_views(&mut runtime)
            .iter()
            .any(|view| view.path == file.to_string_lossy())
    );

    let first = desk_views(&mut runtime)[0].id.clone();
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "close", "display_id": first, "discard": true}),
    );
    agent_open(&mut runtime, "r-full", &file).expect("the same request opens once there is room");
    assert!(
        desk_views(&mut runtime)
            .iter()
            .any(|view| view.path == file.to_string_lossy())
    );
}

#[test]
fn desk_is_refused_a_diff_another_action_a_path_outside_every_project_and_a_missing_file() {
    let (mut runtime, other, _) = desk_runtime_with_other("own-desk-agent-refused");
    let file = other.join("a.rs");
    std::fs::write(&file, "a\n").expect("fixture");
    let reason = |result: Result<ActionResult, Refusal>| result.unwrap_err().reason;

    assert_eq!(
        reason(desk_action(
            &mut runtime,
            "diff",
            Action::OpenDiff {
                path: file.to_string_lossy().into_owned()
            }
        )),
        "desk_diff_unsupported"
    );
    assert_eq!(
        reason(desk_action(
            &mut runtime,
            "select",
            Action::Select {
                view_id: "v".into(),
                expected_browser_area: None
            }
        )),
        "desk_has_no_checkout"
    );
    let outside = other.with_file_name("own-desk-agent-refused-outside");
    std::fs::create_dir_all(&outside).unwrap();
    let stray = outside.join("notes.md");
    std::fs::write(&stray, "x\n").unwrap();
    assert_eq!(
        reason(agent_open(&mut runtime, "outside", &stray)),
        "desk_file_outside_project"
    );
    let sibling = PathBuf::from(format!("{}x/a.rs", other.to_string_lossy()));
    assert_eq!(
        reason(agent_open(&mut runtime, "sibling", &sibling)),
        "desk_file_outside_project"
    );
    assert_eq!(
        reason(agent_open(&mut runtime, "missing", &other.join("gone.md"))),
        "file_missing"
    );
    assert!(desk_views(&mut runtime).is_empty(), "nothing refused opens");
}

#[test]
fn desk_opens_nothing_while_its_process_is_not_running() {
    let (mut runtime, other, _) = desk_runtime_with_other("own-desk-agent-stopped");
    let file = other.join("a.md");
    std::fs::write(&file, "a\n").expect("fixture");
    runtime.close_test_desk();
    assert_eq!(
        agent_open(&mut runtime, "stopped", &file)
            .unwrap_err()
            .reason,
        "desk_not_running"
    );
}

/// Desk's Explorer changes its session's checkout whatever is in front, and
/// the core opens the file it creates in no View: the window opens it on
/// Desk's desk as any press does.
#[test]
fn desks_explorer_changes_its_sessions_checkout_and_opens_its_created_file_nowhere() {
    let (mut runtime, other, _) = desk_runtime_with_other("own-desk-explorer");
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
    assert_eq!(open_tab(&runtime, &made), None);
    assert!(desk_views(&mut runtime).is_empty());
}

#[test]
fn a_page_of_desks_desk_is_the_core_machines_and_resolves_to_load_there() {
    let (mut runtime, _, _) = desk_runtime("desk-own-desk-page");
    runtime.snapshot.status.engine.state = "connected".into();
    let node = runtime.node.as_str().to_owned();
    runtime.dispatch_json(&explorer_event(
        "browser_open",
        serde_json::json!({"url": "https://example.com/", "workspace": {"device_id": node, "path": "hide:desk"}}),
    ));
    let page = desk_views(&mut runtime)
        .into_iter()
        .find(|display| display.kind == crate::view_layout::DisplayKind::Browser)
        .expect("the page is on Desk's desk");
    assert_eq!(
        runtime
            .snapshot
            .desk_view
            .as_ref()
            .map(|view| view.device_id.clone()),
        Some(node.clone())
    );
    let load = runtime
        .view_display(&runtime.desk_key(), &page.id)
        .map(|display| display.load)
        .expect("the page's display");
    let source = runtime
        .browser_route_source(&node, "hide:desk", &page.id, load)
        .expect("a page of Desk's desk resolves");
    assert_eq!(
        (source.device_id, source.url),
        (node, "https://example.com/".to_owned())
    );
    assert!(
        shown(&mut runtime).is_empty(),
        "the page is on no Workspace's desk"
    );
}

// A View a session opened, pressed in its `뷰 N` list or on its card (D-12,
// B25, B34): it opens on Desk's desk from the session's own machine and
// checkout, and is seen only once it opened there.

const DEVICE: &str = "remote:desk-device";
const DEVICE_ROOT: &str = "/srv/remote";

/// A device whose one project is `DEVICE_ROOT`, as its engine session lists
/// it, in `state`.
fn device_status(runtime: &mut Runtime, state: &str) {
    let mut project = workspace(
        "remote-workspace",
        "Remote",
        DEVICE_ROOT,
        vec![checkout(
            "remote-workspace",
            "remote-checkout",
            DEVICE_ROOT,
            Some(pane("remote-pane", DEVICE_ROOT)),
        )],
    );
    project.device_id = DEVICE.to_owned();
    project.remote_target_id = Some(DEVICE.to_owned());
    runtime
        .snapshot
        .status
        .remote
        .retain(|remote| remote.target_id != DEVICE);
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: DEVICE.to_owned(),
        state: state.to_owned(),
        message: None,
        session: Some(RemoteSessionSnapshot {
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
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
}

/// A View of `kind` at `at` (an address for a page) the session at `key`
/// opened and the operator has not looked at.
fn session_view(
    runtime: &mut Runtime,
    key: &(String, String),
    kind: crate::view_layout::DisplayKind,
    at: &str,
) -> String {
    let entry = runtime
        .workspace_views
        .as_mut()
        .expect("views")
        .views
        .entry(&key.0, &key.1);
    let layout = &mut entry
        .tray_entry(crate::workspace_views::PENDING_TRAY)
        .layout;
    let display = if kind == crate::view_layout::DisplayKind::Browser {
        layout.new_browser_display(at, 1)
    } else {
        layout.new_display(at, kind, None, false)
    };
    let id = display.id.clone();
    let area = layout.active_area().id.clone();
    layout.append(&area, display).expect("room");
    runtime.mark_view_unseen(key, &id);
    assert!(
        unseen(runtime, key, &id),
        "the fixture's View waits to be looked at"
    );
    id
}

fn open_output(runtime: &mut Runtime, key: &(String, String), id: &str) {
    runtime.dispatch_json(&explorer_event(
        "desk_view_open",
        serde_json::json!({"kind": "output", "device_id": key.0, "checkout": key.1, "id": id}),
    ));
}

fn unseen(runtime: &Runtime, key: &(String, String), id: &str) -> bool {
    runtime
        .view_display(key, id)
        .is_some_and(|display| display.unseen.is_some())
}

fn error_kind(runtime: &Runtime) -> Option<String> {
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.clone())
}

#[test]
fn a_device_sessions_page_opens_on_desks_desk_and_loads_through_that_device() {
    use crate::view_layout::DisplayKind;
    let (mut runtime, _, _) = desk_runtime("own-desk-device-page");
    runtime.snapshot.status.engine.state = "connected".into();
    device_status(&mut runtime, "connected");
    let key = (DEVICE.to_owned(), DEVICE_ROOT.to_owned());
    let web = "http://localhost:5173/";
    let local = "file:///srv/remote/site/index.html";
    for url in [web, local] {
        let id = session_view(&mut runtime, &key, DisplayKind::Browser, url);
        open_output(&mut runtime, &key, &id);
        assert!(!unseen(&runtime, &key, &id), "{url} opened, so it is seen");
    }

    let node = runtime.node.as_str().to_owned();
    let pages: Vec<_> = runtime
        .desk_views()
        .expect("Desk's desk")
        .displays()
        .filter(|display| display.kind == DisplayKind::Browser)
        .map(|display| {
            (
                display.id.clone(),
                display.load,
                display.url.clone(),
                display.origin.clone(),
            )
        })
        .collect();
    assert_eq!(pages.len(), 2);
    for (id, load, url, origin) in &pages {
        assert_eq!(
            origin.as_ref().map(|origin| origin.key()),
            Some(key.clone())
        );
        let source = runtime
            .browser_route_source(&node, "hide:desk", id, *load)
            .expect("the page resolves");
        assert_eq!(
            (
                source.device_id.as_str(),
                source.checkout_path.as_str(),
                Some(&source.url)
            ),
            (DEVICE, DEVICE_ROOT, url.as_ref()),
            "it loads on the session's device, from its checkout"
        );
    }

    // The same address of this machine's session is another page.
    let directory = runtime
        .catalog_workspaces()
        .find(|workspace| workspace.device_id == node)
        .map(|workspace| workspace.path.clone())
        .expect("this machine's project");
    let here = (node.clone(), directory);
    let id = session_view(&mut runtime, &here, DisplayKind::Browser, web);
    open_output(&mut runtime, &here, &id);
    let pages = runtime
        .desk_views()
        .unwrap()
        .displays()
        .filter(|display| display.kind == DisplayKind::Browser)
        .count();
    assert_eq!(pages, 3, "a page of another machine at the same address");

    device_status(&mut runtime, "disconnected");
    let (id, load, ..) = &pages_of(&runtime)[0];
    assert!(
        runtime
            .browser_route_source(&node, "hide:desk", id, *load)
            .is_none(),
        "a page of a device that went away loads nowhere else"
    );
}

fn pages_of(runtime: &Runtime) -> Vec<(String, u64)> {
    runtime
        .desk_views()
        .expect("Desk's desk")
        .displays()
        .filter(|display| {
            display.kind == crate::view_layout::DisplayKind::Browser
                && display
                    .origin
                    .as_ref()
                    .is_some_and(|origin| origin.device_id == DEVICE)
        })
        .map(|display| (display.id.clone(), display.load))
        .collect()
}

#[test]
fn an_output_that_does_not_open_on_desks_desk_stays_to_look_at() {
    use crate::view_layout::DisplayKind;
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-output-refused");
    runtime.snapshot.status.engine.state = "connected".into();
    let key = (
        runtime.node.as_str().to_owned(),
        directory.to_string_lossy().into_owned(),
    );

    // An outside file is the desktop window's grant, which Desk's desk never holds.
    let outside = session_view(
        &mut runtime,
        &key,
        DisplayKind::External,
        "/Users/x/Downloads/shot.png",
    );
    open_output(&mut runtime, &key, &outside);
    assert!(unseen(&runtime, &key, &outside));

    // Thirty-two Views of unsaved work leave no room (B28).
    for n in 0..crate::view_layout::MAX_VIEW_DISPLAYS {
        let file = directory.join(format!("f{n:02}.md"));
        std::fs::write(&file, "x\n").expect("fixture");
        open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
        let tab = open_tab(&runtime, &file).expect("open");
        runtime.dispatch_json(&explorer_event(
            "file_draft",
            serde_json::json!({"tab_id": tab, "contents_utf8": "changed\n"}),
        ));
    }
    runtime.snapshot.status.last_error = None;
    let page = session_view(
        &mut runtime,
        &key,
        DisplayKind::Browser,
        "https://example.com/",
    );
    open_output(&mut runtime, &key, &page);
    assert!(
        unseen(&runtime, &key, &page),
        "a refused page stays to look at"
    );
    assert_eq!(
        error_kind(&runtime).as_deref(),
        Some(crate::view_layout::LayoutError::DisplayLimit.kind()),
        "the window says why"
    );
    let plan = directory.join("plan.md");
    std::fs::write(&plan, "plan\n").expect("fixture");
    let file = session_view(
        &mut runtime,
        &key,
        DisplayKind::File,
        &plan.to_string_lossy(),
    );
    open_output(&mut runtime, &key, &file);
    assert!(
        unseen(&runtime, &key, &file),
        "a refused file stays to look at"
    );
}

#[test]
fn a_file_of_a_device_that_is_away_waits_naming_it_and_no_read_begins() {
    let (mut runtime, _, _) = desk_runtime("own-desk-device-wait");
    let views = runtime.desk_views_mut().expect("Desk's desk");
    let area = views.active_area().id.clone();
    let mut file = views.new_display(
        &format!("{DEVICE_ROOT}/plan.md"),
        crate::view_layout::DisplayKind::File,
        None,
        false,
    );
    file.origin = Some(crate::view_layout::ViewOrigin {
        device_id: DEVICE.to_owned(),
        path: DEVICE_ROOT.to_owned(),
    });
    let id = file.id.clone();
    views.append(&area, file).expect("room");

    assert!(
        !runtime.restore_desk_views(),
        "B41: no read while its device is away"
    );
    let shown = desk_views(&mut runtime);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].state, crate::model::ViewDisplayState::Waiting);
    assert!(
        shown[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains(DEVICE)),
        "it names the device: {:?}",
        shown[0].reason
    );
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "retry", "display_id": id}),
    );
    assert_eq!(error_kind(&runtime).as_deref(), Some("view_layout.waiting"));
}

#[test]
fn desks_desk_comes_back_with_its_document_on_a_runtime_started_from_its_saved_store() {
    let (mut runtime, checkout_id, directory) = desk_runtime("own-desk-restart");
    let file = directory.join("notes.md");
    open_on_desk(&mut runtime, "workspace:order", &checkout_id, &file);
    let bytes = crate::desk::encode_for_test(runtime.desk_stored_for_test());

    // B8: a second runtime over the same checkout, Desk's store read from the file.
    let (second, _) = tab_order_runtime(&directory.to_string_lossy());
    let mut second = with_new_views(second, "own-desk-restart-second");
    second.read_desk_store_for_test(
        crate::desk::decode_for_test(&bytes).expect("the saved store reads"),
    );
    second.start_test_desk();
    let views = desk_views(&mut second);

    assert_eq!(views.len(), 1);
    assert_eq!(views[0].path, file.to_string_lossy());
    assert_eq!(views[0].state, crate::model::ViewDisplayState::Open);
    assert!(
        open_tab(&second, &file).is_some(),
        "its document was read back"
    );
}

/// B42: a file of a project with no tab yet that cannot be read opens on
/// Desk's desk as a tab saying why, in front, and Retry reads it once it can.
#[cfg(unix)]
#[test]
fn a_file_desk_cannot_read_opens_as_a_tab_saying_why_and_retry_reads_it_once_it_can() {
    use std::os::unix::fs::PermissionsExt;
    let (mut runtime, other, other_checkout) = desk_runtime_with_other("own-desk-unreadable");
    let file = other.join("locked.md");
    std::fs::write(&file, "unlocked\n").expect("fixture");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).expect("lock");
    // A mode bit locks a file only for an account that is not root.
    if std::fs::read(&file).is_ok() {
        return;
    }

    open_on_desk(&mut runtime, "workspace:other", &other_checkout, &file);

    let views = desk_views(&mut runtime);
    assert_eq!(views.len(), 1, "the open is a View of Desk's desk");
    assert_eq!(views[0].path, file.to_string_lossy());
    let reason = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.path == file.to_string_lossy())
        .and_then(|tab| tab.unavailable_reason.clone());
    assert!(reason.is_some(), "its tab says why it could not be read");

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("unlock");
    desk_layout(
        &mut runtime,
        serde_json::json!({"action": "retry", "display_id": views[0].id}),
    );

    let tab = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .find(|tab| tab.path == file.to_string_lossy())
        .expect("the tab stays");
    assert_eq!(tab.unavailable_reason, None, "Retry read the file");
    assert_eq!(desk_views(&mut runtime).len(), 1);
}
